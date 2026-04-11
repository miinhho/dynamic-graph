//! Signal-propagation example domain.
//!
//! This is the framework's first end-to-end dogfood. It demonstrates
//! the full stack:
//!
//!   Stimulus → batch loop → Change log → Relationship auto-emergence
//!            → Entity recognition → Cohere extraction
//!
//! Also showcases the query surface added in recent redesign iterations:
//! `WorldDiff`, `WorldMetrics`, `path_between`, `reachable_from`, and
//! `connected_components`.
//!
//! ## Domain
//!
//! A small network arranged as a tree:
//!
//!   Emitter (L1)
//!       ├── Relay A (L2)  ──► Sink AA (L3)
//!       └── Sink B (L4)
//!
//! Three locus kinds:
//! - `KIND_EMITTER`: on a stimulus, broadcasts to each downstream.
//! - `KIND_RELAY`: re-emits to one downstream with `gain`, then stops.
//! - `KIND_SINK`: accepts incoming, never emits.
//!
//! Influence kind: "signal" (decay 0.9/batch — slow decay so
//! relationships are still visible when we call recognize_entities).
//!
//! Run: `cargo run -p graph-engine --example signal_propagation`

use graph_core::{
    Change, ChangeSubject, InfluenceKindId, Locus, LocusId, LocusKindId, LocusProgram,
    ProposedChange, StateVector,
};
use graph_engine::{
    DefaultCoherePerspective, DefaultEmergencePerspective, Engine, EngineConfig, InfluenceKindConfig,
    InfluenceKindRegistry, LocusKindRegistry,
};
use graph_query::{connected_components, path_between, reachable_from};
use graph_world::World;

// ── Kind constants ────────────────────────────────────────────────────────────
const KIND_EMITTER: LocusKindId = LocusKindId(1);
const KIND_RELAY: LocusKindId = LocusKindId(2);
const KIND_SINK: LocusKindId = LocusKindId(3);
const SIGNAL: InfluenceKindId = InfluenceKindId(1);

// ── Locus IDs ─────────────────────────────────────────────────────────────────
const L1: LocusId = LocusId(1); // emitter
const L2: LocusId = LocusId(2); // relay_a  (L1 → L2 → L3)
const L3: LocusId = LocusId(3); // sink_aa  (terminal)
const L4: LocusId = LocusId(4); // sink_b   (terminal)

// ── Programs ──────────────────────────────────────────────────────────────────

/// On stimulus only: broadcasts a scaled copy to each downstream.
struct EmitterProgram {
    downstream: Vec<LocusId>,
    gain: f32,
}
impl LocusProgram for EmitterProgram {
    fn process(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        let stimuli_total: f32 = incoming
            .iter()
            .filter(|c| c.is_stimulus())
            .flat_map(|c| c.after.as_slice())
            .sum();
        if stimuli_total < 1e-6 {
            return Vec::new();
        }
        self.downstream
            .iter()
            .map(|&dst| ProposedChange::new(
                ChangeSubject::Locus(dst),
                SIGNAL,
                StateVector::from_slice(&[stimuli_total * self.gain]),
            ))
            .collect()
    }
}

/// Re-emits to one downstream, scaled by `gain`. Stops on second
/// activation (so the chain doesn't cascade indefinitely).
struct RelayProgram {
    downstream: LocusId,
    gain: f32,
}
impl LocusProgram for RelayProgram {
    fn process(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        let total: f32 = incoming.iter().flat_map(|c| c.after.as_slice()).sum();
        if total < 0.01 {
            return Vec::new();
        }
        // Only forward on the first received change (predecessors.len() == 1
        // means it came from a single upstream, i.e. the first pass).
        let is_first_pass = incoming.iter().all(|c| c.predecessors.len() <= 1);
        if !is_first_pass {
            return Vec::new();
        }
        vec![ProposedChange::new(
            ChangeSubject::Locus(self.downstream),
            SIGNAL,
            StateVector::from_slice(&[total * self.gain]),
        )]
    }
}

/// Accepts incoming, never emits.
struct SinkProgram;
impl LocusProgram for SinkProgram {
    fn process(&self, _: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        Vec::new()
    }
}

// ── World construction ────────────────────────────────────────────────────────

