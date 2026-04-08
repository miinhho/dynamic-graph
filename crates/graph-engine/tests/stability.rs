//! Integration tests for the stability layer added in tasks #1–#3.
//!
//! These tests drive a real engine over a real world via testkit fixtures and
//! verify the new stabilizer/classifier/SCC primitives behave as documented.

use graph_core::{
    Channel, EmissionLaw, EntityId, EntityKindId, EntityProgram, EntityState, InteractionKind,
    LawId, SignalVector, StateVector, Stimulus, TickId,
};
use graph_engine::{
    BasicStabilizer, DefaultRegimeClassifier, DynamicsRegime, Engine, EngineConfig, LawRegistry,
    MetricsHistory, ProgramRegistry, RegimeClassifier, RoutingPolicy, SaturationMode, SccPlan,
    TickMetrics, compute_scc_plan,
};
use graph_testkit::{
    assert_bounded_history, assert_states_equivalent, chain_world, cyclic_pair_world,
    pairwise_world, random_pairwise_world, Seed,
};
use graph_world::World;

struct LinearProgram;
struct ExcitatoryLaw;

impl EntityProgram for LinearProgram {
    fn next_state(
        &self,
        current: &EntityState,
        inbox: &SignalVector,
        stimulus: Option<&Stimulus>,
    ) -> EntityState {
        let external_signal = stimulus.map(|s| s.signal.clone()).unwrap_or_default();
        let combined = inbox.add(&external_signal);
        EntityState {
            internal: current
                .internal
                .add(&StateVector::from(combined.scaled(0.5))),
            emitted: combined,
            cooldown: 0,
        }
    }
}

impl EmissionLaw for ExcitatoryLaw {
    fn project(
        &self,
        outbound: &SignalVector,
        _: &EntityState,
        channel: &Channel,
    ) -> SignalVector {
        outbound.scaled(channel.weight * (1.0_f32 - channel.attenuation).clamp(0.0, 1.0))
    }

    fn kind(&self) -> InteractionKind {
        InteractionKind::Excitatory
    }
}

fn registries() -> (ProgramRegistry, LawRegistry) {
    let mut programs = ProgramRegistry::default();
    programs.insert(EntityKindId(1), Box::new(LinearProgram));
    programs.insert(EntityKindId(2), Box::new(LinearProgram));
    let mut laws = LawRegistry::default();
    laws.insert(LawId(1), Box::new(ExcitatoryLaw));
    (programs, laws)
}

fn engine_with(stabilizer: BasicStabilizer) -> Engine<BasicStabilizer> {
    Engine::new(
        EngineConfig {
            activation_threshold: 0.05,
            routing_policy: RoutingPolicy::default(),
        },
        stabilizer,
    )
}

fn entity_states(world: &World) -> Vec<EntityState> {
    let mut entities: Vec<_> = world.entities().collect();
    entities.sort_by_key(|e| e.id.0);
    entities.into_iter().map(|e| e.state.clone()).collect()
}

fn run_metrics(
    engine: &Engine<BasicStabilizer>,
    world: &mut World,
    ticks: u64,
) -> (Vec<f32>, MetricsHistory) {
    let (programs, laws) = registries();
    let mut energy_history = Vec::with_capacity(ticks as usize);
    let mut metrics = MetricsHistory::new(16);
    for t in 1..=ticks {
        let result = engine.tick(TickId(t), world, &programs, &laws, &[]);
        let m = TickMetrics::from_transaction(&result.transaction);
        energy_history.push(m.total_energy);
        metrics.push(m);
    }
    (energy_history, metrics)
}

#[test]
fn stabilizer_keeps_cyclic_pair_bounded_under_long_run() {
    // Without damping, two emitting entities feeding each other in a cycle
    // would explode. With alpha=0.5 + decay=0.9 the energy must stay bounded.
    let mut world = cyclic_pair_world();
    let engine = engine_with(BasicStabilizer {
        alpha: 0.5,
        decay: 0.9,
        saturation: SaturationMode::Tanh,
        trust_region: Some(2.0),
    });
    let (energy, _) = run_metrics(&engine, &mut world, 50);
    assert_bounded_history(&energy, 50.0);
}

