//! The substrate batch loop.
//!
//! Implements `docs/redesign.md` §6: commit pending changes, dispatch
//! each affected locus's program, queue follow-ups, repeat until
//! quiescent or the batch cap fires.
//!
//! Design decisions (`docs/redesign.md` §8):
//! - **Predecessors are auto-derived** (O1): internal changes inherit
//!   the ids of changes that fired into their subject locus during the
//!   same batch. `extra_predecessors` on a `ProposedChange` are merged.
//! - **Stimulus = Change with empty predecessors** (O9).
//! - **Single-subject changes only** (O7).
//! - **Locus state = `change.after`** on commit.

mod apply;
pub(crate) mod batch;
mod dispatch;
mod emergence_apply;
mod pipeline;
mod types;
pub(crate) mod world_ops;

use graph_core::{
    BatchId, Change, ChangeId, ChangeSubject, InfluenceKindId, LocusId, ProposedChange,
    RelationshipId, StateVector, WorldEvent,
};
use graph_world::World;

use crate::cohere::CoherePerspective;
use crate::emergence::EmergencePerspective;
use crate::registry::{InfluenceKindRegistry, LocusKindRegistry};

use batch::{
    BuiltChange, ComputedChange, DispatchInput, PendingChange, PlasticityObs, TimingOrder,
    build_computed_change, compute_pending_change,
};
use types::{
    AppliedBatch, BuiltBatch, ComputedBatch, CrossLocusContext, DispatchExecuted, DispatchPrepared,
    EmergenceRecord, SettleContext, SettledBatch, TickState, TickTelemetry,
};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Hard cap on the number of batches a single `tick` call may
    /// process before bailing out and setting `TickResult::hit_batch_cap`.
    ///
    /// **Default**: `64`. Internal constant chosen as an
    /// infinite-cascade guard; not benchmark-tuned. Generously above
    /// what any test in the tree needs for quiescence.
    ///
    /// **Override when**: a legitimate cascade takes longer than 64
    /// batches to settle (raise the cap) or you want a tighter
    /// wall-clock bound per `tick` (lower the cap and treat
    /// `hit_batch_cap = true` as a backpressure signal). For recurring
    /// non-quiescence, also inspect `refractory_batches` on the
    /// culprit locus kind.
    pub max_batches_per_tick: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_batches_per_tick: 64,
        }
    }
}

/// Summary of one `tick` call.
#[derive(Debug, Clone, Default)]
pub struct TickResult {
    pub batches_committed: u32,
    pub changes_committed: u32,
    /// True if the loop stopped because `max_batches_per_tick` fired
    /// rather than because the system went quiescent. A caller can use
    /// this as a signal to escalate (raise the cap, log, etc.).
    pub hit_batch_cap: bool,
    /// Events emitted during this tick: new relationships auto-emerged,
    /// and soft schema violations (endpoint kinds not in `applies_between`).
    pub events: Vec<graph_core::WorldEvent>,
}

/// A stateless policy object that drives the batch loop and on-demand
/// world operations.
///
/// `Engine` holds only `EngineConfig` — no mutable state. All methods
/// take `&self` and operate on a caller-supplied `&mut World`. Use
/// `Simulation` for the full lifecycle (regime classification, adaptive
/// guard rail, WAL persistence).
#[derive(Debug, Default, Clone)]
pub struct Engine {
    config: EngineConfig,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Run the batch loop until quiescent or the per-tick cap fires.
    ///
    /// `stimuli` are the root changes that kick this tick off. Per O9
    /// they are just `ProposedChange`s with no predecessors; the engine
    /// commits them as the first batch's content.
    pub fn tick(
        &self,
        world: &mut World,
        loci_registry: &LocusKindRegistry,
        influence_registry: &InfluenceKindRegistry,
        stimuli: Vec<ProposedChange>,
    ) -> TickResult {
        let slot_defs = influence_registry.slot_defs();
        let mut state = TickState::new(stimuli);
        let mut telemetry = TickTelemetry::new();

        while !state.pending.is_empty() {
            if state.result.batches_committed >= self.config.max_batches_per_tick {
                state.result.hit_batch_cap = true;
                break;
            }
            self.process_batch(
                world,
                loci_registry,
                influence_registry,
                slot_defs,
                &mut state,
                &mut telemetry,
            );
        }

        telemetry.print(state.result.batches_committed);
        state.result
    }

    fn process_batch(
        &self,
        world: &mut World,
        loci_registry: &LocusKindRegistry,
        influence_registry: &InfluenceKindRegistry,
        slot_defs: &crate::registry::SlotDefsMap,
        state: &mut TickState,
        telemetry: &mut TickTelemetry,
    ) {
        pipeline::process_batch(
            self,
            world,
            loci_registry,
            influence_registry,
            slot_defs,
            state,
            telemetry,
        );
    }

    fn build_changes(
        &self,
        world: &mut World,
        computed_batch: ComputedBatch,
        batch: BatchId,
        telemetry: &mut TickTelemetry,
    ) -> BuiltBatch {
        pipeline::build_changes(self, world, computed_batch, batch, telemetry)
    }