fn build_world() -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let mut world = World::new();
    world.insert_locus(Locus::new(L1, KIND_EMITTER, StateVector::zeros(1)));
    world.insert_locus(Locus::new(L2, KIND_RELAY, StateVector::zeros(1)));
    world.insert_locus(Locus::new(L3, KIND_SINK, StateVector::zeros(1)));
    world.insert_locus(Locus::new(L4, KIND_SINK, StateVector::zeros(1)));

    let mut loci = LocusKindRegistry::new();
    loci.insert(
        KIND_EMITTER,
        Box::new(EmitterProgram {
            downstream: vec![L2, L4],
            gain: 0.9,
        }),
    );
    loci.insert(
        KIND_RELAY,
        Box::new(RelayProgram { downstream: L3, gain: 0.8 }),
    );
    loci.insert(KIND_SINK, Box::new(SinkProgram));

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(
        SIGNAL,
        InfluenceKindConfig::new("signal").with_decay(0.9),
    );

    (world, loci, influences)
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let (mut world, loci, influences) = build_world();
    let engine = Engine::new(EngineConfig {
        max_batches_per_tick: 16,
    });

    println!("=== Signal Propagation Example ===\n");
    println!("  L1 (emitter)  → L2 (relay) → L3 (sink)");
    println!("               ↘ L4 (sink)\n");

    let before_tick = world.current_batch();

    let stimulus = ProposedChange::new(
        ChangeSubject::Locus(L1),
        SIGNAL,
        StateVector::from_slice(&[1.0]),
    );

    let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);

    println!(
        "Tick: {} batches, {} changes, hit_cap={}",
        result.batches_committed, result.changes_committed, result.hit_batch_cap
    );
    println!();

    // ── WorldDiff ────────────────────────────────────────────────────────────

    let diff = world.diff_since(before_tick);
    println!("--- WorldDiff (batch {} → {}) ---",
        before_tick.0, world.current_batch().0);
    println!("  changes:               {}", diff.change_ids.len());
    println!("  relationships created: {}", diff.relationships_created.len());
    println!("  relationships updated: {}", diff.relationships_updated.len());
    println!();

    // ── WorldMetrics ─────────────────────────────────────────────────────────

    let m = world.metrics();
    println!("--- WorldMetrics ---");
    println!("  loci:              {}", m.locus_count);
    println!("  relationships:     {} ({} active)", m.relationship_count, m.active_relationship_count);
    println!("  mean activity:     {:.4}", m.mean_activity);
    println!("  max activity:      {:.4}", m.max_activity);
    println!("  components:        {} (largest: {} loci)", m.component_count, m.largest_component_size);
    println!("  max degree:        {}", m.max_degree);
    if let Some((lid, deg)) = m.top_loci_by_degree.first() {
        println!("  top locus by deg:  L{} ({} edges)", lid.0, deg);
    }
    println!();

    // ── Graph traversal ───────────────────────────────────────────────────────

    let path = path_between(&world, L1, L3);
    if let Some(p) = path {
        let hops: Vec<String> = p.iter().map(|l| format!("L{}", l.0)).collect();
        println!("Shortest path L1 → L3: {}", hops.join(" → "));
    } else {
        println!("No path L1 → L3 (relationships not yet emerged)");
    }

    let reachable = reachable_from(&world, L1, 2);
    println!(
        "Reachable from L1 within 2 hops: {:?}",
        reachable.iter().map(|l| l.0).collect::<Vec<_>>()
    );

    let components = connected_components(&world);
    println!("Connected components: {} (largest: {} loci)", components.len(),
        components.iter().map(Vec::len).max().unwrap_or(0));
    println!();

    // ── Change log ────────────────────────────────────────────────────────────

    println!("--- Change log ---");
    for c in world.log().iter() {
        let subj = match c.subject {
            ChangeSubject::Locus(id) => format!("L{}", id.0),
            ChangeSubject::Relationship(id) => format!("R{}", id.0),
        };
        let after_val = c.after.as_slice().first().copied().unwrap_or(0.0);
        let preds: Vec<u64> = c.predecessors.iter().map(|p| p.0).collect();
        println!(
            "  #{} batch={} {} after={:.3} preds={preds:?}",
            c.id.0, c.batch.0, subj, after_val
        );
    }
    println!();

    // ── Relationships ─────────────────────────────────────────────────────────

    println!("--- Relationships (auto-emerged) ---");
    for r in world.relationships().iter() {
        let (f, t) = match &r.endpoints {
            graph_core::Endpoints::Directed { from, to } => (from.0, to.0),
            _ => (0, 0),
        };
        println!(
            "  L{}→L{}  activity={:.3}  touches={}",
            f, t, r.activity(), r.lineage.change_count
        );
    }
    println!();

    // ── Entity recognition ────────────────────────────────────────────────────

    // Lower threshold so freshly-decayed activities are still visible.
    let ep = DefaultEmergencePerspective {
        min_activity_threshold: 0.01,
        ..Default::default()
    };
    engine.recognize_entities(&mut world, &influences, &ep);

    println!(
        "--- Entities ({} active) ---",
        world.entities().active_count()
    );
    for e in world.entities().active() {
        let members: Vec<u64> = e.current.members.iter().map(|l| l.0).collect();
        println!(
            "  entity#{} members={members:?} coherence={:.3} layers={}",
            e.id.0, e.current.coherence, e.layer_count()
        );
    }
    println!();

    // ── Cohere extraction ─────────────────────────────────────────────────────

    let cp = DefaultCoherePerspective {
        min_bridge_activity: 0.01,
        ..Default::default()
    };
    engine.extract_cohere(&mut world, &influences, &cp);

    let coheres = world.coheres().get("default").unwrap_or(&[]);
    println!("--- Coheres ({}) ---", coheres.len());
    for c in coheres {
        let ms = match &c.members {
            graph_core::CohereMembers::Entities(ids) =>
                ids.iter().map(|e| format!("entity#{}", e.0)).collect::<Vec<_>>().join(", "),
            _ => "(mixed)".to_string(),
        };
        println!("  cohere#{} [{ms}] strength={:.3}", c.id.0, c.strength);
    }
    if coheres.is_empty() {
        println!("  (none — entities not bridged above threshold)");
    }

    println!("\nDone.");
}
