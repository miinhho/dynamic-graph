//! Neural population simulation — 1000 neurons.
//!
//! Validates the substrate at non-trivial scale:
//!
//! - **Parallel dispatch**: 1000 loci with rayon enabled.
//! - **Entity lifecycle**: populations form, go dormant when unstimulated,
//!   revive when re-stimulated.  Split/merge occurs when inhibition
//!   divides a previously unified cluster.
//! - **Structural proposals**: excitatory neurons prune their weakest
//!   incoming connection when overloaded, keeping connectivity bounded.
//! - **Hebbian plasticity**: frequently co-activated pathways strengthen;
//!   unused connections decay toward pruning threshold.
//! - **LocusContext queries**: programs read neighbor states, discover
//!   outgoing topology, and query recent change history.
//! - **Weathering + trim**: after 100 ticks, entity layers are weathered
//!   and the change log is trimmed to validate long-run memory management.
//!
//! ## Topology
//!
//! ```text
//! Pop A (250) ──→ Pop B (250) ──→ Pop C (250) ──→ Pop D (250)
//!   ↕ intra          ↕ intra          ↕ intra          ↕ intra
//! ```
//!
//! - 90% excitatory, 10% inhibitory per population.
//! - Sparse connectivity: ~5 intra-pop + ~3 inter-pop per excitatory neuron.
//! - Inhibitory neurons connect to ~10 local excitatory peers.
//!
//! Run: `cargo run -p graph-engine --release --example neural_population`

use std::sync::Arc;
use std::time::Instant;

use graph_core::{
    BatchId, Change, Endpoints, EntityStatus, InfluenceKindId, LifecycleCause, Locus,
    LocusContext, LocusId, LocusKindId, LocusProgram, ProposedChange, StateVector,
    StructuralProposal, WorldEvent,
};
use graph_engine::{
    DefaultCoherePerspective, DefaultEmergencePerspective, EngineConfig, InfluenceKindConfig,
    InfluenceKindRegistry, LocusKindConfig, LocusKindRegistry, PlasticityConfig, Simulation,
    SimulationConfig,
};
use graph_world::World;
use rustc_hash::FxHashMap;

// ── Constants ─────────────────────────────────────────────────────────────────

const POP_SIZE: u64 = 250;
const NUM_POPS: u64 = 4;
const TOTAL: u64 = POP_SIZE * NUM_POPS;
const INHIBITORY_FRAC: f64 = 0.10;

const KIND_EXC: LocusKindId = LocusKindId(1);
const KIND_INH: LocusKindId = LocusKindId(2);

const INF_EXC: InfluenceKindId = InfluenceKindId(1);
const INF_INH: InfluenceKindId = InfluenceKindId(2);

const FIRE_THRESHOLD: f32 = 0.15;
const NOISE_FLOOR: f32 = 0.005;
/// Connections with weight below this are pruned.
const PRUNE_WEIGHT: f32 = 0.001;
/// Maximum incoming connections before pruning kicks in.
const MAX_IN_DEGREE: usize = 12;

// ── Deterministic RNG (inlined to avoid testkit dev-dep in example) ──────────

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self { Self(seed) }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo)
    }
    fn f32(&mut self) -> f32 {
        (self.next() >> 11) as f32 / (1u64 << 53) as f32
    }

}

// ── Shared topology ───────────────────────────────────────────────────────────

struct NetworkTopology {
    /// excitatory targets per neuron (locus_id → vec of target locus_ids)
    exc_targets: FxHashMap<LocusId, Vec<LocusId>>,
    /// inhibitory targets per neuron
    inh_targets: FxHashMap<LocusId, Vec<LocusId>>,
    /// which neurons are inhibitory
    is_inhibitory: FxHashMap<LocusId, bool>,
}

fn pop_base(pop: u64) -> u64 { pop * POP_SIZE }

