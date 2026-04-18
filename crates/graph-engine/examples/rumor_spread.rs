//! Rumor propagation in a social network.
//!
//! Models opinion spreading through a small community. Exercises the
//! graph-query API and surfaces ergonomic gaps for future improvement.
//!
//! Topology:
//! ```text
//!   INFLUENCER_A ──→ REG_1 ──→ REG_3 ──→ SKEPTIC_1
//!   INFLUENCER_A ──→ REG_2
//!   INFLUENCER_B ──→ REG_2 ──→ REG_4 ──→ SKEPTIC_2
//!   INFLUENCER_B ──→ REG_3
//! ```
//!
//! Run: `cargo run -p graph-engine --example rumor_spread`

use graph_core::{
    Change, Endpoints, InfluenceKindId, Locus, LocusContext, LocusId, LocusKindId, LocusProgram,
    ProposedChange, StateSlotDef, StateVector,
};
use graph_engine::{
    Engine, EngineConfig, InfluenceKindConfig, InfluenceKindRegistry, LocusKindConfig,
    LocusKindRegistry, TickResult,
};
use graph_query as Q;
use graph_world::World;

// ─── Kind constants ───────────────────────────────────────────────────────────

const KIND_INFLUENCER: LocusKindId = LocusKindId(1);
const KIND_REGULAR: LocusKindId = LocusKindId(2);
const KIND_SKEPTIC: LocusKindId = LocusKindId(3);

const BELIEF_KIND: InfluenceKindId = InfluenceKindId(1);

// State: [belief (0.0 = no belief, 1.0 = full conviction)]
const BELIEF_SLOT: usize = 0;

// ─── Locus IDs ────────────────────────────────────────────────────────────────

const INFLUENCER_A: LocusId = LocusId(1);
const INFLUENCER_B: LocusId = LocusId(2);
const REG_1: LocusId = LocusId(3);
const REG_2: LocusId = LocusId(4);
const REG_3: LocusId = LocusId(5);
const REG_4: LocusId = LocusId(6);
const SKEPTIC_1: LocusId = LocusId(7);
const SKEPTIC_2: LocusId = LocusId(8);

fn label(id: LocusId) -> &'static str {
    match id {
        INFLUENCER_A => "INFLUENCER_A",
        INFLUENCER_B => "INFLUENCER_B",
        REG_1 => "REG_1",
        REG_2 => "REG_2",
        REG_3 => "REG_3",
        REG_4 => "REG_4",
        SKEPTIC_1 => "SKEPTIC_1",
        SKEPTIC_2 => "SKEPTIC_2",
        _ => "?",
    }
}

// ─── Programs ─────────────────────────────────────────────────────────────────

/// Influencers always emit their full conviction to all downstream.
struct InfluencerProgram;

impl LocusProgram for InfluencerProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        if incoming.is_empty() {
            return vec![];
        }
        // Propagate own belief to all neighbors.
        let my_belief = locus
            .state
            .as_slice()
            .get(BELIEF_SLOT)
            .copied()
            .unwrap_or(0.0);
        ctx.relationships_for(locus.id)
            .filter_map(|r| r.endpoints.target())
            .map(|target| ProposedChange::stimulus(target, BELIEF_KIND, &[my_belief]))
            .collect()
    }
}

/// Regulars gradually adopt opinion based on incoming average, then relay
/// their updated belief downstream so the signal propagates through the graph.
struct RegularProgram {
    learning_rate: f32,
}

impl LocusProgram for RegularProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        if incoming.is_empty() {
            return vec![];
        }
        let current = locus
            .state
            .as_slice()
            .get(BELIEF_SLOT)
            .copied()
            .unwrap_or(0.0);
        let avg_incoming: f32 = incoming
            .iter()
            .map(|c| c.after.as_slice().get(BELIEF_SLOT).copied().unwrap_or(0.0))
            .sum::<f32>()
            / incoming.len() as f32;

        let new_belief = (current + self.learning_rate * (avg_incoming - current)).clamp(0.0, 1.0);
        let mut proposals = Vec::new();

        // Emit own state update only if belief actually moved.
        if (new_belief - current).abs() >= 1e-4 {
            proposals.push(ProposedChange::new(
                graph_core::ChangeSubject::Locus(locus.id),
                BELIEF_KIND,
                StateVector::from_slice(&[new_belief]),
            ));
        }

        // Always relay current belief downstream so the signal propagates
        // through multi-hop paths. Use the effective belief (max of old and new)
        // so downstream gets the strongest available signal.
        let relay_belief = new_belief.max(current);
        if relay_belief > 1e-4 {
            for target in ctx
                .relationships_for(locus.id)
                .filter_map(|r| r.endpoints.target())
            {
                proposals.push(ProposedChange::stimulus(
                    target,
                    BELIEF_KIND,
                    &[relay_belief],
                ));
            }
        }
        proposals
    }
}

