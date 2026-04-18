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

pub(crate) mod batch;
pub(crate) mod world_ops;

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use graph_core::{
    Change, ChangeId, ChangeSubject, InfluenceKindId, KindObservation,
    LocusId, ProposedChange, Relationship, RelationshipId, RelationshipLineage,
    StateVector, StructuralProposal, WorldEvent,
};
use graph_world::{RelationshipStore, World};

use crate::cohere::CoherePerspective;
use crate::emergence::EmergencePerspective;
use crate::registry::{InfluenceKindRegistry, LocusKindRegistry};

use batch::{
    build_computed_change, compute_pending_change, BuiltChange, ComputedChange, DispatchInput,
    DispatchResult, EmergenceResolution, PartitionAccumulator, PendingChange, PlasticityObs, TimingOrder,
};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Hard cap on the number of batches a single `tick` call may
    /// process before bailing out. Prevents an infinite cascade if a
    /// program is non-quiescent. Default: 64.
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
        let mut result = TickResult::default();
        // Pre-build once per tick; the registry is immutable for the duration.
        let slot_defs = influence_registry.slot_defs();
        let mut pending: Vec<PendingChange> = stimuli
            .into_iter()
            .map(|proposed| PendingChange {
                proposed,
                derived_predecessors: Vec::new(),
            })
            .collect();

        // Per-tick refractory tracking: locus_id → batch number when it last
        // fired (produced non-empty changes). Loci in their refractory window
        // still receive and commit incoming changes, but their program is not
        // dispatched — preventing cascade amplification.
        let mut last_fired: FxHashMap<LocusId, u64> = FxHashMap::default();

        let mut acc = PartitionAccumulator::new();

        // Phase timing: set GRAPH_ENGINE_PROFILE=1 to print per-phase μs to stderr.
        let profile = std::env::var_os("GRAPH_ENGINE_PROFILE").is_some();
        let mut t_compute = std::time::Duration::ZERO;
        let mut t_build   = std::time::Duration::ZERO;
        let mut t_apply   = std::time::Duration::ZERO;
        let mut t_apply_locus    = std::time::Duration::ZERO;
        let mut t_apply_emerge   = std::time::Duration::ZERO;
        let mut t_apply_changelog = std::time::Duration::ZERO;
        let mut t_apply_b3       = std::time::Duration::ZERO;
        let mut t_dispatch = std::time::Duration::ZERO;
        let mut t_hebbian = std::time::Duration::ZERO;
        let mut t_other   = std::time::Duration::ZERO;

        while !pending.is_empty() {
            if result.batches_committed >= self.config.max_batches_per_tick {
                result.hit_batch_cap = true;
                break;
            }

            let batch = world.current_batch();
            acc.clear();

            // ── COMPUTE PHASE (parallel) ────────────────────────────────
            // Read the pre-batch world snapshot for every pending change and
            // produce a `ComputedChange` describing the required mutations.
            // No IDs are minted; no world state is modified here.
            let pending_batch: Vec<PendingChange> = std::mem::take(&mut pending);
            let t0 = if profile { Some(std::time::Instant::now()) } else { None };
            let computed: Vec<ComputedChange> = pending_batch
                .into_par_iter()
                .map(|pc| compute_pending_change(pc, world, influence_registry))
                .collect();
            if let Some(t) = t0 { t_compute += t.elapsed(); }

            // ── BUILD PHASE (parallel) ──────────────────────────────────
            // Assign pre-reserved ChangeIds and construct Change structs in
            // parallel.  No world state is read or written here — all
            // inputs come from the ComputedChange values produced above.
            //
            // Elided changes are filtered out first so that the reserved ID
            // block is dense and contains only real commits.

            // Collect non-Elided changes preserving their enumeration order
            // (order determines which ID each change receives).
            let non_elided: Vec<(usize, ComputedChange)> = {
                let mut idx = 0usize;
                computed
                    .into_iter()
                    .filter_map(|c| {
                        if matches!(c, ComputedChange::Elided) {
                            None
                        } else {
                            let i = idx;
                            idx += 1;
                            Some((i, c))
                        }
                    })
                    .collect()
            };

            let n = non_elided.len();
            let ta = if profile { Some(std::time::Instant::now()) } else { None };
            if n > 0 {
                // Single reservation — no per-change mint_change_id calls.
                let base_id = world.reserve_change_ids(n);
                // Pre-allocate the log's inner Vec to avoid reallocations.
                world.log_mut().reserve(n);

                // Only dispatch to rayon for large batches; for small ones
                // the thread-dispatch overhead exceeds the parallel gain.
                const PAR_BUILD_THRESHOLD: usize = 512;
                let tb = if profile { Some(std::time::Instant::now()) } else { None };
                let built: Vec<BuiltChange> = if n >= PAR_BUILD_THRESHOLD {
                    non_elided
                        .into_par_iter()
                        .map(|(i, c)| build_computed_change(i, base_id, c, batch))
                        .collect()
                } else {
                    non_elided
                        .into_iter()
                        .map(|(i, c)| build_computed_change(i, base_id, c, batch))
                        .collect()
                };
                if let Some(t) = tb { t_build += t.elapsed(); }

                // ── APPLY PHASE A: locus state dedup + pre-alloc ──────
                // Multiple changes may target the same locus in one batch;
                // the last write wins.  Pre-applying the final state here
                // eliminates N − n_unique_loci redundant HashMap writes.
                // Also count cross-locus preds so we can pre-allocate the
                // relationship store before bulk emergence.
                let t_al0 = if profile { Some(std::time::Instant::now()) } else { None };
                let mut final_locus_states: FxHashMap<LocusId, &StateVector> =
                    FxHashMap::default();
                let mut n_potential_new_rels: usize = 0;
                for bc in &built {
                    if let BuiltChange::Locus(c) = bc {
                        final_locus_states.insert(c.locus_id, &c.after);
                        n_potential_new_rels += c.cross_locus_preds.len();
                    }
                }
                for (locus_id, state) in final_locus_states {
                    if let Some(locus) = world.locus_mut(locus_id) {
                        locus.state = state.clone();
                    }
                }
                // Pre-allocate relationship store to avoid rehashing during
                // bulk auto-emergence (worst case: all preds create new rels).
                if n_potential_new_rels > 0 {
                    world.relationships_mut().reserve(n_potential_new_rels);
                }
                if let Some(t) = t_al0 { t_apply_locus += t.elapsed(); }

                // ── APPLY PHASE B: collect then bulk-commit ────────────
                // Pass B1: iterate `built` sequentially, collecting bookkeeping
                // data and queueing Change records.  No ChangeLog mutations yet.
                // Pass B2: single `extend_batch` call replaces N individual
                // `append_change` calls, reducing ChangeLog HashMap ops from
                // O(3N) to O(n_unique_subjects + 1).
                acc.batch_changes.reserve(n);

                let t_ae0 = if profile { Some(std::time::Instant::now()) } else { None };

                for bc in built {
                    match bc {
                        BuiltChange::Locus(c) => {
                            let id = c.change.id;
                            acc.batch_changes.push(c.change);
                            if let Some(patch) = c.property_patch {
                                if let Some(props) = world.properties_mut().get_mut(c.locus_id) {
                                    props.extend(&patch);
                                } else {
                                    world.properties_mut().insert(c.locus_id, patch);
                                }
                            }
                            let kind_cfg = influence_registry.get(c.kind);
                            for pred in c.cross_locus_preds {
                                let Some((rel_id, is_new, emerged_state)) = apply_emergence(
                                    world.relationships_mut(), pred.emergence, id, batch, c.kind,
                                    pred.pre_signal, kind_cfg, &c.resolved_slots,
                                ) else { continue };
                                if let Some((fk, tk)) = pred.schema_violation {
                                    acc.events.push(WorldEvent::SchemaViolation {
                                        relationship: rel_id,
                                        kind: c.kind,
                                        from_locus_kind: fk,
                                        to_locus_kind: tk,
                                    });
                                }
                                if is_new {
                                    acc.events.push(WorldEvent::RelationshipEmerged {
                                        relationship: rel_id,
                                        from: pred.from_locus,
                                        to: c.locus_id,
                                        kind: c.kind,
                                        trigger_change_id: id,
                                    });
                                    acc.new_emerged_rels.push((
                                        rel_id, id, c.kind,
                                        emerged_state.expect("new relationship must have initial state"),
                                    ));
                                }
                                if c.plasticity_active {
                                    let timing = if pred.pred_batch < batch {
                                        if pred.is_feedback { TimingOrder::PostFirst } else { TimingOrder::PreFirst }
                                    } else {
                                        TimingOrder::Simultaneous
                                    };
                                    acc.plasticity_obs.push(PlasticityObs {
                                        rel_id,
                                        kind: c.kind,
                                        pre: pred.pre_signal,
                                        post: c.post_signal,
                                        timing,
                                        post_locus: c.locus_id,
                                    });
                                }
                                {
                                    let ep_key = if kind_cfg.map(|k| k.symmetric).unwrap_or(false) {
                                        graph_core::Endpoints::symmetric(pred.from_locus, c.locus_id).key()
                                    } else {
                                        graph_core::Endpoints::directed(pred.from_locus, c.locus_id).key()
                                    };
                                    let entry = acc.batch_kind_touches.entry(ep_key).or_default();
                                    entry.0.insert(c.kind);
                                    entry.1.insert(rel_id);
                                }
                            }
                            acc.committed_ids_by_locus.entry(c.locus_id).or_default().push(id);
                            if acc.affected_loci_set.insert(c.locus_id) {
                                acc.affected_loci.push(c.locus_id);
                            }
                        }
                        BuiltChange::Relationship(c) => {
                            let id = c.change.id;
                            if let Some(rel) = world.relationships_mut().get_mut(c.rel_id) {
                                rel.state = c.after;
                                rel.lineage.last_touched_by = Some(id);
                                rel.lineage.change_count += 1;
                            }
                            acc.batch_changes.push(c.change);
                            if c.has_subscribers {
                                acc.pending_rel_notifications.push((
                                    c.rel_id, id, c.kind, c.from, c.to,
                                ));
                            }
                        }
                    }
                    result.changes_committed += 1;
                }
                if let Some(t) = t_ae0 { t_apply_emerge += t.elapsed(); }

                // Pass B2: bulk ChangeLog append — one grouping pass instead
                // of N individual HashMap inserts.
                let t_acl0 = if profile { Some(std::time::Instant::now()) } else { None };
                world.extend_batch_changes(std::mem::take(&mut acc.batch_changes));
                if let Some(t) = t_acl0 { t_apply_changelog += t.elapsed(); }

                // Pass B3: write ChangeSubject::Relationship entries for every
                // relationship that auto-emerged in this batch.  These entries
                // make emergence discoverable via standard log traversal
                // (e.g. `relationships_caused_by` Category 1) without requiring
                // callers to know about the lineage.created_by backlink.
                //
                // The new Change IDs are reserved here (after the main batch
                // commit) so they don't interfere with the O(1) `get(id)`
                // density invariant: the relationship entries are appended
                // at the tail of the reserved-ID sequence for this batch.
                let t_ab30 = if profile { Some(std::time::Instant::now()) } else { None };
                if !acc.new_emerged_rels.is_empty() {
                    let n_new = acc.new_emerged_rels.len();
                    let emerge_base = world.reserve_change_ids(n_new);
                    world.log_mut().reserve(n_new);
                    let emerge_changes: Vec<Change> = acc.new_emerged_rels
                        .iter()
                        .enumerate()
                        .map(|(i, (rel_id, trigger_id, kind, initial_state))| {
                            let before = StateVector::zeros(initial_state.dim());
                            Change {
                                id: ChangeId(emerge_base.0 + i as u64),
                                subject: ChangeSubject::Relationship(*rel_id),
                                kind: *kind,
                                predecessors: vec![*trigger_id],
                                before,
                                after: initial_state.clone(),
                                batch,
                                wall_time: None,
                                metadata: None,
                            }
                        })
                        .collect();
                    world.extend_batch_changes(emerge_changes);
                }
                if let Some(t) = t_ab30 { t_apply_b3 += t.elapsed(); }
            }

            if let Some(t) = ta { t_apply += t.elapsed(); }

            // Resolve relationship-change notifications to subscriber loci.
            // Each subscriber receives the relationship's committed Change
            // in its inbox, triggering program dispatch in the same batch.
            // All three scopes (Specific, AllOfKind, TouchingLocus) are
            // resolved and deduplicated by `collect_subscribers`.
            let td = if profile { Some(std::time::Instant::now()) } else { None };
            for (rel_id, change_id, kind, from, to) in acc.pending_rel_notifications.drain(..) {
                let subscribers = world
                    .subscriptions()
                    .collect_subscribers(rel_id, kind, from, to);
                for subscriber in subscribers {
                    acc.committed_ids_by_locus.entry(subscriber).or_default().push(change_id);
                    if acc.affected_loci_set.insert(subscriber) {
                        acc.affected_loci.push(subscriber);
                    }
                }
            }

            // Dispatch programs for every locus that just received at
            // least one change.
            let batch_num = batch.0;
            let dispatch_inputs: Vec<DispatchInput> = acc.affected_loci
                .iter()
                .filter_map(|locus_id| {
                    let locus = world.locus(*locus_id)?;
                    let cfg = loci_registry.get_config(locus.kind)?;
                    // Refractory check: skip dispatch if this locus fired
                    // recently (within refractory_batches).
                    if cfg.refractory_batches > 0
                        && let Some(&fired_at) = last_fired.get(locus_id)
                        && batch_num.saturating_sub(fired_at) < cfg.refractory_batches as u64
                    {
                        return None;
                    }
                    let program = cfg.program.as_ref();
                    let inbox: Vec<&Change> = acc.committed_ids_by_locus
                        .get(locus_id)
                        .map(|ids| {
                            ids.iter()
                                .filter_map(|id| world.log().get(*id))
                                .collect()
                        })
                        .unwrap_or_default();
                    let derived: Vec<ChangeId> = inbox.iter().map(|c| c.id).collect();
                    Some(DispatchInput { locus, program, inbox, derived })
                })
                .collect();

            // Build a read-only context from the world's current stores.
            // slot_defs is borrowed from the registry — no per-batch allocation.
            let batch_ctx = graph_world::BatchContext::new(
                world.loci(), world.relationships(), world.log(),
                world.entities(), world.coheres(), batch,
                world.properties(), slot_defs,
            );

            let dispatch_results: Vec<DispatchResult> = dispatch_inputs
                .par_iter()
                .map(|inp| {
                    let state = inp.program.process(inp.locus, &inp.inbox, &batch_ctx);
                    let structural =
                        inp.program.structural_proposals(inp.locus, &inp.inbox, &batch_ctx);
                    (state, structural, inp.derived.clone())
                })
                .collect();

            for (idx, (mut state_proposals, structural, derived)) in
                dispatch_results.into_iter().enumerate()
            {
                if let Some(cfg) = loci_registry.get_config(dispatch_inputs[idx].locus.kind)
                    && let Some(max) = cfg.max_proposals_per_dispatch
                {
                    state_proposals.truncate(max);
                }
                if !state_proposals.is_empty() {
                    last_fired.insert(dispatch_inputs[idx].locus.id, batch_num);
                }
                pending.extend(state_proposals.into_iter().map(|p| PendingChange {
                    proposed: p,
                    derived_predecessors: derived.clone(),
                }));
                acc.structural_proposals.extend(structural);
            }
            if let Some(t) = td { t_dispatch += t.elapsed(); }

            // Apply structural proposals at end-of-batch.
            // Tombstone proposals (for deleted relationships with Specific
            // subscribers) are injected into `pending` for the next batch
            // so subscribers fire once more with a deletion signal in inbox.
            let tombstones = batch::apply_structural_proposals(
                world,
                std::mem::take(&mut acc.structural_proposals),
                influence_registry,
            );
            pending.extend(tombstones);

            // End-of-batch: apply Hebbian plasticity updates and cross-kind
            // interaction effects (delegated to world_ops for testability).
            let th = if profile { Some(std::time::Instant::now()) } else { None };
            let hebbian_changes =
                world_ops::apply_hebbian_updates(world, &acc.plasticity_obs, influence_registry);
            world_ops::apply_interaction_effects(world, &acc.batch_kind_touches, influence_registry);

            // Weight-based structural pruning: delete relationships whose weight
            // fell at or below `prune_weight_threshold` after a plasticity update.
            // Derived from hebbian_changes (O(n_changed)) — no extra world scan.
            // `apply_structural_proposals` ensures subscribers receive tombstone
            // notifications in the next batch.
            let weight_prune_proposals: Vec<StructuralProposal> = hebbian_changes
                .iter()
                .filter_map(|(rel_id, kind, _, after)| {
                    let cfg = influence_registry.get(*kind)?;
                    if cfg.prune_weight_threshold == 0.0 {
                        return None;
                    }
                    let w = after.as_slice().get(Relationship::WEIGHT_SLOT).copied().unwrap_or(0.0);
                    if w <= cfg.prune_weight_threshold {
                        Some(StructuralProposal::DeleteRelationship { rel_id: *rel_id })
                    } else {
                        None
                    }
                })
                .collect();
            if !weight_prune_proposals.is_empty() {
                let prune_tombstones = batch::apply_structural_proposals(
                    world,
                    weight_prune_proposals,
                    influence_registry,
                );
                pending.extend(prune_tombstones);
            }

            // Emit ChangeLog entries for every relationship whose weight was
            // updated by Hebbian/STDP plasticity. This mirrors the Pass B3
            // pattern used for emerged relationships (lines above) and allows
            // `relationship_activity_trend` / `relationship_weight_trend` to
            // track weight evolution via the standard ChangeLog query surface.
            if !hebbian_changes.is_empty() {
                let n_hebb = hebbian_changes.len();
                let hebb_base = world.reserve_change_ids(n_hebb);
                world.log_mut().reserve(n_hebb);
                let hebb_log: Vec<Change> = hebbian_changes
                    .into_iter()
                    .enumerate()
                    .map(|(i, (rel_id, kind, before, after))| Change {
                        id: ChangeId(hebb_base.0 + i as u64),
                        subject: ChangeSubject::Relationship(rel_id),
                        kind,
                        predecessors: vec![],
                        before,
                        after,
                        batch,
                        wall_time: None,
                        metadata: None,
                    })
                    .collect();
                world.extend_batch_changes(hebb_log);
            }

            // Decay is now lazy: accumulated decay is applied in
            // auto_emerge_relationship (on touch) and flushed before entity
            // recognition via flush_relationship_decay. No per-batch
            // O(all_relationships) scan needed.
            if let Some(t) = th { t_hebbian += t.elapsed(); }

            let to2 = if profile { Some(std::time::Instant::now()) } else { None };
            result.events.append(&mut acc.events);
            world.advance_batch();
            if let Some(t) = to2 { t_other += t.elapsed(); }
            result.batches_committed += 1;
        }

        if profile {
            eprintln!(
                "[engine profile] batches={} compute={:.1}ms build={:.1}ms apply={:.1}ms(locus={:.1} emerge={:.1} changelog={:.1} b3={:.1}) dispatch={:.1}ms hebbian={:.1}ms other={:.1}ms",
                result.batches_committed,
                t_compute.as_secs_f64() * 1000.0,
                t_build.as_secs_f64() * 1000.0,
                t_apply.as_secs_f64() * 1000.0,
                t_apply_locus.as_secs_f64() * 1000.0,
                t_apply_emerge.as_secs_f64() * 1000.0,
                t_apply_changelog.as_secs_f64() * 1000.0,
                t_apply_b3.as_secs_f64() * 1000.0,
                t_dispatch.as_secs_f64() * 1000.0,
                t_hebbian.as_secs_f64() * 1000.0,
                t_other.as_secs_f64() * 1000.0,
            );
        }
        result
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
    pub fn bootstrap_subscriptions(
        &self,
        world: &mut World,
        loci_registry: &LocusKindRegistry,
    ) {
        // Collect (locus_id, rel_ids) pairs to avoid borrow conflicts.
        let pending: Vec<(LocusId, Vec<RelationshipId>)> = world
            .loci()
            .iter()
            .filter_map(|locus| {
                let program = loci_registry.get(locus.kind)?;
                let subs = program.initial_subscriptions(locus);
                if subs.is_empty() { None } else { Some((locus.id, subs)) }
            })
            .collect();

        for (locus_id, rel_ids) in pending {
            for rel_id in rel_ids {
                world.subscriptions_mut().subscribe(locus_id, rel_id);
            }
        }
    }
}