fn build_topology(seed: u64) -> NetworkTopology {
    let mut rng = Rng::new(seed);
    let mut exc_targets: FxHashMap<LocusId, Vec<LocusId>> = FxHashMap::default();
    let mut inh_targets: FxHashMap<LocusId, Vec<LocusId>> = FxHashMap::default();
    let mut is_inhibitory: FxHashMap<LocusId, bool> = FxHashMap::default();

    let inh_count = (POP_SIZE as f64 * INHIBITORY_FRAC) as u64;

    for pop in 0..NUM_POPS {
        let base = pop_base(pop);
        // First `inh_count` neurons in each pop are inhibitory.
        for i in 0..POP_SIZE {
            let id = LocusId(base + i);
            let is_inh = i < inh_count;
            is_inhibitory.insert(id, is_inh);

            if is_inh {
                // Inhibitory: connect to ~10 local excitatory peers.
                let mut targets = Vec::new();
                for _ in 0..10 {
                    let t = rng.range(inh_count, POP_SIZE);
                    targets.push(LocusId(base + t));
                }
                targets.sort();
                targets.dedup();
                inh_targets.insert(id, targets);
            } else {
                // Excitatory: ~5 intra-pop targets only.
                // Inter-population connections are sparse (~1 per neuron,
                // only from Pop A→B and Pop C→D) so populations form
                // distinct entity clusters that split/merge/go dormant
                // depending on stimulation patterns.
                let mut targets = Vec::new();
                for _ in 0..5 {
                    let t = rng.range(inh_count, POP_SIZE);
                    let tid = LocusId(base + t);
                    if tid != id { targets.push(tid); }
                }
                // Sparse inter-pop: only ~10% of excitatory neurons have
                // a single cross-population connection.
                if pop + 1 < NUM_POPS && rng.range(0, 10) == 0 {
                    let next_base = pop_base(pop + 1);
                    let t = rng.range(inh_count, POP_SIZE);
                    targets.push(LocusId(next_base + t));
                }
                targets.sort();
                targets.dedup();
                exc_targets.insert(id, targets);
            }
        }
    }
    NetworkTopology { exc_targets, inh_targets, is_inhibitory }
}

// ── Programs ──────────────────────────────────────────────────────────────────

/// Excitatory neuron.  Sums weighted incoming excitatory − inhibitory.
/// If net exceeds threshold, fires to downstream targets.
///
/// Structural: prunes weakest incoming edge when in-degree > MAX_IN_DEGREE
/// and the weakest weight is below PRUNE_WEIGHT.
struct ExcitatoryProgram {
    topo: Arc<NetworkTopology>,
}

impl LocusProgram for ExcitatoryProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        let (exc_sum, inh_sum) = sum_by_kind(incoming);
        let net = exc_sum - inh_sum;

        // Downward causation: if this neuron belongs to an entity with
        // low coherence, raise the firing threshold — conserve energy
        // when the population is fragmenting.
        let threshold = match ctx.entity_of(locus.id) {
            Some(entity) if entity.current.coherence < 0.5 => FIRE_THRESHOLD * 1.5,
            _ => FIRE_THRESHOLD,
        };

        if net < threshold || net < NOISE_FLOOR {
            return vec![];
        }
        let signal = (net * 0.5).min(1.0);
        let targets = match self.topo.exc_targets.get(&locus.id) {
            Some(ts) => ts,
            None => return vec![],
        };
        targets
            .iter()
            .map(|&t| ProposedChange::stimulus(t, INF_EXC, &[signal]))
            .collect()
    }

    fn structural_proposals(
        &self,
        locus: &Locus,
        _incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<StructuralProposal> {
        // Count incoming directed edges (where this locus is the target).
        let mut weakest: Option<(graph_core::RelationshipId, f32)> = None;
        let mut in_degree = 0usize;
        for r in ctx.relationships_for(locus.id) {
            let is_incoming = matches!(
                &r.endpoints,
                Endpoints::Directed { to, .. } if *to == locus.id
            );
            if is_incoming {
                in_degree += 1;
                let w = r.weight();
                if weakest.is_none_or(|(_, ww)| w < ww) {
                    weakest = Some((r.id, w));
                }
            }
        }
        if in_degree > MAX_IN_DEGREE
            && let Some((rid, w)) = weakest
            && w < PRUNE_WEIGHT
        {
            return vec![StructuralProposal::DeleteRelationship { rel_id: rid }];
        }
        vec![]
    }
}

/// Inhibitory neuron.  Sums excitatory input, broadcasts inhibition
/// to local excitatory peers.  No structural proposals.
struct InhibitoryProgram {
    topo: Arc<NetworkTopology>,
}