/// Skeptics adopt very slowly (1/5th of regulars).
struct SkepticProgram;

impl LocusProgram for SkepticProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        if incoming.is_empty() {
            return vec![];
        }
        let current = locus
            .state
            .as_slice()
            .get(BELIEF_SLOT)
            .copied()
            .unwrap_or(0.0);
        let avg_incoming: f32 = incoming
            .iter()
            .map(|c| c.after.as_slice().get(BELIEF_SLOT).copied().unwrap_or(0.0))
            .sum::<f32>()
            / incoming.len() as f32;

        let new_belief = (current + 0.05 * (avg_incoming - current)).clamp(0.0, 1.0);
        if (new_belief - current).abs() < 1e-4 {
            return vec![];
        }
        vec![ProposedChange::new(
            graph_core::ChangeSubject::Locus(locus.id),
            BELIEF_KIND,
            StateVector::from_slice(&[new_belief]),
        )]
    }
}

// ─── World construction ───────────────────────────────────────────────────────

fn build_world() -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    // Register locus kinds with named state slots so queries can be self-documenting.
    let belief_slot_def = StateSlotDef::new("belief")
        .with_description("Conviction level — 0.0 = no belief, 1.0 = full conviction")
        .with_range(0.0, 1.0);

    let mut loci = LocusKindRegistry::new();
    loci.insert_with_config(
        KIND_INFLUENCER,
        LocusKindConfig {
            name: Some("Influencer".into()),
            state_slots: vec![belief_slot_def.clone()],
            program: Box::new(InfluencerProgram),
            refractory_batches: 0,
            encoder: None,
            max_proposals_per_dispatch: None,
        },
    );
    loci.insert_with_config(
        KIND_REGULAR,
        LocusKindConfig {
            name: Some("Regular".into()),
            state_slots: vec![belief_slot_def.clone()],
            program: Box::new(RegularProgram {
                learning_rate: 0.25,
            }),
            refractory_batches: 0,
            encoder: None,
            max_proposals_per_dispatch: None,
        },
    );
    loci.insert_with_config(
        KIND_SKEPTIC,
        LocusKindConfig {
            name: Some("Skeptic".into()),
            state_slots: vec![belief_slot_def],
            program: Box::new(SkepticProgram),
            refractory_batches: 0,
            encoder: None,
            max_proposals_per_dispatch: None,
        },
    );

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(
        BELIEF_KIND,
        // applies_between: belief can flow from Influencer→Regular,
        // Influencer→Skeptic, or Regular→Regular, Regular→Skeptic.
        // Direct Skeptic→* or Skeptic→Influencer is not a declared pattern.
        InfluenceKindConfig::new("belief")
            .with_decay(0.85)
            .with_learning_rate(0.05)
            .with_applies_between(KIND_INFLUENCER, KIND_REGULAR)
            .with_applies_between(KIND_INFLUENCER, KIND_SKEPTIC)
            .with_applies_between(KIND_REGULAR, KIND_REGULAR)
            .with_applies_between(KIND_REGULAR, KIND_SKEPTIC),
    );

    let mut world = World::new();

    // Influencers start fully convinced; everyone else starts neutral.
    world.insert_locus(Locus::new(
        INFLUENCER_A,
        KIND_INFLUENCER,
        StateVector::from_slice(&[1.0]),
    ));
    world.insert_locus(Locus::new(
        INFLUENCER_B,
        KIND_INFLUENCER,
        StateVector::from_slice(&[1.0]),
    ));
    world.insert_locus(Locus::new(
        REG_1,
        KIND_REGULAR,
        StateVector::from_slice(&[0.0]),
    ));
    world.insert_locus(Locus::new(
        REG_2,
        KIND_REGULAR,
        StateVector::from_slice(&[0.0]),
    ));
    world.insert_locus(Locus::new(
        REG_3,
        KIND_REGULAR,
        StateVector::from_slice(&[0.0]),
    ));
    world.insert_locus(Locus::new(
        REG_4,
        KIND_REGULAR,
        StateVector::from_slice(&[0.0]),
    ));
    world.insert_locus(Locus::new(
        SKEPTIC_1,
        KIND_SKEPTIC,
        StateVector::from_slice(&[0.0]),
    ));
    world.insert_locus(Locus::new(
        SKEPTIC_2,
        KIND_SKEPTIC,
        StateVector::from_slice(&[0.0]),
    ));

    // Pre-wire the social graph.
    for (from, to) in [
        (INFLUENCER_A, REG_1),
        (INFLUENCER_A, REG_2),
        (INFLUENCER_B, REG_2),
        (INFLUENCER_B, REG_3),
        (REG_1, REG_3),
        (REG_2, REG_4),
        (REG_3, SKEPTIC_1),
        (REG_4, SKEPTIC_2),
    ] {
        world.add_relationship(
            Endpoints::directed(from, to),
            BELIEF_KIND,
            influences.initial_state_for(BELIEF_KIND),
        );
    }

    (world, loci, influences)
}

