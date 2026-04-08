//! Integration tests for the phase-2 follow-ups: AdaptiveStabilizer (#6) and
//! ScheduledDriver (#7).

use graph_core::{
    Channel, EmissionLaw, EntityKindId, EntityProgram, EntityState, InteractionKind, LawId,
    SignalVector, StateVector, Stimulus, TickId,
};
use graph_engine::{
    AdaptiveConfig, AdaptiveStabilizer, BasicStabilizer, DefaultRegimeClassifier, DynamicsRegime,
    Engine, EngineConfig, LawRegistry, MetricsHistory, ProgramRegistry, RegimeClassifier,
    RoutingPolicy, SaturationMode, ScheduleConfig, ScheduledDriver, TickMetrics,
};
use graph_testkit::{chain_world, cyclic_pair_world};
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

fn engine_with<S: graph_engine::Stabilizer>(stabilizer: S) -> Engine<S> {
    Engine::new(
        EngineConfig {
            activation_threshold: 0.05,
            routing_policy: RoutingPolicy::default(),
        },
        stabilizer,
    )
}

fn world_internal_norm(world: &World) -> f32 {
    world
        .entities()
        .map(|e| e.state.internal.l2_norm())
        .sum::<f32>()
}

#[test]
fn adaptive_stabilizer_shrinks_only_on_diverging() {
    // Under the new framing in docs/identity.md, the guard rail tightens
    // only on Diverging. Oscillating is a valid regime and must NOT shrink
    // alpha.
    let base = BasicStabilizer {
        alpha: 0.8,
        decay: 1.0,
        ..BasicStabilizer::default()
    };
    let adaptive = AdaptiveStabilizer::new(
        base,
        AdaptiveConfig {
            min_scale: 0.1,
            max_scale: 1.0,
            shrink_factor: 0.5,
            recovery_factor: 1.05,
        },
    );
    let initial = adaptive.effective_alpha();

    adaptive.observe(DynamicsRegime::Oscillating);
    assert!(
        (adaptive.effective_alpha() - initial).abs() < 1e-6,
        "oscillation must not move the guard rail"
    );

    adaptive.observe(DynamicsRegime::Diverging);
    assert!(
        adaptive.effective_alpha() < initial,
        "divergence must shrink the guard rail: {initial} -> {}",
        adaptive.effective_alpha()
    );
}

#[test]
fn regime_feedback_loop_keeps_cyclic_world_observable() {
    // Drive a cyclic_pair world with an adaptive stabilizer and the default
    // regime classifier. The internal norm at tick 30 must be finite and
    // below a generous bound — that is, the guard rail must keep the system
    // inside the observable range. This is the integration check that the
    // whole feedback loop (engine -> metrics -> classifier -> adaptive
    // observe) holds together. We do *not* assert that the system reached
    // any particular regime, because the engine is not trying to suppress
    // dynamics — only to keep them observable.
    let base = BasicStabilizer {
        alpha: 1.0,
        decay: 1.0,
        saturation: SaturationMode::Tanh,
        trust_region: Some(1.0),
    };
    let adaptive = AdaptiveStabilizer::from_base(base);
    let engine = engine_with(adaptive);
    let (programs, laws) = registries();

    let mut world = cyclic_pair_world();
    let mut history = MetricsHistory::new(8);
    let classifier = DefaultRegimeClassifier::default();

    for t in 1..=30 {
        let result = engine.tick(TickId(t), &mut world, &programs, &laws, &[]);
        let metrics = TickMetrics::from_transaction(&result.transaction);
        history.push(metrics);
        let regime = classifier.classify(&history);
        engine.stabilizer().observe(regime);
    }

    let norm = world_internal_norm(&world);
    assert!(
        norm.is_finite() && norm < 100.0,
        "adaptive feedback failed to tame cyclic world: norm = {norm}"
    );
}