impl LocusProgram for InhibitoryProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        let (exc_sum, _) = sum_by_kind(incoming);
        if exc_sum < FIRE_THRESHOLD {
            return vec![];
        }
        let signal = (exc_sum * 0.8).min(1.0);
        let targets = match self.topo.inh_targets.get(&locus.id) {
            Some(ts) => ts,
            None => return vec![],
        };
        targets
            .iter()
            .map(|&t| ProposedChange::stimulus(t, INF_INH, &[signal]))
            .collect()
    }
}

fn sum_by_kind(incoming: &[&Change]) -> (f32, f32) {
    let mut exc = 0.0f32;
    let mut inh = 0.0f32;
    for c in incoming {
        let v: f32 = c.after.as_slice().iter().sum();
        match c.kind {
            INF_EXC => exc += v,
            INF_INH => inh += v,
            _ => {}
        }
    }
    (exc, inh)
}

// ── World construction ────────────────────────────────────────────────────────

fn build_world(topo: Arc<NetworkTopology>) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let mut world = World::new();
    for i in 0..TOTAL {
        let id = LocusId(i);
        let kind = if *topo.is_inhibitory.get(&id).unwrap_or(&false) {
            KIND_INH
        } else {
            KIND_EXC
        };
        world.insert_locus(Locus::new(id, kind, StateVector::zeros(1)));
    }

    let mut loci = LocusKindRegistry::new();
    // Refractory period of 3 batches: after a neuron fires, its program
    // is not dispatched for the next 3 batches within the same tick.
    // This linearizes cascade amplification from O(5^N) to O(N × fan_out).
    loci.insert_with_config(KIND_EXC, LocusKindConfig {
        name: None,
        state_slots: Vec::new(),
        program: Box::new(ExcitatoryProgram { topo: Arc::clone(&topo) }),
        refractory_batches: 3,
        encoder: None,
        max_proposals_per_dispatch: None,
    });
    loci.insert_with_config(KIND_INH, LocusKindConfig {
        name: None,
        state_slots: Vec::new(),
        program: Box::new(InhibitoryProgram { topo }),
        refractory_batches: 2,
        encoder: None,
        max_proposals_per_dispatch: None,
    });

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(
        INF_EXC,
        InfluenceKindConfig::new("excitatory")
            .with_decay(0.85)
            .with_plasticity(PlasticityConfig {
                learning_rate: 0.01,
                weight_decay: 0.995,
                max_weight: 2.0,
            })
            .with_prune_threshold(0.001),
    );
    influences.insert(
        INF_INH,
        InfluenceKindConfig::new("inhibitory")
            .with_decay(0.90)
            .with_prune_threshold(0.001),
    );

    (world, loci, influences)
}

// ── Stimulus generation ───────────────────────────────────────────────────────

/// Stimulate a random subset of a population's excitatory neurons.
fn pop_stimulus(rng: &mut Rng, pop: u64, count: usize, intensity: f32) -> Vec<ProposedChange> {
    let base = pop_base(pop);
    let inh_count = (POP_SIZE as f64 * INHIBITORY_FRAC) as u64;
    let exc_range = POP_SIZE - inh_count;
    (0..count)
        .map(|_| {
            let offset = rng.range(inh_count, inh_count + exc_range);
            let id = LocusId(base + offset);
            let v = intensity * (0.5 + 0.5 * rng.f32());
            ProposedChange::stimulus(id, INF_EXC, &[v])
        })
        .collect()
}

// ── Main ──────────────────────────────────────────────────────────────────────

/// Run a phase: N ticks with a stimulus function, recognizing entities
/// every `recognize_every` ticks. Returns `(duration_ms, events)`.
#[allow(clippy::too_many_arguments)]
fn run_phase(
    sim: &mut Simulation,
    rng: &mut Rng,
    ep: &DefaultEmergencePerspective,
    cp: &DefaultCoherePerspective,
    phase_name: &str,
    tick_range: std::ops::Range<u32>,
    stimulus_fn: fn(&mut Rng, u32) -> Vec<ProposedChange>,
    recognize_every: u32,
) -> (u128, Vec<WorldEvent>) {
    println!("--- {phase_name} ---");
    let t = Instant::now();
    let mut all_events = Vec::new();
    for tick in tick_range {
        let stimuli = stimulus_fn(rng, tick);
        let obs = sim.step(stimuli);
        all_events.extend(obs.events);

        // Periodic entity recognition drives lifecycle transitions.
        if tick > 0 && tick % recognize_every == 0 {
            let events = sim.recognize_entities(ep);
            all_events.extend(events);
            sim.extract_cohere(cp);
        }

        if tick % 5 == 4 {
            let active = sim.world.entities().active_count();
            let dormant = sim.world.entities().iter()
                .filter(|e| e.status == EntityStatus::Dormant).count();
            let total = sim.world.entities().iter().count();
            println!(
                "  tick {:>3}: rels={:<5} entities={}/{}/{} (active/dormant/total) regime={:?}",
                tick, obs.relationships, active, dormant, total, obs.regime
            );
        }
    }
    let ms = t.elapsed().as_millis();
    println!("  → {ms}ms\n");
    (ms, all_events)
}