// ─── Query helpers ────────────────────────────────────────────────────────────

fn belief(world: &World, id: LocusId) -> f32 {
    world
        .locus(id)
        .and_then(|l| l.state.as_slice().get(BELIEF_SLOT).copied())
        .unwrap_or(0.0)
}

fn print_beliefs(world: &World) {
    for id in [
        INFLUENCER_A,
        INFLUENCER_B,
        REG_1,
        REG_2,
        REG_3,
        REG_4,
        SKEPTIC_1,
        SKEPTIC_2,
    ] {
        println!("  {:<14} belief={:.3}", label(id), belief(world, id));
    }
}

fn rumor_stimuli() -> Vec<ProposedChange> {
    vec![
        ProposedChange::stimulus(INFLUENCER_A, BELIEF_KIND, &[1.0]),
        ProposedChange::stimulus(INFLUENCER_B, BELIEF_KIND, &[1.0]),
    ]
}

fn print_intro() {
    println!("=== Rumor Propagation Example ===\n");
    println!("  INFLUENCER_A → REG_1 → REG_3 → SKEPTIC_1");
    println!("  INFLUENCER_A → REG_2");
    println!("  INFLUENCER_B → REG_2 → REG_4 → SKEPTIC_2");
    println!("  INFLUENCER_B → REG_3\n");
}

fn run_push_tick(
    engine: &Engine,
    world: &mut World,
    loci: &LocusKindRegistry,
    influences: &InfluenceKindRegistry,
    title: &str,
) -> TickResult {
    println!("--- {title} ---");
    let result = engine.tick(world, loci, influences, rumor_stimuli());
    println!(
        "  batches={} changes={} relationships={}",
        result.batches_committed,
        result.changes_committed,
        world.relationships().len()
    );
    print_beliefs(world);
    println!();
    result
}

fn run_propagation(
    engine: &Engine,
    world: &mut World,
    loci: &LocusKindRegistry,
    influences: &InfluenceKindRegistry,
) -> [TickResult; 3] {
    [
        run_push_tick(
            engine,
            world,
            loci,
            influences,
            "Tick 1: influencers start spreading",
        ),
        run_push_tick(engine, world, loci, influences, "Tick 2: second push"),
        run_push_tick(engine, world, loci, influences, "Tick 3: final push"),
    ]
}

fn print_graph_query_analysis(
    world: &World,
    loci: &LocusKindRegistry,
    influences: &InfluenceKindRegistry,
    tick_results: &[TickResult],
) {
    let current_batch = world.current_batch();
    println!("  current batch: {}", current_batch.0);

    println!("\n=== Graph-Query Analysis ===\n");
    print_structural_analysis(world);
    print_belief_analysis(world);
    print_relationship_analysis(world, current_batch);
    print_temporal_and_causal_analysis(world, current_batch);
    print_schema_and_event_analysis(loci, influences, tick_results);
    print_transitive_and_profile_analysis(world);
    println!("\nDone.");
}