    fn apply_built_changes(
        &self,
        world: &mut World,
        influence_registry: &InfluenceKindRegistry,
        batch: BatchId,
        built_batch: BuiltBatch,
        state: &mut TickState,
        telemetry: &mut TickTelemetry,
    ) -> AppliedBatch {
        apply::apply_built_changes(
            self,
            world,
            influence_registry,
            batch,
            built_batch,
            state,
            telemetry,
        )
    }

    fn settle_batch(&self, context: &mut SettleContext<'_>, applied: AppliedBatch) -> SettledBatch {
        apply::settle_batch(self, context, applied)
    }

    fn settle_empty_batch(&self, context: &mut SettleContext<'_>, batch: BatchId) -> SettledBatch {
        apply::settle_empty_batch(self, context, batch)
    }

    fn advance_batch(
        &self,
        world: &mut World,
        settled: SettledBatch,
        state: &mut TickState,
        telemetry: &mut TickTelemetry,
    ) {
        apply::advance_batch(self, world, settled, state, telemetry);
    }

    fn preapply_locus_state(
        &self,
        world: &mut World,
        built: &[BuiltChange],
        telemetry: &mut TickTelemetry,
    ) {
        apply::preapply_locus_state(self, world, built, telemetry);
    }

    fn apply_built_change(
        &self,
        world: &mut World,
        influence_registry: &InfluenceKindRegistry,
        batch: BatchId,
        built: BuiltChange,
        state: &mut TickState,
    ) {
        apply::apply_built_change(self, world, influence_registry, batch, built, state);
    }

    fn apply_locus_change(
        &self,
        world: &mut World,
        influence_registry: &InfluenceKindRegistry,
        batch: BatchId,
        c: batch::BuiltLocusChange,
        state: &mut TickState,
    ) {
        apply::apply_locus_change(self, world, influence_registry, batch, c, state);
    }

    fn apply_locus_property_patch(
        &self,
        world: &mut World,
        locus_id: LocusId,
        property_patch: Option<graph_core::Properties>,
    ) {
        apply::apply_locus_property_patch(self, world, locus_id, property_patch);
    }

    fn apply_cross_locus_emergence(
        &self,
        context: &mut CrossLocusContext<'_>,
        cross_locus_preds: Vec<batch::CrossLocusPred>,
    ) {
        apply::apply_cross_locus_emergence(self, context, cross_locus_preds);
    }

    fn record_schema_violation(
        &self,
        schema_violation: Option<(graph_core::LocusKindId, graph_core::LocusKindId)>,
        kind: InfluenceKindId,
        rel_id: RelationshipId,
        state: &mut TickState,
    ) {
        apply::record_schema_violation(self, schema_violation, kind, rel_id, state);
    }

    fn record_relationship_emergence(
        &self,
        applied: apply::AppliedCrossLocusEmergence,
        state: &mut TickState,
    ) {
        apply::record_relationship_emergence(self, applied, state);
    }

    fn record_plasticity_observation(&self, record: &EmergenceRecord, state: &mut TickState) {
        apply::record_plasticity_observation(self, record, state);
    }

    fn record_batch_kind_touch(
        &self,
        from_locus: LocusId,
        to_locus: LocusId,
        kind: InfluenceKindId,
        rel_id: RelationshipId,
        kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
        state: &mut TickState,
    ) {
        apply::record_batch_kind_touch(self, from_locus, to_locus, kind, rel_id, kind_cfg, state);
    }

    fn apply_relationship_change(
        &self,
        c: batch::BuiltRelChange,
        world: &mut World,
        state: &mut TickState,
    ) {
        apply::apply_relationship_change(self, c, world, state);
    }

    fn append_emergence_changes(&self, world: &mut World, batch: BatchId, state: &mut TickState) {
        apply::append_emergence_changes(self, world, batch, state);
    }

    fn dispatch_affected_loci(
        &self,
        world: &mut World,
        loci_registry: &LocusKindRegistry,
        slot_defs: &crate::registry::SlotDefsMap,
        batch: BatchId,
        state: &mut TickState,
        telemetry: &mut TickTelemetry,
    ) {
        dispatch::dispatch_affected_loci(
            self,
            world,
            loci_registry,
            slot_defs,
            batch,
            state,
            telemetry,
        );
    }

    fn resolve_relationship_notifications(&self, world: &World, state: &mut TickState) {
        dispatch::resolve_relationship_notifications(self, world, state);
    }