// ── Apply-phase helper ────────────────────────────────────────────────────────

/// APPLY PHASE — execute a pre-resolved emergence decision.
///
/// Returns `(rel_id, is_new, initial_state)`:
/// - `rel_id == RelationshipId(u64::MAX)` signals `Blocked` — caller skips.
/// - `is_new == true` means a relationship was just created; `initial_state`
///   will be `Some(...)` and the caller should emit `RelationshipEmerged`.
/// - `is_new == false`: update path; `initial_state` is `None`.
///
/// ## Update path
/// Decay and activity arithmetic are performed in-place on the existing
/// relationship — no allocations.  `rel_id` was pre-resolved in the
/// parallel compute phase, saving the 2-level endpoint-key lookup here.
///
/// ## Create path
/// `initial_state` was pre-computed in the parallel compute phase.
/// Concurrent-creation is handled: if another resolution in the same batch
/// already inserted the same (endpoints, kind) pair, falls back to an
/// activity touch.
#[allow(clippy::too_many_arguments)]
fn apply_emergence(
    store: &mut RelationshipStore,
    resolution: EmergenceResolution,
    change_id: ChangeId,
    batch: graph_core::BatchId,
    kind: InfluenceKindId,
    pre_signal: f32,
    kind_cfg: Option<&crate::registry::InfluenceKindConfig>,
    resolved_slots: &[graph_core::RelationshipSlotDef],
) -> Option<(RelationshipId, bool, Option<StateVector>)> {
    match resolution {
        EmergenceResolution::Blocked => None,

        EmergenceResolution::Update { rel_id } => {
            // Hoist all config reads out of the hot get_mut path.
            let activity_decay        = kind_cfg.map_or(1.0, |c| c.decay_per_batch);
            let weight_decay          = kind_cfg.map_or(1.0, |c| c.plasticity.weight_decay);
            let activity_contribution = kind_cfg.map_or(1.0, |c| c.activity_contribution);
            let max_activity          = kind_cfg.and_then(|c| c.max_activity);
            let abs_signal            = pre_signal.abs();

            if let Some(rel) = store.get_mut(rel_id) {
                let delta = batch.0.saturating_sub(rel.last_decayed_batch);
                if delta > 0 {
                    // delta==1 is the common case; skip powi for a direct multiply.
                    let act_factor = if delta == 1 { activity_decay } else { activity_decay.powi(delta as i32) };
                    let wt_factor  = if delta == 1 { weight_decay   } else { weight_decay.powi(delta as i32) };
                    let slots = rel.state.as_mut_slice();
                    // Decay and activity addition merged into one slot[0] write.
                    if let Some(a) = slots.get_mut(Relationship::ACTIVITY_SLOT) {
                        *a = *a * act_factor + activity_contribution * abs_signal;
                        if let Some(cap) = max_activity {
                            *a = a.clamp(-cap, cap);
                        }
                    }
                    if let Some(w) = slots.get_mut(Relationship::WEIGHT_SLOT) {
                        *w *= wt_factor;
                    }
                    for (i, slot_def) in resolved_slots.iter().enumerate() {
                        if let Some(factor) = slot_def.decay {
                            if let Some(v) = slots.get_mut(2 + i) {
                                *v *= if delta == 1 { factor } else { factor.powi(delta as i32) };
                            }
                        }
                    }
                    rel.last_decayed_batch = batch.0;
                } else {
                    // delta==0: no decay needed, just add activity.
                    if let Some(a) = rel.state.as_mut_slice().get_mut(Relationship::ACTIVITY_SLOT) {
                        *a += activity_contribution * abs_signal;
                        if let Some(cap) = max_activity {
                            *a = a.clamp(-cap, cap);
                        }
                    }
                }
                rel.lineage.last_touched_by = Some(change_id);
                rel.lineage.change_count += 1;
                rel.lineage.observe_kind(kind, batch);
            }
            Some((rel_id, false, None))
        }

        EmergenceResolution::Create {
            endpoints, kind: rel_kind, initial_state,
            pre_signal: create_pre_signal, activity_contribution, max_activity,
        } => {
            let key = endpoints.key();
            // Guard against concurrent creation within the same batch: two
            // compute-phase resolutions may both see `lookup → None` for the
            // same (endpoints, kind) when they are part of separate
            // `PendingChange`s processed in parallel.  The second one falls
            // back to an activity touch instead of a duplicate insert.
            if let Some(existing_id) = store.lookup(&key, rel_kind) {
                let rel = store.get_mut(existing_id).expect("indexed id must exist");
                if let Some(slot) = rel.state.as_mut_slice().get_mut(Relationship::ACTIVITY_SLOT) {
                    *slot += activity_contribution * create_pre_signal.abs();
                    if let Some(cap) = max_activity {
                        *slot = slot.clamp(-cap, cap);
                    }
                }
                rel.lineage.last_touched_by = Some(change_id);
                rel.lineage.change_count += 1;
                rel.lineage.observe_kind(rel_kind, batch);
                Some((existing_id, false, None))
            } else {
                let new_id = store.mint_id();
                store.insert(Relationship {
                    id: new_id,
                    kind: rel_kind,
                    endpoints,
                    state: initial_state.clone(),
                    lineage: RelationshipLineage {
                        created_by: Some(change_id),
                        last_touched_by: Some(change_id),
                        change_count: 1,
                        kinds_observed: smallvec::smallvec![
                            KindObservation::once(rel_kind, batch)
                        ],
                    },
                    created_batch: batch,
                    last_decayed_batch: batch.0,
                    metadata: None,
                });
                Some((new_id, true, Some(initial_state)))
            }
        }
    }
}