#[test]
fn regime_classifier_reports_settling_or_quiescent_on_passive_chain() {
    // A chain world with no continuous stimulus should drift into Settling
    // or Quiescent within a handful of ticks. Note: these are *observation
    // regimes*, not "success" — see docs/identity.md §4.
    let mut world = chain_world(6);
    let engine = engine_with(BasicStabilizer {
        alpha: 0.4,
        decay: 0.6,
        ..BasicStabilizer::default()
    });
    let (_, history) = run_metrics(&engine, &mut world, 12);
    let classifier = DefaultRegimeClassifier::default();
    let regime = classifier.classify(&history);
    assert!(
        matches!(regime, DynamicsRegime::Settling | DynamicsRegime::Quiescent),
        "expected passive chain to be in a transient/quiescent regime, got {regime:?}"
    );
}

#[test]
fn replay_with_same_seed_world_is_deterministic() {
    // Two engines built with identical config running the same fixture must
    // produce identical entity states after the same number of ticks. This is
    // the foundational replay-equivalence test that future WAL/replay work
    // will build on.
    let engine_a = engine_with(BasicStabilizer {
        alpha: 0.6,
        decay: 0.9,
        ..BasicStabilizer::default()
    });
    let engine_b = engine_with(BasicStabilizer {
        alpha: 0.6,
        decay: 0.9,
        ..BasicStabilizer::default()
    });
    let mut world_a = pairwise_world();
    let mut world_b = pairwise_world();

    let _ = run_metrics(&engine_a, &mut world_a, 8);
    let _ = run_metrics(&engine_b, &mut world_b, 8);

    assert_states_equivalent(&entity_states(&world_a), &entity_states(&world_b), 1e-6);
}

#[test]
fn compute_scc_plan_finds_cycle_in_cyclic_pair_world() {
    let world = cyclic_pair_world();
    let plan: SccPlan = compute_scc_plan(world.snapshot());
    assert_eq!(plan.cyclic_block_count(), 1, "{plan:?}");
    assert_eq!(plan.cyclic_components[0], vec![EntityId(1), EntityId(2)]);
    assert!(plan.acyclic_order.is_empty());
}

#[test]
fn compute_scc_plan_orders_chain_world_dependencies_first() {
    let world = chain_world(4);
    let plan = compute_scc_plan(world.snapshot());
    // No cycles in a chain.
    assert_eq!(plan.cyclic_block_count(), 0);
    // Reverse-topological order: tail first, head last.
    assert_eq!(
        plan.acyclic_order,
        vec![EntityId(4), EntityId(3), EntityId(2), EntityId(1)]
    );
}

#[test]
fn random_world_scc_plan_is_reproducible() {
    // Same seed → same world → same plan. Guards against any non-determinism
    // creeping into the SCC primitive.
    let world_a = random_pairwise_world(Seed(11), 12, 0.25);
    let world_b = random_pairwise_world(Seed(11), 12, 0.25);
    let plan_a = compute_scc_plan(world_a.snapshot());
    let plan_b = compute_scc_plan(world_b.snapshot());
    assert_eq!(plan_a, plan_b);
}

#[test]
fn trust_region_clamps_runaway_growth() {
    // Compare two engines: one with no trust region, one tightly clamped.
    // The clamped run must produce strictly smaller peak energy on the same
    // cyclic fixture.
    let mut loose_world = cyclic_pair_world();
    let mut tight_world = cyclic_pair_world();

    let loose = engine_with(BasicStabilizer {
        alpha: 1.0,
        decay: 1.0,
        ..BasicStabilizer::default()
    });
    let tight = engine_with(BasicStabilizer {
        alpha: 1.0,
        decay: 1.0,
        trust_region: Some(0.1),
        ..BasicStabilizer::default()
    });

    let (loose_energy, _) = run_metrics(&loose, &mut loose_world, 20);
    let (tight_energy, _) = run_metrics(&tight, &mut tight_world, 20);

    let loose_peak = loose_energy.iter().cloned().fold(0.0_f32, f32::max);
    let tight_peak = tight_energy.iter().cloned().fold(0.0_f32, f32::max);
    assert!(
        tight_peak < loose_peak,
        "trust region failed to suppress growth: tight {tight_peak} vs loose {loose_peak}"
    );
}