    fn collect_dispatch_inputs<'a>(
        &self,
        world: &'a World,
        loci_registry: &'a LocusKindRegistry,
        batch: BatchId,
        state: &TickState,
    ) -> DispatchPrepared<'a> {
        dispatch::collect_dispatch_inputs(self, world, loci_registry, batch, state)
    }

    fn run_dispatches<'a>(
        &self,
        world: &World,
        slot_defs: &crate::registry::SlotDefsMap,
        batch: BatchId,
        prepared: DispatchPrepared<'a>,
    ) -> DispatchExecuted<'a> {
        dispatch::run_dispatches(self, world, slot_defs, batch, prepared)
    }

    fn collect_dispatch_outputs(
        &self,
        loci_registry: &LocusKindRegistry,
        batch: BatchId,
        executed: DispatchExecuted<'_>,
        state: &mut TickState,
    ) {
        dispatch::collect_dispatch_outputs(self, loci_registry, batch, executed, state);
    }

    fn apply_structural_and_hebbian(
        &self,
        world: &mut World,
        influence_registry: &InfluenceKindRegistry,
        batch: BatchId,
        state: &mut TickState,
        telemetry: &mut TickTelemetry,
    ) {
        apply::apply_structural_and_hebbian(
            self,
            world,
            influence_registry,
            batch,
            state,
            telemetry,
        );
    }

    fn record_hebbian_effects(
        &self,
        world: &mut World,
        batch: BatchId,
        hebbian_effects: Vec<world_ops::HebbianEffect>,
    ) {
        apply::record_hebbian_effects(self, world, batch, hebbian_effects);
    }

    // ── on-demand operations — delegate to world_ops ──────────────────────

    /// Flush all pending lazy decay for every relationship.
    ///
    /// Call this before reading relationship activity values (e.g. before
    /// `recognize_entities` or `extract_cohere`).
    pub fn flush_relationship_decay(
        &self,
        world: &mut World,
        influence_registry: &InfluenceKindRegistry,
    ) -> (usize, Vec<graph_core::WorldEvent>) {
        world_ops::flush_relationship_decay(world, influence_registry)
    }

    /// Apply an `EmergencePerspective` to the current world state and
    /// commit its proposals to the entity store. Flushes pending
    /// relationship decay first.
    ///
    /// On-demand — the caller decides when to run. Per
    /// `docs/redesign.md` §6 step 7: "Optional, on-demand."
    pub fn recognize_entities(
        &self,
        world: &mut World,
        influence_registry: &InfluenceKindRegistry,
        perspective: &dyn EmergencePerspective,
    ) -> Vec<graph_core::WorldEvent> {
        world_ops::recognize_entities(world, influence_registry, perspective)
    }

    /// Run a `CoherePerspective` and store the resulting clusters.
    /// Flushes pending relationship decay first.
    ///
    /// On-demand, like `recognize_entities`. Per `docs/redesign.md`
    /// §6 step 8: "Optional, on-demand."
    pub fn extract_cohere(
        &self,
        world: &mut World,
        influence_registry: &InfluenceKindRegistry,
        perspective: &dyn CoherePerspective,
    ) {
        world_ops::extract_cohere(world, influence_registry, perspective);
    }

    /// Trim the change log, dropping all changes in batches strictly older
    /// than `current_batch - retention_batches`. Returns the count removed.
    pub fn trim_change_log(&self, world: &mut World, retention_batches: u64) -> usize {
        world_ops::trim_change_log(world, retention_batches)
    }

    /// Trim the change log to a specific batch cutoff. Changes in batches
    /// strictly before `retain_from` are removed. Returns the count removed.
    pub fn trim_change_log_to(&self, world: &mut World, retain_from: graph_core::BatchId) -> usize {
        world.log_mut().trim_before_batch(retain_from)
    }

    /// Apply a weathering policy to every entity's sediment layer stack.
    ///
    /// Typical cadence: every N ticks (e.g. every 50–100 ticks) rather
    /// than after every tick.
    pub fn weather_entities(
        &self,
        world: &mut World,
        policy: &dyn graph_core::EntityWeatheringPolicy,
    ) {
        world_ops::weather_entities(world, policy);
    }

    /// Pre-wire subscriptions declared by programs via
    /// `LocusProgram::initial_subscriptions`.
    ///
    /// Call this **once** after registering all loci and programs, before
    /// the first `tick`. Programs that need to monitor pre-existing
    /// relationships from the very first batch (e.g. an analyst locus that
    /// must react to the opening state of a conflict edge) should return
    /// those `RelationshipId`s from `initial_subscriptions`. Programs that
    /// subscribe dynamically (e.g. after an activation threshold is crossed)
    /// should continue to use `StructuralProposal::SubscribeToRelationship`
    /// inside `structural_proposals` instead.
    ///
    /// This is idempotent — calling it multiple times is safe, though
    /// wasteful. Subscriptions for non-existent relationships are silently
    /// ignored.
    pub fn bootstrap_subscriptions(&self, world: &mut World, loci_registry: &LocusKindRegistry) {
        // Collect (locus_id, rel_ids) pairs to avoid borrow conflicts.
        let pending: Vec<(LocusId, Vec<RelationshipId>)> = world
            .loci()
            .iter()
            .filter_map(|locus| {
                let program = loci_registry.get(locus.kind)?;
                let subs = program.initial_subscriptions(locus);
                if subs.is_empty() {
                    None
                } else {
                    Some((locus.id, subs))
                }
            })
            .collect();

        for (locus_id, rel_ids) in pending {
            for rel_id in rel_ids {
                world.subscriptions_mut().subscribe(locus_id, rel_id);
            }
        }
    }
}