fn print_structural_analysis(world: &World) {
    println!("--- Hub & degree analysis ---");
    let hubs = Q::loci(world).top_n_by_degree(3).collect();
    for l in &hubs {
        let out = Q::locus_out_degree(world, l.id);
        let inn = Q::locus_in_degree(world, l.id);
        let deg = out + inn;
        println!("  {} deg={} (out={} in={})", label(l.id), deg, out, inn);
    }

    let isolated = Q::isolated_loci(world);
    println!("  isolated loci: {}", isolated.len());

    println!("\n--- Neighbors of REG_2 ---");
    let upstr = Q::upstream_of(world, REG_2, 1);
    let downstr = Q::downstream_of(world, REG_2, 1);
    print!("  sources of REG_2: ");
    for id in &upstr {
        print!("{} ", label(*id));
    }
    println!();
    print!("  targets of REG_2: ");
    for id in &downstr {
        print!("{} ", label(*id));
    }
    println!();
}

fn print_belief_analysis(world: &World) {
    println!("\n--- Belief distribution ---");
    let convinced = Q::loci(world)
        .where_state(BELIEF_SLOT, |b| b > 0.5)
        .collect();
    let unconvinced = Q::loci(world)
        .where_state(BELIEF_SLOT, |b| b <= 0.1)
        .collect();
    println!(
        "  convinced (>0.5): {}  {:?}",
        convinced.len(),
        convinced.iter().map(|l| label(l.id)).collect::<Vec<_>>()
    );
    println!(
        "  unconvinced (<=0.1): {}  {:?}",
        unconvinced.len(),
        unconvinced.iter().map(|l| label(l.id)).collect::<Vec<_>>()
    );

    println!("\n--- Top 4 by belief ---");
    let top4 = Q::loci(world).top_n_by_state(BELIEF_SLOT, 4).collect();
    for (rank, l) in top4.iter().enumerate() {
        println!(
            "  #{} {} belief={:.3}",
            rank + 1,
            label(l.id),
            l.state.as_slice()[BELIEF_SLOT]
        );
    }

    println!("\n--- Convinced reach from INFLUENCER_A (depth=3, belief>0.3) ---");
    let convinced_reach = Q::loci(world)
        .reachable_from(INFLUENCER_A, 3)
        .where_state(BELIEF_SLOT, |b| b > 0.3)
        .top_n_by_state(BELIEF_SLOT, 5)
        .collect();
    for l in &convinced_reach {
        println!(
            "  {} belief={:.3}",
            label(l.id),
            l.state.as_slice()[BELIEF_SLOT]
        );
    }

    println!("\n--- Influence balance ---");
    for id in [INFLUENCER_A, INFLUENCER_B, REG_2, REG_3, SKEPTIC_1] {
        let balance = Q::net_influence_balance(world, id);
        let direction = if balance > 0.0 {
            "sender"
        } else if balance < 0.0 {
            "receiver"
        } else {
            "neutral"
        };
        println!("  {:<14} net={:+.3}  ({})", label(id), balance, direction);
    }
}

fn print_relationship_analysis(world: &World, current_batch: graph_core::BatchId) {
    println!("\n--- Strongest relationships ---");
    let top3 = Q::relationships(world)
        .of_kind(BELIEF_KIND)
        .top_n_by_strength(3)
        .collect();
    for r in &top3 {
        let from = r.endpoints.source().map(label).unwrap_or("?");
        let to = r.endpoints.target().map(label).unwrap_or("?");
        println!(
            "  {}→{}  strength={:.3}  activity={:.3}  weight={:.3}  touches={}",
            from,
            to,
            r.strength(),
            r.activity(),
            r.weight(),
            r.lineage.change_count
        );
    }

    println!("\n--- Active outgoing edges from INFLUENCER_A ---");
    let active_out = Q::relationships(world)
        .from(INFLUENCER_A)
        .above_activity(0.0)
        .collect();
    for r in &active_out {
        let to = r.endpoints.target().map(label).unwrap_or("?");
        println!("  INFLUENCER_A→{}  activity={:.3}", to, r.activity());
    }

    println!("\n--- Relationship lifecycle ---");
    let old_rels = Q::relationships(world).older_than(current_batch, 1).count();
    println!("  relationships older than 1 batch: {}", old_rels);
    let volatile_count = Q::relationships(world).above_activity(1.0).count();
    println!("  highly active (activity > 1.0): {}", volatile_count);

    println!("\n--- Reciprocal (bidirectional) pairs ---");
    let pairs = Q::reciprocal_pairs(world);
    println!(
        "  reciprocal pairs: {} (expected 0 for DAG topology)",
        pairs.len()
    );

    println!("\n--- Relationship touch rate (touches/batch) ---");
    let top_touch = Q::relationships(world).top_n_by_change_count(3).collect();
    for r in &top_touch {
        let rate = Q::relationship_touch_rate(world, r.id, current_batch);
        let from = r.endpoints.source().map(label).unwrap_or("?");
        let to = r.endpoints.target().map(label).unwrap_or("?");
        println!("  {}→{}  {:.2} touches/batch", from, to, rate);
    }
}

