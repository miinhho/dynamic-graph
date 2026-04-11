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
//! 6. If storage is configured, persists the committed batches.
//! 7. Returns a `StepObservation` snapshot.
//!
//! The world is `pub` so callers can call `recognize_entities`,
//! `extract_cohere`, or query the log between steps.

pub(crate) mod builder;
mod config;
mod ingest;
mod lifecycle;

pub use builder::SimulationBuilder;
pub use config::{SimulationConfig, StepObservation};

use rustc_hash::FxHashMap;

use graph_core::{BatchId, InfluenceKindId, ProposedChange, RelationshipId, WorldEvent};
use graph_world::World;

use crate::regime::AdaptiveGuardRail;
use crate::cohere::CoherePerspective;
use crate::emergence::EmergencePerspective;
use crate::engine::{self, Engine};
use crate::regime::{BatchHistory, BatchMetrics, DefaultRegimeClassifier, DynamicsRegime, RegimeClassifier};
use crate::registry::{InfluenceKindRegistry, LocusKindRegistry};

#[cfg(feature = "storage")]
use graph_storage::Storage;

/// Bundles the world, registries, engine, regime classifier, and
/// adaptive guard rail into a single step-by-step interface.
pub struct Simulation {
    /// Direct read/write access to the world state.
    ///
    /// # Invariants — direct mutation can break these
    ///
    /// Most callers should use `Simulation` methods rather than mutating
    /// `world` directly. The following engine invariants can be violated by
    /// bypassing the normal mutation paths:
    ///
    /// - **Change log coherence**: `ChangeLog` is append-only and relies on
    ///   ChangeId density (`id − offset = index`). Inserting non-sequential
    ///   IDs or removing entries without `trim_before_batch` corrupts `get()`.
    /// - **Batch monotonicity**: `world.advance_batch()` must be called only
    ///   by the engine. Calling it externally desynchronizes `prev_batch` and
    ///   breaks per-tick metrics and storage commit ranges.
    /// - **Subscription consistency**: add/remove loci or relationships
    ///   without going through `StructuralProposal` and the subscription
    ///   store's `remove_locus`/`remove_relationship` helpers will leave
    ///   dangling subscriber entries that deliver notifications to gone IDs.
    /// - **Relationship index sync**: `RelationshipStore` maintains a
    ///   `by_locus` index. Inserting or removing relationships directly
    ///   (not via the store's own methods) desynchronizes the index and
    ///   breaks neighbor traversal.
    /// - **Subscription audit log**: calling `subscribe`/`unsubscribe`
    ///   directly (without a batch tag) produces events not visible to
    ///   `WorldDiff`. Use `subscribe_at`/`unsubscribe_at` or a
    ///   `StructuralProposal`.
    pub world: World,
    pub(crate) loci: LocusKindRegistry,
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
    /// Previous regime, for detecting regime shifts.
    prev_regime: DynamicsRegime,
    /// redb-backed persistent storage.
    #[cfg(feature = "storage")]
    pub(crate) storage: Option<Storage>,
    /// Most recent storage write error (cleared on next successful write).
    #[cfg(feature = "storage")]
    pub(crate) last_storage_error: Option<graph_storage::StorageError>,
    /// Auto-trim change log retention window (in batches). `None` = no trim.
    change_retention_batches: Option<u64>,
    /// Activity threshold below which a relationship is considered cold.
    cold_relationship_threshold: Option<f32>,
    /// Minimum idle batches before a relationship can be evicted.
    cold_relationship_min_idle_batches: u64,
    /// Stimuli queued by `ingest()`, drained on the next `step()` or
    /// `flush_ingested()` call.
    pub(crate) pending_stimuli: Vec<ProposedChange>,
    /// String → LocusKindId lookup. Populated by `SimulationBuilder` or
    /// `register_locus_kind_name()`.
    pub(crate) locus_kind_names: FxHashMap<String, graph_core::LocusKindId>,
    /// String → InfluenceKindId lookup.
    pub(crate) influence_kind_names: FxHashMap<String, graph_core::InfluenceKindId>,
    /// Default influence kind used when `ingest()` is called without
    /// specifying one.
    pub(crate) default_influence: Option<graph_core::InfluenceKindId>,
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