#[test]
fn scheduled_driver_runs_one_sub_tick_for_acyclic_world() {
    // chain_world has zero cyclic blocks, so the iterative loop should
    // short-circuit after the initial pass.
    let engine = engine_with(BasicStabilizer {
        alpha: 0.6,
        decay: 0.9,
        ..BasicStabilizer::default()
    });
    let driver = ScheduledDriver::new(engine, ScheduleConfig::default());
    let (programs, laws) = registries();
    let mut world = chain_world(5);

    let result = driver.tick(TickId(1), &mut world, &programs, &laws, &[]);
    assert_eq!(
        result.iterations(),
        1,
        "acyclic world must not iterate: plan = {:?}",
        result.plan
    );
    assert_eq!(result.plan.cyclic_block_count(), 0);
}

#[test]
fn scheduled_driver_iterates_cyclic_world() {
    // cyclic_pair_world has one cyclic block. With aggressive damping the
    // iterative loop should run more than once and still terminate cleanly.
    let engine = engine_with(BasicStabilizer {
        alpha: 0.5,
        decay: 0.7,
        saturation: SaturationMode::Tanh,
        trust_region: Some(1.0),
    });
    let driver = ScheduledDriver::new(
        engine,
        ScheduleConfig {
            max_inner_ticks: 6,
            convergence_epsilon: 1e-3,
        },
    );
    let (programs, laws) = registries();
    let mut world = cyclic_pair_world();

    let result = driver.tick(TickId(1), &mut world, &programs, &laws, &[]);
    assert!(
        result.iterations() >= 2,
        "expected at least 2 sub-ticks for cyclic world, got {}",
        result.iterations()
    );
    assert_eq!(result.plan.cyclic_block_count(), 1);
    // No matter what, the loop must respect the cap.
    assert!(result.iterations() <= 6);
}

#[test]
fn scheduled_driver_settles_faster_than_single_engine() {
    // Run cyclic_pair_world for the same number of inner ticks via both
    // strategies. The scheduled driver should reach an equal-or-smaller
    // residual delta norm because it short-circuits on convergence and
    // because each sub-tick re-reads the freshly committed state — exactly
    // the iterative-block behaviour from architecture §7.6.
    let total_inner = 6_u64;

    // Baseline: plain engine for `total_inner` logical ticks.
    let baseline_engine = engine_with(BasicStabilizer {
        alpha: 0.5,
        decay: 0.8,
        saturation: SaturationMode::Tanh,
        trust_region: Some(0.5),
    });
    let (programs, laws) = registries();
    let mut baseline_world = cyclic_pair_world();
    for t in 1..=total_inner {
        baseline_engine.tick(TickId(t), &mut baseline_world, &programs, &laws, &[]);
    }
    let baseline_norm = world_internal_norm(&baseline_world);

    // Scheduled: one logical tick with `max_inner_ticks = total_inner`.
    let scheduled_engine = engine_with(BasicStabilizer {
        alpha: 0.5,
        decay: 0.8,
        saturation: SaturationMode::Tanh,
        trust_region: Some(0.5),
    });
    let driver = ScheduledDriver::new(
        scheduled_engine,
        ScheduleConfig {
            max_inner_ticks: total_inner as u32,
            convergence_epsilon: 1e-6,
        },
    );
    let mut scheduled_world = cyclic_pair_world();
    let _ = driver.tick(TickId(1), &mut scheduled_world, &programs, &laws, &[]);
    let scheduled_norm = world_internal_norm(&scheduled_world);

    // Both must produce finite norms; the scheduled run should be at least
    // as well-damped as the baseline (it does the same total work, just
    // bundled into one logical tick).
    assert!(baseline_norm.is_finite() && scheduled_norm.is_finite());
    assert!(
        scheduled_norm <= baseline_norm * 1.001,
        "scheduled run should not be worse than baseline: scheduled = {scheduled_norm}, baseline = {baseline_norm}"
    );
}