fn print_temporal_and_causal_analysis(world: &World, current_batch: graph_core::BatchId) {
    println!("\n--- Per-batch activity ---");
    for batch_id in Q::committed_batches(world) {
        let changed = Q::loci_changed_in_batch(world, batch_id);
        if changed.is_empty() {
            continue;
        }
        print!("  batch {}: ", batch_id.0);
        for id in &changed {
            print!("{} ", label(*id));
        }
        println!("({} loci)", changed.len());
    }

    println!("\n--- Causal depth of skeptics' changes ---");
    let s1_latest = Q::last_change_to_locus(world, SKEPTIC_1);
    let s2_latest = Q::last_change_to_locus(world, SKEPTIC_2);
    match (s1_latest, s2_latest) {
        (Some(c1), Some(c2)) => {
            let d1 = Q::causal_depth(world, c1.id);
            let d2 = Q::causal_depth(world, c2.id);
            println!("  SKEPTIC_1 latest change: depth={d1} (chain length from root)");
            println!("  SKEPTIC_2 latest change: depth={d2}");
            let shared = Q::common_ancestors(world, c1.id, c2.id);
            println!("  common ancestors of both: {}", shared.len());
        }
        _ => println!("  (skeptics unchanged — need more ticks)"),
    }

    println!("\n--- Entity recognition ---");
    let entities = Q::active_entities(world);
    println!("  active entities: {}", entities.len());
    for e in &entities {
        let members: Vec<_> = e.current.members.iter().map(|id| label(*id)).collect();
        println!(
            "  entity#{} coherence={:.3} members={:?}",
            e.id.0, e.current.coherence, members
        );
    }

    let skeptic_entities = Q::entities_with_member(world, SKEPTIC_1);
    if skeptic_entities.is_empty() {
        println!("  SKEPTIC_1 is not yet in any entity (not enough coherence)");
    } else {
        println!("  SKEPTIC_1 belongs to entity#{}", skeptic_entities[0].id.0);
    }

    println!("\n--- Upstream neighbors of SKEPTIC_1 (via builder) ---");
    let upstream_ids = Q::upstream_of(world, SKEPTIC_1, 1);
    let upstream_loci = Q::loci_from_ids(world, &upstream_ids)
        .sort_by_state(BELIEF_SLOT)
        .collect();
    for l in &upstream_loci {
        println!(
            "  {} belief={:.3}",
            label(l.id),
            l.state.as_slice().get(BELIEF_SLOT).copied().unwrap_or(0.0)
        );
    }

    let _ = current_batch;
}