        #[cfg(feature = "storage")]
        let (storage, initial_error) = match config.storage_path {
            Some(ref path) => match Storage::open(path) {
                Ok(s) => (Some(s), None),
                Err(e) => (None, Some(e)),
            },
            None => (None, None),
        };

        Self {
            world,
            loci,
            base_influences: influences,
            engine: Engine::new(config.engine),
            guard_rail,
            classifier: DefaultRegimeClassifier::default(),
            history: BatchHistory::new(config.history_window),
            prev_batch,
            prev_regime: DynamicsRegime::Initializing,
            #[cfg(feature = "storage")]
            storage,
            #[cfg(feature = "storage")]
            last_storage_error: initial_error,
            change_retention_batches: config.change_retention_batches,
            cold_relationship_threshold: config.cold_relationship_threshold,
            cold_relationship_min_idle_batches: config.cold_relationship_min_idle_batches,
            pending_stimuli: Vec::new(),
            locus_kind_names: FxHashMap::default(),
            influence_kind_names: FxHashMap::default(),
            default_influence: None,
        }
    }

    /// Run one tick, update regime history, adapt the guard rail, and
    /// return a `StepObservation`.
    ///
    /// If storage persistence is configured, the committed batches are
    /// written to disk before returning. A storage write failure is
    /// non-fatal — the simulation continues and the error is accessible
    /// via `last_storage_error()`.
    pub fn step(&mut self, stimuli: Vec<ProposedChange>) -> StepObservation {
        // Merge any buffered stimuli (queued via `ingest`) with the caller's
        // explicit stimuli. Without this, mixing `ingest()` + `step()` would
        // silently drop the buffered ones.
        let stimuli = if self.pending_stimuli.is_empty() {
            stimuli
        } else {
            let mut all = std::mem::take(&mut self.pending_stimuli);
            all.extend(stimuli);
            all
        };

        let mut effective = self.base_influences.clone();
        let kinds: Vec<InfluenceKindId> = effective.kinds().collect();
        for kind in &kinds {
            if let Some(cfg) = effective.get_mut(*kind) {
                cfg.stabilization = self
                    .guard_rail
                    .effective_stabilization_config(*kind, &cfg.stabilization);
            }
        }

        let prev_batch = self.prev_batch;
        let tick = self
            .engine
            .tick(&mut self.world, &self.loci, &effective, stimuli);

        let current_batch = self.world.current_batch();
        let metrics = self.tick_metrics(prev_batch, current_batch);
        self.prev_batch = current_batch;

        self.history.push(metrics);
        let regime = self.classifier.classify(&self.history);

        for kind in &kinds {
            self.guard_rail.observe(*kind, regime);
        }

        let scales = kinds
            .iter()
            .map(|&k| (k, self.guard_rail.current_scale(k)))
            .collect();

        let mut events = Vec::new();
        if regime != self.prev_regime {
            events.push(WorldEvent::RegimeShift {
                from: self.prev_regime.to_tag(),
                to: regime.to_tag(),
            });
            self.prev_regime = regime;
        }

        // Persist committed batches to redb storage.
        #[cfg(feature = "storage")]
        if let Some(ref storage) = self.storage {
            let mut had_error = false;
            for batch_idx in prev_batch.0..current_batch.0 {
                if let Err(e) = storage.commit_batch(&self.world, BatchId(batch_idx)) {
                    self.last_storage_error = Some(e);
                    had_error = true;
                    break;
                }
            }
            if !had_error && self.last_storage_error.is_some() {
                self.last_storage_error = None;
            }
        }

        // ── Hot/Cold memory management ────────────────────────────────
        // Auto-trim: keep only recent batches in the in-memory ChangeLog.
        // Older changes are already persisted in storage (if configured).
        if let Some(retention) = self.change_retention_batches
            && current_batch.0 > retention
        {
            let cutoff = BatchId(current_batch.0 - retention);
            self.engine.trim_change_log_to(&mut self.world, cutoff);
            self.world.subscriptions_mut().trim_audit_before(cutoff);
        }

        // Cold eviction: move inactive relationships out of memory.
        // They remain in storage for on-demand promotion or analysis.
        if let Some(threshold) = self.cold_relationship_threshold {
            let min_idle = self.cold_relationship_min_idle_batches;
            self.world.evict_cold_relationships(
                threshold,
                min_idle,
                current_batch,
            );
        }

        StepObservation {
            tick,
            regime,
            relationships: self.world.relationships().len(),
            active_entities: self.world.entities().active_count(),
            scales,
            events,
        }
    }

    /// Aggregate `BatchMetrics` over all batches committed between
    /// `from` (exclusive) and `to` (inclusive).
    fn tick_metrics(&self, from: BatchId, to: BatchId) -> BatchMetrics {
        let changes = (from.0 + 1..=to.0)
            .flat_map(|b| self.world.log().batch(BatchId(b)));
        BatchMetrics::from_changes(changes)
    }

    // ── multi-step convenience ────────────────────────────────────────────

    /// Run `n` steps, injecting `stimuli` on the **first** step only.
    /// Returns all `StepObservation`s in order.
    ///
    /// Panics if `n == 0`.
    pub fn step_n(&mut self, n: usize, stimuli: Vec<ProposedChange>) -> Vec<StepObservation> {
        assert!(n > 0, "step_n: n must be at least 1");
        self.step_until(|_, _| false, n, stimuli).0
    }

    /// Run steps until `pred(observation, world)` returns `true` or
    /// `max_steps` is reached.
    ///
    /// `stimuli` are injected on the **first** step only.
    ///
    /// Returns `(observations, converged)` where `converged` is `true` if
    /// the predicate fired before hitting `max_steps`.
    pub fn step_until(
        &mut self,
        mut pred: impl FnMut(&StepObservation, &graph_world::World) -> bool,
        max_steps: usize,
        stimuli: Vec<ProposedChange>,
    ) -> (Vec<StepObservation>, bool) {
        let mut observations = Vec::new();
        let mut stimuli = Some(stimuli);
        for _ in 0..max_steps {
            let s = self.step(stimuli.take().unwrap_or_default());
            let done = pred(&s, &self.world);
            observations.push(s);
            if done {
                return (observations, true);
            }
        }
        (observations, false)
    }

    // ── on-demand world operations ───────────────────────────────────────

    /// Recognize entities using `perspective`. Convenience wrapper so
    /// the caller avoids split-borrow issues with `engine()` + `world`.
    pub fn recognize_entities(&mut self, perspective: &dyn EmergencePerspective) -> Vec<WorldEvent> {
        engine::world_ops::recognize_entities(&mut self.world, &self.base_influences, perspective)
    }

    /// Extract cohere clusters using `perspective`.
    pub fn extract_cohere(&mut self, perspective: &dyn CoherePerspective) {
        engine::world_ops::extract_cohere(&mut self.world, &self.base_influences, perspective);
    }

    /// Apply a weathering policy to every entity's sediment layer stack.
    pub fn weather_entities(&mut self, policy: &dyn graph_core::EntityWeatheringPolicy) {
        self.engine.weather_entities(&mut self.world, policy);
    }

    /// Trim the change log, dropping all changes in batches strictly
    /// older than `current_batch - retention_batches`. Returns count removed.
    pub fn trim_change_log(&mut self, retention_batches: u64) -> usize {
        self.engine.trim_change_log(&mut self.world, retention_batches)
    }

    /// Point-in-time entity query: returns all entities with their
    /// state at or before `batch`. See `World::entities_at_batch`.
    pub fn entities_at_batch(&self, batch: BatchId) -> Vec<(graph_core::EntityId, &graph_core::EntityLayer)> {
        self.world.entities_at_batch(batch)
    }

    // ── cold → hot promotion ─────────────────────────────────────────────

    /// Promote a single evicted relationship back into hot memory.
    ///
    /// Loads the relationship from persistent storage and inserts it into
    /// the in-memory world. Returns `true` if the relationship was found in
    /// storage and was not already in memory, `false` otherwise.
    ///
    /// No-op and returns `false` if storage is not configured.
    #[cfg(feature = "storage")]
    pub fn promote_relationship(&mut self, rel_id: graph_core::RelationshipId) -> bool {
        let Some(ref storage) = self.storage else { return false; };
        match storage.get_relationship(rel_id) {
            Ok(Some(rel)) => self.world.restore_relationship(rel),
            _ => false,
        }
    }

    /// Promote all evicted relationships that involve `locus_id`.
    ///
    /// Scans the stored relationship table and restores any that are not
    /// already in hot memory. Returns the number of relationships promoted.
    ///
    /// This is O(total stored relationships) — use selectively for loci
    /// that are actively participating in the simulation again after a
    /// period of dormancy.
    ///
    /// No-op and returns 0 if storage is not configured.
    #[cfg(feature = "storage")]
    pub fn promote_relationships_for_locus(&mut self, locus_id: graph_core::LocusId) -> usize {
        let Some(ref storage) = self.storage else { return 0; };
        match storage.relationships_for_locus(locus_id) {
            Ok(rels) => rels.into_iter().filter(|r| self.world.restore_relationship(r.clone())).count(),
            Err(e) => {
                self.last_storage_error = Some(e);
                0
            }
        }
    }

    // ── slot queries ─────────────────────────────────────────────────────

    /// Read a named extra slot value from a relationship's current state.
    ///
    /// Returns `None` if the relationship doesn't exist, the kind isn't
    /// registered, or the slot name isn't declared for that kind.
    pub fn rel_slot_value(
        &self,
        rel_id: RelationshipId,
        kind: InfluenceKindId,
        slot_name: &str,
    ) -> Option<f32> {
        let rel = self.world.relationships().get(rel_id)?;
        self.base_influences.get(kind)?.read_slot(&rel.state, slot_name)
    }

    /// History of a named slot for a relationship, newest-first, back to `since`.
    ///
    /// Scans the change log for `ChangeSubject::Relationship(rel_id)` entries
    /// and extracts the slot value from each `after` vector. Useful for
    /// plotting how a slot (e.g. `"tension"`) evolved over time.
    ///
    /// Returns an empty `Vec` if the kind or slot name is unregistered.
    pub fn slot_history(
        &self,
        rel_id: RelationshipId,
        kind: InfluenceKindId,
        slot_name: &str,
        since: BatchId,
    ) -> Vec<(BatchId, f32)> {
        let slot_idx = match self.base_influences.get(kind).and_then(|c| c.slot_index(slot_name)) {
            Some(idx) => idx,
            None => return Vec::new(),
        };
        self.world
            .changes_to_relationship(rel_id)
            .take_while(|c| c.batch.0 >= since.0)
            .filter_map(|c| c.after.as_slice().get(slot_idx).copied().map(|v| (c.batch, v)))
            .collect()
    }

    // ── world convenience accessors ──────────────────────────────────────

    /// Current batch id — the id that will be assigned to the *next* batch
    /// committed by a `step()` call.
    pub fn current_batch(&self) -> graph_core::BatchId {
        self.world.current_batch()
    }

    /// Return the locus with the given id, if it exists.
    ///
    /// Shorthand for `sim.world.locus(id)`. Avoids requiring callers to
    /// know the internal `world` field layout.
    pub fn locus(&self, id: graph_core::LocusId) -> Option<&graph_core::Locus> {
        self.world.locus(id)
    }

    /// Return the relationship with the given id, if it exists.
    pub fn relationship(&self, id: graph_core::RelationshipId) -> Option<&graph_core::Relationship> {
        self.world.relationships().get(id)
    }

    /// All loci of a specific kind.
    pub fn loci_of_kind(&self, kind: graph_core::LocusKindId) -> Vec<&graph_core::Locus> {
        graph_query::loci_of_kind(&self.world, kind)
    }

    /// Find a relationship between two loci, if any exists.
    ///
    /// Checks both directed and symmetric edges in either direction.
    /// O(k_a) where k_a is the degree of locus `a`.
    pub fn relationship_between(
        &self,
        a: graph_core::LocusId,
        b: graph_core::LocusId,
    ) -> Option<&graph_core::Relationship> {
        self.world.relationships().relationships_between(a, b).next()
    }

    // ── kind registry accessors ──────────────────────────────────────────

    /// Resolve a locus kind name (e.g. `"ORG"`) to its `LocusKindId`,
    /// or `None` if the name was never registered.
    ///
    /// Names are populated by `SimulationBuilder::locus_kind()` or
    /// `Simulation::register_locus_kind_name()`.
    pub fn locus_kind_id(&self, name: &str) -> Option<graph_core::LocusKindId> {
        self.locus_kind_names.get(name).copied()
    }

    /// Resolve an influence kind name to its `InfluenceKindId`, or `None`.
    pub fn influence_kind_id(&self, name: &str) -> Option<graph_core::InfluenceKindId> {
        self.influence_kind_names.get(name).copied()
    }

    // ── original accessors ───────────────────────────────────────────────

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
        fn process(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
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
        fn process(&self, _: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
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
        let obs = sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        assert_eq!(obs.regime, DynamicsRegime::Initializing);
    }

    #[test]
    fn relationships_emerge_after_step() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
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

    #[test]
    fn diff_since_captures_changes_and_new_relationships() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let before = sim.world.current_batch();
        sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        let diff = sim.world.diff_since(before);
        assert!(diff.change_count() > 0);
        assert!(!diff.relationships_created.is_empty());
        assert!(diff.relationships_updated.is_empty());
    }

    #[test]
    fn diff_since_second_step_shows_updated_not_created() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        let before = sim.world.current_batch();
        sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        let diff = sim.world.diff_since(before);
        assert!(diff.relationships_created.is_empty());
        assert!(!diff.relationships_updated.is_empty());
    }

    #[test]
    fn step_n_returns_n_observations_and_only_first_gets_stimulus() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let obs = sim.step_n(5, vec![stimulus_to(LocusId(0), 1.0)]);
        assert_eq!(obs.len(), 5);
        assert!(obs[0].tick.changes_committed > 0);
    }

    #[test]
    #[should_panic(expected = "step_n: n must be at least 1")]
    fn step_n_panics_on_zero() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        sim.step_n(0, vec![]);
    }

    #[test]
    fn step_until_stops_when_predicate_fires() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let (obs, converged) = sim.step_until(
            |_, world| !world.relationships().is_empty(),
            20,
            vec![stimulus_to(LocusId(0), 1.0)],
        );
        assert!(converged);
        assert!(obs.last().unwrap().relationships > 0);
    }

    #[test]
    fn step_until_returns_not_converged_when_max_reached() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let (obs, converged) = sim.step_until(|_, _| false, 3, vec![]);
        assert!(!converged);
        assert_eq!(obs.len(), 3);
    }

    #[test]
    fn ingest_creates_locus_and_stores_properties() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let id = sim.ingest("Apple", KIND, SIGNAL, graph_core::props! {
            "type" => "ORG",
            "confidence" => 0.92_f64,
        });
        assert!(sim.world.locus(id).is_some());
        assert_eq!(sim.name_of(id), Some("Apple"));
        assert_eq!(sim.resolve("Apple"), Some(id));
        let props = sim.properties_of(id).unwrap();
        assert_eq!(props.get_str("type"), Some("ORG"));
    }

    #[test]
    fn ingest_same_name_reuses_locus() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let id1 = sim.ingest("Apple", KIND, SIGNAL, graph_core::props! {
            "confidence" => 0.8_f64,
        });
        let id2 = sim.ingest("Apple", KIND, SIGNAL, graph_core::props! {
            "confidence" => 0.95_f64,
            "source" => "Reuters",
        });
        assert_eq!(id1, id2);
        let props = sim.properties_of(id1).unwrap();
        assert_eq!(props.get_f64("confidence"), Some(0.95));
        assert_eq!(props.get_str("source"), Some("Reuters"));
    }

    #[test]
    fn flush_ingested_commits_pending_stimuli() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        sim.ingest("Apple", KIND, SIGNAL, graph_core::props! { "confidence" => 0.9_f64 });
        sim.ingest("Google", KIND, SIGNAL, graph_core::props! { "confidence" => 0.8_f64 });
        let obs = sim.flush_ingested();
        assert!(obs.tick.changes_committed >= 2);
    }

    #[test]
    fn ingest_batch_creates_cooccurrence_relationships() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let ids = sim.ingest_batch(vec![
            ("Apple", KIND, graph_core::props! { "confidence" => 0.9_f64 }),
            ("Tim Cook", KIND, graph_core::props! { "confidence" => 0.95_f64 }),
        ], SIGNAL);
        assert_eq!(ids.len(), 2);
        let obs = sim.flush_ingested();
        assert!(obs.tick.changes_committed >= 2);
        assert!(
            !sim.world.relationships().is_empty(),
            "expected co-occurrence relationship, got 0"
        );
    }

    #[test]
    fn rel_slot_value_and_slot_history_work() {
        use graph_core::{RelationshipSlotDef, RelationshipId};
        use crate::registry::InfluenceKindConfig;

        const SLOTTED: InfluenceKindId = InfluenceKindId(99);
        const SLOT_KIND: LocusKindId = LocusKindId(10);

        // Two loci with a program that emits a relationship-subject change
        // carrying an extra slot value.
        struct SlotProgram { peer: LocusId }
        impl LocusProgram for SlotProgram {
            fn process(&self, locus: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
                let val = locus.state.as_slice().first().copied().unwrap_or(0.0);
                if val < 0.001 { return Vec::new(); }
                // Emit a relationship-subject change with extra slot at index 2.
                vec![ProposedChange::new(
                    ChangeSubject::Locus(self.peer),
                    SLOTTED,
                    StateVector::from_slice(&[val]),
                )]
            }
        }

        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(0), SLOT_KIND, StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(1), SLOT_KIND, StateVector::zeros(1)));

        let mut loci = LocusKindRegistry::new();
        loci.insert(SLOT_KIND, Box::new(SlotProgram { peer: LocusId(1) }));

        let mut influences = InfluenceKindRegistry::new();
        influences.insert(SLOTTED, InfluenceKindConfig::new("slotted")
            .with_extra_slots(vec![RelationshipSlotDef::new("pressure", 0.0)]));

        let mut sim = Simulation::new(world, loci, influences);

        // Stimulate a few steps to build relationship history.
        for i in 1..=3u32 {
            sim.step(vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(0)),
                SLOTTED,
                StateVector::from_slice(&[i as f32 * 0.1]),
            )]);
        }

        // The relationship between locus 0 and 1 should exist.
        assert!(!sim.world.relationships().is_empty());

        // rel_slot_value: unknown slot returns None.
        let rel_id = RelationshipId(0);
        assert!(sim.rel_slot_value(rel_id, SLOTTED, "nonexistent").is_none());

        // slot_history: unregistered kind returns empty.
        let history = sim.slot_history(rel_id, InfluenceKindId(0), "pressure", BatchId(0));
        assert!(history.is_empty());
    }

    #[cfg(feature = "storage")]
    mod storage_tests {
        use super::*;
        use tempfile::NamedTempFile;

        fn storage_config(f: &NamedTempFile) -> SimulationConfig {
            SimulationConfig {
                storage_path: Some(f.path().to_path_buf()),
                ..Default::default()
            }
        }

        #[test]
        fn sim_persists_and_recovers() {
            let f = NamedTempFile::new().unwrap();
            let expected_meta;
            let expected_rels;
            {
                let (world, loci, influences) = two_locus_world();
                let config = storage_config(&f);
                let mut sim = Simulation::with_config(world, loci, influences, config);
                sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
                for _ in 0..4 {
                    sim.step(vec![]);
                }
                assert!(sim.last_storage_error().is_none());
                expected_meta = sim.world.world_meta();
                expected_rels = sim.world.relationships().len();
            }

            let (_, loci2, influences2) = two_locus_world();
            let sim2 = Simulation::from_storage(f.path(), loci2, influences2, SimulationConfig::default()).unwrap();
            assert_eq!(expected_meta, sim2.world.world_meta());
            assert_eq!(expected_rels, sim2.world.relationships().len());
        }

        #[test]
        fn point_queries_work_after_steps() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let mut sim = Simulation::with_config(world, loci, influences, storage_config(&f));
            sim.step(vec![stimulus_to(LocusId(0), 1.0)]);

            let storage = sim.storage().unwrap();
            assert!(storage.get_locus(LocusId(0)).unwrap().is_some());
        }

        #[test]
        fn ingest_persists_properties_and_names() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let mut sim = Simulation::with_config(world, loci, influences, storage_config(&f));
            let id = sim.ingest("Apple", KIND, SIGNAL, graph_core::props! {
                "type" => "ORG",
                "confidence" => 0.92_f64,
            });
            sim.flush_ingested();

            let storage = sim.storage().unwrap();
            let props = storage.get_properties(id).unwrap().unwrap();
            assert_eq!(props.get_str("type"), Some("ORG"));
            assert_eq!(storage.resolve_name("Apple").unwrap(), Some(id));
        }

        #[test]
        fn storage_error_is_none_when_all_writes_succeed() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let mut sim = Simulation::with_config(world, loci, influences, storage_config(&f));
            sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
            sim.step(vec![]);
            assert!(sim.last_storage_error().is_none());
        }

        #[test]
        fn full_save_and_load_round_trip() {
            let f = NamedTempFile::new().unwrap();
            let expected_meta;
            {
                let (world, loci, influences) = two_locus_world();
                let mut sim = Simulation::with_config(world, loci, influences, storage_config(&f));
                sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
                for _ in 0..9 {
                    sim.step(vec![]);
                }
                // Full save instead of incremental.
                sim.save_world().unwrap();
                expected_meta = sim.world.world_meta();
            }

            let (_, loci2, influences2) = two_locus_world();
            let sim2 = Simulation::from_storage(f.path(), loci2, influences2, SimulationConfig::default()).unwrap();
            assert_eq!(expected_meta, sim2.world.world_meta());
        }

        #[test]
        fn change_log_auto_trim_keeps_recent_window() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let config = SimulationConfig {
                storage_path: Some(f.path().to_path_buf()),
                change_retention_batches: Some(2),
                ..Default::default()
            };
            let mut sim = Simulation::with_config(world, loci, influences, config);

            // Keep stimulating every step to ensure changes are generated.
            for _ in 0..10 {
                sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
            }

            // Storage has ALL changes committed across all 10 steps.
            let storage = sim.storage().unwrap();
            let storage_changes = storage.table_counts().unwrap().changes;

            // In-memory log should only retain the recent retention window.
            let log_len = sim.world.log().iter().count();

            assert!(
                storage_changes > log_len as u64,
                "storage ({storage_changes}) should have more changes than trimmed in-memory log ({log_len})"
            );
        }

        #[test]
        fn cold_eviction_reduces_in_memory_relationships() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let config = SimulationConfig {
                storage_path: Some(f.path().to_path_buf()),
                // Aggressive eviction: threshold=100.0 means everything is "cold".
                cold_relationship_threshold: Some(100.0),
                cold_relationship_min_idle_batches: 0,
                ..Default::default()
            };
            let mut sim = Simulation::with_config(world, loci, influences, config);

            sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
            // After step, relationships emerged, but eviction runs at end of step.
            // With threshold=100.0, all relationships have activity < 100.0.
            // With min_idle=0, all are eligible.
            let rels_in_memory = sim.world.relationships().len();

            // Storage has the relationships from commit_batch (before eviction).
            let storage = sim.storage().unwrap();
            let counts = storage.table_counts().unwrap();

            // Relationships were evicted from memory but exist in storage.
            assert_eq!(rels_in_memory, 0, "all relationships should be evicted");
            assert!(counts.relationships > 0, "storage should still have relationships");
        }

        #[test]
        fn promote_relationship_restores_from_storage() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let config = SimulationConfig {
                storage_path: Some(f.path().to_path_buf()),
                // Evict everything immediately.
                cold_relationship_threshold: Some(100.0),
                cold_relationship_min_idle_batches: 0,
                ..Default::default()
            };
            let mut sim = Simulation::with_config(world, loci, influences, config);
            sim.step(vec![stimulus_to(LocusId(0), 1.0)]);

            // All relationships are now evicted from memory.
            assert_eq!(sim.world.relationships().len(), 0);
            let stored_count = sim.storage().unwrap().table_counts().unwrap().relationships;
            assert!(stored_count > 0);

            // Promote back by relationship ID.
            let rel_id = graph_core::RelationshipId(0);
            let was_promoted = sim.promote_relationship(rel_id);
            assert!(was_promoted);
            assert_eq!(sim.world.relationships().len(), 1);

            // Promoting the same relationship again is a no-op.
            assert!(!sim.promote_relationship(rel_id));
            assert_eq!(sim.world.relationships().len(), 1);
        }

        #[test]
        fn promote_relationships_for_locus_restores_all_edges() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let config = SimulationConfig {
                storage_path: Some(f.path().to_path_buf()),
                cold_relationship_threshold: Some(100.0),
                cold_relationship_min_idle_batches: 0,
                ..Default::default()
            };
            let mut sim = Simulation::with_config(world, loci, influences, config);
            sim.step(vec![stimulus_to(LocusId(0), 1.0)]);

            assert_eq!(sim.world.relationships().len(), 0);

            // Promote all relationships involving locus 0.
            let promoted = sim.promote_relationships_for_locus(LocusId(0));
            assert!(promoted > 0);
            assert_eq!(sim.world.relationships().len(), promoted);
        }
    }
}
