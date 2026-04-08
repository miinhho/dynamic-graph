//! Multi-tick simulation runner.
//!
//! `Simulation` wires together the batch loop, regime classifier, and
//! adaptive guard rail into a single step-by-step interface. Each
//! `step()` call:
//!
//! 1. Applies the guard-rail-scaled alphas to the influence configs.
//! 2. Runs one `Engine::tick`.
//! 3. Computes `BatchMetrics` for the batches just committed.
//! 4. Pushes to the rolling `BatchHistory` and classifies the regime.
//! 5. Feeds the regime back to the `AdaptiveGuardRail`.
//! 6. Returns a `StepObservation` snapshot.
//!
//! The world is `pub` so callers can call `recognize_entities`,
//! `extract_cohere`, or query the log between steps.
//!
//! ## Usage
//!
//! ```ignore
//! let (world, loci, influences) = ring_world(8, 0.9);
//! let mut sim = Simulation::new(world, loci, influences);
//! let obs = sim.step(vec![stimulus(1.0)]);
//! println!("regime: {:?}", obs.regime);
//! for _ in 0..20 {
//!     let obs = sim.step(vec![]);
//!     println!("regime={:?} rels={} entities={}", obs.regime, obs.relationships, obs.active_entities);
//! }
//! ```

use rustc_hash::FxHashMap;

use graph_core::{BatchId, InfluenceKindId, ProposedChange};
use graph_world::World;

use crate::adaptive::{AdaptiveConfig, AdaptiveGuardRail};
use crate::cohere::CoherePerspective;
use crate::emergence::EmergencePerspective;
use crate::engine::{Engine, EngineConfig, TickResult};
use crate::regime::{BatchHistory, BatchMetrics, DefaultRegimeClassifier, DynamicsRegime, RegimeClassifier};
use crate::registry::{InfluenceKindRegistry, LocusKindRegistry};

/// Snapshot of observable state after one `step()`.
#[derive(Debug, Clone)]
pub struct StepObservation {
    /// Result from the underlying `Engine::tick` call.
    pub tick: TickResult,
    /// Dynamical regime classified from the rolling history window.
    pub regime: DynamicsRegime,
    /// Total relationships in the world (all, not just active above a
    /// threshold — relationships are kept until explicitly deleted).
    pub relationships: usize,
    /// Number of entities with `EntityStatus::Active`.
    pub active_entities: usize,
    /// Current guard-rail scale per registered influence kind. A scale
    /// of 1.0 means the guard rail is fully open; < 1.0 means it has
    /// tightened in response to divergence.
    pub scales: FxHashMap<InfluenceKindId, f32>,
}

/// Configuration for `Simulation`.
#[derive(Debug, Clone)]
pub struct SimulationConfig {
    pub engine: EngineConfig,
    pub adaptive: AdaptiveConfig,
    /// Number of ticks to keep in history for regime classification.
    /// Regime is `Initializing` until the window is full.
    pub history_window: usize,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            engine: EngineConfig::default(),
            adaptive: AdaptiveConfig::default(),
            history_window: 8,
        }
    }
}

/// Bundles the world, registries, engine, regime classifier, and
/// adaptive guard rail into a single step-by-step interface.
pub struct Simulation {
    pub world: World,
    loci: LocusKindRegistry,
    /// Original influence configs — never mutated. Each tick we clone
    /// this and apply the guard-rail scale before calling the engine.
    base_influences: InfluenceKindRegistry,
    engine: Engine,
    guard_rail: AdaptiveGuardRail,
    classifier: DefaultRegimeClassifier,
    history: BatchHistory,
    /// Batch at the end of the previous `step()`, used to slice the
    /// change log for this tick's metrics.
    prev_batch: BatchId,
}

impl Simulation {
    pub fn new(world: World, loci: LocusKindRegistry, influences: InfluenceKindRegistry) -> Self {
        Self::with_config(world, loci, influences, SimulationConfig::default())
    }

    pub fn with_config(
        world: World,
        loci: LocusKindRegistry,
        influences: InfluenceKindRegistry,
        config: SimulationConfig,
    ) -> Self {
        let mut guard_rail = AdaptiveGuardRail::new(config.adaptive);
        for kind in influences.kinds() {
            guard_rail.register(kind);
        }
        let prev_batch = world.current_batch();
        Self {
            world,
            loci,
            base_influences: influences,
            engine: Engine::new(config.engine),
            guard_rail,
            classifier: DefaultRegimeClassifier::default(),
            history: BatchHistory::new(config.history_window),
            prev_batch,
        }
    }