fn print_schema_and_event_analysis(
    loci: &LocusKindRegistry,
    influences: &InfluenceKindRegistry,
    tick_results: &[TickResult],
) {
    println!("\n--- Ontology events from ticks ---");
    let all_events: Vec<_> = tick_results.iter().flat_map(|r| r.events.iter()).collect();
    let emerged: Vec<_> = all_events
        .iter()
        .filter(|e| matches!(e, graph_core::WorldEvent::RelationshipEmerged { .. }))
        .collect();
    let violations: Vec<_> = all_events
        .iter()
        .filter(|e| matches!(e, graph_core::WorldEvent::SchemaViolation { .. }))
        .collect();
    println!(
        "  relationships auto-emerged across ticks: {}",
        emerged.len()
    );
    println!("  schema violations (soft): {}", violations.len());
    if violations.is_empty() {
        println!("  → all auto-emerged edges respect applies_between constraints");
    } else {
        println!("  → some edges emerged outside declared applies_between pairs");
    }

    println!("\n--- Schema introspection (locus kind metadata) ---");
    for (kind_id, name) in loci.named_kinds() {
        let cfg = loci.get_config(kind_id).unwrap();
        let slots: Vec<&str> = cfg.state_slots.iter().map(|s| s.name.as_str()).collect();
        let ranges: Vec<String> = cfg
            .state_slots
            .iter()
            .map(|s| {
                s.range
                    .map(|(lo, hi)| format!("[{lo},{hi}]"))
                    .unwrap_or_else(|| "unbounded".into())
            })
            .collect();
        println!(
            "  {name} ({kind_id:?}): slots={:?}  ranges={:?}",
            slots, ranges
        );
    }
    let belief_cfg = influences.get(BELIEF_KIND).unwrap();
    let constraints: Vec<_> = belief_cfg
        .applies_between
        .iter()
        .map(|(fk, tk)| {
            (
                loci.named_kinds()
                    .find(|(id, _)| *id == *fk)
                    .map(|(_, n)| n)
                    .unwrap_or("?"),
                loci.named_kinds()
                    .find(|(id, _)| *id == *tk)
                    .map(|(_, n)| n)
                    .unwrap_or("?"),
            )
        })
        .collect();
    println!("  'belief' applies_between: {:?}", constraints);
}

fn print_transitive_and_profile_analysis(world: &World) {
    println!("\n--- Transitive influence (INFLUENCER_A → SKEPTIC_1, 3 hops) ---");
    let transitive_product = Q::infer_transitive(
        world,
        INFLUENCER_A,
        SKEPTIC_1,
        BELIEF_KIND,
        Q::TransitiveRule::Product,
    );
    let transitive_min = Q::infer_transitive(
        world,
        INFLUENCER_A,
        SKEPTIC_1,
        BELIEF_KIND,
        Q::TransitiveRule::Min,
    );
    match transitive_product {
        Some(v) => println!("  Product rule (weakens with distance): {v:.4}"),
        None => println!("  Product rule: no directed BELIEF_KIND path found"),
    }
    match transitive_min {
        Some(v) => println!("  Min rule    (bottleneck):             {v:.4}"),
        None => println!("  Min rule: no directed BELIEF_KIND path found"),
    }

    let b_to_s2 = Q::infer_transitive(
        world,
        INFLUENCER_B,
        SKEPTIC_2,
        BELIEF_KIND,
        Q::TransitiveRule::Product,
    );
    match b_to_s2 {
        Some(v) => println!("  INFLUENCER_B → SKEPTIC_2 (Product): {v:.4}"),
        None => println!("  INFLUENCER_B → SKEPTIC_2: no path"),
    }

    println!("\n--- Relationship profile: INFLUENCER_A ↔ REG_1 ---");
    let bundle_a1 = Q::relationship_profile(world, INFLUENCER_A, REG_1);
    println!(
        "  edges: {}  net_activity={:.3}  is_excitatory={}",
        bundle_a1.len(),
        bundle_a1.net_activity(),
        bundle_a1.is_excitatory(),
    );
    if let Some(dom) = bundle_a1.dominant_kind() {
        println!("  dominant_kind={dom:?}");
    }

    let bundle_a2 = Q::relationship_profile(world, INFLUENCER_A, REG_2);
    let sim = bundle_a1.profile_similarity(&bundle_a2);
    println!("\n--- Profile similarity: (A↔REG_1) vs (A↔REG_2) ---");
    println!("  profile_similarity={sim:.3}  (1.0 = identical coupling profile)");
    println!(
        "  (with single-kind topology all profiles are collinear — add multi-kind for richer signal)"
    );
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let (mut world, loci, influences) = build_world();
    let engine = Engine::new(EngineConfig {
        max_batches_per_tick: 32,
    });

    print_intro();
    let tick_results = run_propagation(&engine, &mut world, &loci, &influences);
    print_graph_query_analysis(&world, &loci, &influences, &tick_results);
}