fn main() {
    let t0 = Instant::now();

    // Build topology and world.
    let topo = Arc::new(build_topology(42));
    let (world, loci, influences) = build_world(topo);

    let mut sim = Simulation::with_config(
        world,
        loci,
        influences,
        SimulationConfig {
            engine: EngineConfig {
                max_batches_per_tick: 32,
            },
            ..Default::default()
        },
    );

    // Enable cohere history: retain last 10 snapshots per perspective
    // so we can observe cluster dynamics over time.
    sim.world.coheres_mut().set_max_history(10);

    let build_ms = t0.elapsed().as_millis();
    println!("=== Neural Population Simulation ({TOTAL} neurons) ===\n");
    println!("  4 populations × {POP_SIZE} neurons ({}% inhibitory)", (INHIBITORY_FRAC * 100.0) as u32);
    println!("  build: {build_ms}ms\n");

    let mut rng = Rng::new(7777);

    // Higher threshold so inter-population weak connections don't merge
    // everything into a single entity. This allows population-level
    // entity dynamics (split, dormancy, revive).
    let ep = DefaultEmergencePerspective {
        min_activity_threshold: 0.25,
        overlap_threshold: 0.3,
    };

    let cp = DefaultCoherePerspective {
        min_bridge_activity: 0.15,
        ..Default::default()
    };

    let mut all_events: Vec<WorldEvent> = Vec::new();

    // ── Phase 1: warm-up — stimulate Pop A ──────────────────────────────

    let (phase1_ms, events) = run_phase(
        &mut sim, &mut rng, &ep, &cp,
        "Phase 1: warm-up (stimulate Pop A, 20 ticks)",
        0..20,
        |rng, tick| if tick % 2 == 0 { pop_stimulus(rng, 0, 30, 0.5) } else { vec![] },
        5,
    );
    all_events.extend(events);

    // ── Phase 2: alternating stimulation — entity dynamics ──────────────

    let (phase2_ms, events) = run_phase(
        &mut sim, &mut rng, &ep, &cp,
        "Phase 2: alternating stimulus (Pop A & C, 40 ticks)",
        20..60,
        |rng, tick| if tick % 4 < 2 { pop_stimulus(rng, 0, 20, 0.6) } else { pop_stimulus(rng, 2, 20, 0.6) },
        5,
    );
    all_events.extend(events);

    // ── Phase 3: silence — dormancy ─────────────────────────────────────

    let (phase3_ms, events) = run_phase(
        &mut sim, &mut rng, &ep, &cp,
        "Phase 3: silence (no stimulus, 20 ticks)",
        60..80,
        |_, _| vec![],
        5,
    );
    all_events.extend(events);

    // ── Phase 4: re-stimulate Pop D — revive ────────────────────────────

    let (phase4_ms, events) = run_phase(
        &mut sim, &mut rng, &ep, &cp,
        "Phase 4: re-stimulate Pop D (20 ticks)",
        80..100,
        |rng, tick| if tick % 2 == 0 { pop_stimulus(rng, 3, 30, 0.7) } else { vec![] },
        5,
    );
    all_events.extend(events);

    // Final recognition to capture the last state.
    let final_events = sim.recognize_entities(&ep);
    all_events.extend(final_events);

    // ── Cohere extraction (final snapshot) ─────────────────────────────

    sim.extract_cohere(&cp);
    let coheres = sim.world.coheres().get("default").unwrap_or(&[]);
    println!("--- Cohere clusters (final): {} ---", coheres.len());
    if coheres.is_empty() {
        let active_count = sim.world.entities().active_count();
        println!("  (no clusters — {} active entity/entities; cohere needs ≥2 active)", active_count);
    }
    for c in coheres.iter().take(5) {
        let ms = match &c.members {
            graph_core::CohereMembers::Entities(ids) => ids
                .iter()
                .map(|e| format!("e#{}", e.0))
                .collect::<Vec<_>>()
                .join(", "),
            _ => "(mixed)".to_string(),
        };
        println!("  cohere#{} [{ms}] strength={:.3}", c.id.0, c.strength);
    }

    // ── Cohere history ──────────────────────────────────────────────────

    if let Some(history) = sim.world.coheres().history("default") {
        println!("\n--- Cohere history ({} snapshots) ---", history.len());
        for snap in history.iter() {
            let entity_count: usize = snap.coheres.iter().map(|c| match &c.members {
                graph_core::CohereMembers::Entities(ids) => ids.len(),
                _ => 0,
            }).sum();
            println!(
                "  batch {:>4}: {} clusters, {} entity memberships",
                snap.batch.0, snap.coheres.len(), entity_count
            );
        }
    }
    println!();

    // ── Entity lifecycle report ──────────────────────────────────────────

    println!("--- Entity lifecycle ---");
    let active = sim.world.entities().active_count();
    let dormant = sim.world.entities().iter()
        .filter(|e| e.status == graph_core::EntityStatus::Dormant).count();
    let total_entities = sim.world.entities().iter().count();
    println!("  active: {}  dormant: {}  total: {}", active, dormant, total_entities);

    for e in sim.world.entities().iter().take(12) {
        let members: Vec<u64> = e.current.members.iter().map(|l| l.0).collect();
        let member_summary = if members.len() > 6 {
            format!("[{}, {}, ... {} total]", members[0], members[1], members.len())
        } else {
            format!("{members:?}")
        };
        let transitions: Vec<String> = e
            .layers
            .iter()
            .map(|l| {
                let tag = match &l.transition {
                    graph_core::LayerTransition::Born => "Born".to_string(),
                    graph_core::LayerTransition::MembershipDelta { added, removed } =>
                        format!("Δmembers(+{}/-{})", added.len(), removed.len()),
                    graph_core::LayerTransition::CoherenceShift { from, to } =>
                        format!("coherence({from:.2}→{to:.2})"),
                    graph_core::LayerTransition::Split { offspring } =>
                        format!("Split(→{})", offspring.len()),
                    graph_core::LayerTransition::Merged { absorbed } =>
                        format!("Merged(←{})", absorbed.len()),
                    graph_core::LayerTransition::BecameDormant => "Dormant".to_string(),
                    graph_core::LayerTransition::Revived => "Revived".to_string(),
                };
                format!("{tag}@b{}", l.batch.0)
            })
            .collect();
        println!(
            "  entity#{:<3} {:?} members={} coherence={:.3} layers={}",
            e.id.0, e.status, member_summary, e.current.coherence, e.layer_count()
        );
        if !transitions.is_empty() {
            let shown = if transitions.len() > 8 {
                format!("{}, ... +{} more", transitions[..5].join(", "), transitions.len() - 5)
            } else {
                transitions.join(", ")
            };
            println!("           transitions: {shown}");
        }
    }
    println!();

    // ── Event stream summary ──────────────────────────────────────────────

    let mut born_count = 0usize;
    let mut dormant_count = 0usize;
    let mut revived_count = 0usize;
    let mut split_count = 0usize;
    let mut merge_count = 0usize;
    let mut prune_count = 0usize;
    let mut regime_shifts = 0usize;
    let mut coherence_shifts = 0usize;
    for ev in &all_events {
        match ev {
            WorldEvent::EntityBorn { .. } => born_count += 1,
            WorldEvent::EntityDormant { .. } => dormant_count += 1,
            WorldEvent::EntityRevived { .. } => revived_count += 1,
            WorldEvent::EntitySplit { .. } => split_count += 1,
            WorldEvent::EntityMerged { .. } => merge_count += 1,
            WorldEvent::RelationshipPruned { .. } => prune_count += 1,
            WorldEvent::RelationshipEmerged { .. } => {}
            WorldEvent::RegimeShift { .. } => regime_shifts += 1,
            WorldEvent::CoherenceShift { .. } => coherence_shifts += 1,
            WorldEvent::SchemaViolation { .. } => {}
        }
    }
    println!("--- Event stream ({} total events) ---", all_events.len());
    println!("  born={born_count}  dormant={dormant_count}  revived={revived_count}  split={split_count}  merge={merge_count}");
    println!("  pruned_rels={prune_count}  regime_shifts={regime_shifts}  coherence_shifts={coherence_shifts}");
    println!();

    // ── Causal tracing (sample) ─────────────────────────────────────────

    println!("--- Causal tracing (first 5 dormant entities) ---");
    let mut shown = 0;
    for e in sim.world.entities().iter() {
        if shown >= 5 { break; }
        for layer in &e.layers {
            if matches!(layer.transition, graph_core::LayerTransition::BecameDormant) {
                let cause_str = match &layer.cause {
                    LifecycleCause::RelationshipDecay { decayed_relationships } =>
                        format!("RelationshipDecay({} rels)", decayed_relationships.len()),
                    LifecycleCause::ComponentSplit { weak_bridges } =>
                        format!("ComponentSplit({} bridges)", weak_bridges.len()),
                    LifecycleCause::Unspecified => "Unspecified".to_string(),
                    other => format!("{other:?}"),
                };
                println!("  entity#{} → Dormant@b{}: cause={cause_str}", e.id.0, layer.batch.0);
                shown += 1;
                break;
            }
        }
    }
    println!();

    // ── Time-travel query ───────────────────────────────────────────────

    // Show entity landscape at mid-simulation (batch ~500).
    let mid_batch = BatchId(500);
    let entities_then = sim.world.entities_at_batch(mid_batch);
    let active_then = entities_then.iter()
        .filter(|(_, layer)| !matches!(layer.transition, graph_core::LayerTransition::BecameDormant))
        .count();
    let total_then = entities_then.len();
    println!("--- Time-travel: entity landscape at batch {} ---", mid_batch.0);
    println!("  entities visible: {} ({} non-dormant)", total_then, active_then);
    for (eid, layer) in entities_then.iter().take(5) {
        let members = layer.snapshot.as_ref().map(|s| s.members.len()).unwrap_or(0);
        let coh = layer.snapshot.as_ref().map(|s| s.coherence).unwrap_or(0.0);
        println!("  entity#{} @ b{}: members={} coherence={:.3} transition={:?}",
            eid.0, layer.batch.0, members, coh,
            graph_core::CompressedTransition::from(&layer.transition));
    }
    println!();

    // ── Weathering + trim ────────────────────────────────────────────────

    let pre_layers: usize = sim.world.entities().iter().map(|e| e.layers.len()).sum();
    let pre_changes = sim.world.log().iter().count();

    sim.weather_entities(&graph_core::DefaultEntityWeathering::default());
    let trimmed = sim.trim_change_log(50);

    let post_layers: usize = sim.world.entities().iter().map(|e| e.layers.len()).sum();
    let post_changes = sim.world.log().iter().count();

    println!("--- Weathering + trim ---");
    println!("  entity layers: {} → {}", pre_layers, post_layers);
    println!("  change log: {} → {} (trimmed {})", pre_changes, post_changes, trimmed);
    println!();

    // ── Relationship stats ───────────────────────────────────────────────

    let rels: Vec<_> = sim.world.relationships().iter().collect();
    let active_rels = rels.iter().filter(|r| r.activity() > 0.01).count();
    let max_w = rels.iter().map(|r| r.weight()).fold(0.0f32, f32::max);
    let mean_w = if rels.is_empty() { 0.0 } else {
        rels.iter().map(|r| r.weight()).sum::<f32>() / rels.len() as f32
    };
    println!("--- Relationships ---");
    println!("  total: {}  active(>0.01): {}", rels.len(), active_rels);
    println!("  max weight: {:.4}  mean weight: {:.6}", max_w, mean_w);
    println!();

    // ── Performance summary ──────────────────────────────────────────────

    let total_ms = t0.elapsed().as_millis();
    println!("--- Performance ---");
    println!("  build:   {build_ms:>6}ms");
    println!("  phase 1: {phase1_ms:>6}ms  (warm-up, 20 ticks)");
    println!("  phase 2: {phase2_ms:>6}ms  (alternating, 40 ticks)");
    println!("  phase 3: {phase3_ms:>6}ms  (silence, 20 ticks)");
    println!("  phase 4: {phase4_ms:>6}ms  (re-stimulate, 20 ticks)");
    println!("  total:   {total_ms:>6}ms  (100 ticks)");
    let ms_per_tick = total_ms as f64 / 100.0;
    println!("  ~{ms_per_tick:.1}ms/tick avg");

    println!("\nDone.");
}