    /// Run one tick, update regime history, adapt the guard rail, and
    /// return a `StepObservation`.
    pub fn step(&mut self, stimuli: Vec<ProposedChange>) -> StepObservation {
        // Build an effective influence registry with guard-rail-scaled alphas.
        let mut effective = self.base_influences.clone();
        let kinds: Vec<InfluenceKindId> = effective.kinds().collect();
        for kind in &kinds {
            if let Some(cfg) = effective.get_mut(*kind) {
                cfg.stabilization = self
                    .guard_rail
                    .effective_stabilization_config(*kind, &cfg.stabilization);
            }
        }

        let tick = self
            .engine
            .tick(&mut self.world, &self.loci, &effective, stimuli);

        // Compute aggregate metrics for all batches committed this tick.
        let current_batch = self.world.current_batch();
        let metrics = self.tick_metrics(self.prev_batch, current_batch);
        self.prev_batch = current_batch;

        self.history.push(metrics);
        let regime = self.classifier.classify(&self.history);

        // Feed regime back to the guard rail for every kind.
        for kind in &kinds {
            self.guard_rail.observe(*kind, regime);
        }

        let scales = kinds
            .iter()
            .map(|&k| (k, self.guard_rail.current_scale(k)))
            .collect();

        StepObservation {
            tick,
            regime,
            relationships: self.world.relationships().len(),
            active_entities: self.world.entities().active_count(),
            scales,
        }
    }

    /// Aggregate `BatchMetrics` over all batches committed between
    /// `from` (exclusive) and `to` (inclusive).
    fn tick_metrics(&self, from: BatchId, to: BatchId) -> BatchMetrics {
        let changes = (from.0 + 1..=to.0)
            .flat_map(|b| self.world.log().batch(BatchId(b)));
        BatchMetrics::from_changes(changes)
    }

    /// Recognize entities using `perspective`. Convenience wrapper so
    /// the caller avoids split-borrow issues with `engine()` + `world`.
    pub fn recognize_entities(&mut self, perspective: &dyn EmergencePerspective) {
        self.engine.recognize_entities(&mut self.world, perspective);
    }

    /// Extract cohere clusters using `perspective`.
    pub fn extract_cohere(&mut self, perspective: &dyn CoherePerspective) {
        self.engine.extract_cohere(&mut self.world, perspective);
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn guard_rail(&self) -> &AdaptiveGuardRail {
        &self.guard_rail
    }

    pub fn history(&self) -> &BatchHistory {
        &self.history
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Change, ChangeSubject, InfluenceKindId, Locus, LocusId, LocusKindId, LocusProgram,
        ProposedChange, StateVector,
    };
    use graph_world::World;

    const KIND: LocusKindId = LocusKindId(1);
    const SIGNAL: InfluenceKindId = InfluenceKindId(1);

    struct ForwardProgram {
        downstream: LocusId,
    }
    impl LocusProgram for ForwardProgram {
        fn process(&self, _: &Locus, incoming: &[Change]) -> Vec<ProposedChange> {
            let total: f32 = incoming.iter().flat_map(|c| c.after.as_slice()).sum();
            if total < 0.001 {
                return Vec::new();
            }
            vec![ProposedChange::new(
                ChangeSubject::Locus(self.downstream),
                SIGNAL,
                StateVector::from_slice(&[total * 0.9]),
            )]
        }
    }

    struct InertProgram;
    impl LocusProgram for InertProgram {
        fn process(&self, _: &Locus, _: &[Change]) -> Vec<ProposedChange> {
            Vec::new()
        }
    }

    fn two_locus_world() -> (World, LocusKindRegistry, InfluenceKindRegistry) {
        const SINK_KIND: LocusKindId = LocusKindId(2);
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(0), KIND, StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(1), SINK_KIND, StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(KIND, Box::new(ForwardProgram { downstream: LocusId(1) }));
        loci.insert(SINK_KIND, Box::new(InertProgram));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(SIGNAL, crate::registry::InfluenceKindConfig::new("test").with_decay(0.9));
        (world, loci, influences)
    }

    fn stimulus_to(locus: LocusId, value: f32) -> ProposedChange {
        ProposedChange::new(
            ChangeSubject::Locus(locus),
            SIGNAL,
            StateVector::from_slice(&[value]),
        )
    }

    #[test]
    fn step_returns_observation_and_advances_batch() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let obs = sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        assert!(obs.tick.batches_committed > 0);
        assert!(obs.tick.changes_committed > 0);
    }

    #[test]
    fn regime_initializing_before_window_fills() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        // Window = 8; first step → Initializing
        let obs = sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        assert_eq!(obs.regime, DynamicsRegime::Initializing);
    }

    #[test]
    fn relationships_emerge_after_step() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        // L0→L1 cross-locus flow → 1 relationship
        assert_eq!(sim.world.relationships().len(), 1);
    }

    #[test]
    fn scales_present_for_registered_kinds() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let obs = sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        assert!(!obs.scales.is_empty());
        for &scale in obs.scales.values() {
            assert!(scale > 0.0 && scale <= 1.0);
        }
    }
}
