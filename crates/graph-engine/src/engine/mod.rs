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

use rustc_hash::{FxHashMap, FxHashSet};

use graph_core::{
    Change, ChangeId, ChangeSubject, LocusId, ProposedChange, Relationship,
    StructuralProposal,
};
use graph_world::World;

use crate::cohere::CoherePerspective;
use crate::emergence::EmergencePerspective;
use crate::registry::{InfluenceKindRegistry, LocusKindRegistry};

use batch::{DecayParams, DispatchInput, DispatchResult, PendingChange};

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

        // Pre-allocate outside the loop; clear + reuse each iteration.
        let mut committed_ids_by_locus: FxHashMap<LocusId, Vec<ChangeId>> = FxHashMap::default();
        let mut affected_loci: Vec<LocusId> = Vec::new();
        let mut affected_loci_set: FxHashSet<LocusId> = FxHashSet::default();
        let mut plasticity_obs: Vec<(graph_core::RelationshipId, graph_core::InfluenceKindId, f32, f32)> = Vec::new();
        let mut structural_proposals: Vec<StructuralProposal> = Vec::new();

        while !pending.is_empty() {
            if result.batches_committed >= self.config.max_batches_per_tick {
                result.hit_batch_cap = true;
                break;
            }

            let batch = world.current_batch();
            committed_ids_by_locus.clear();
            affected_loci.clear();
            affected_loci_set.clear();
            plasticity_obs.clear();
            structural_proposals.clear();

            for pending_change in pending.drain(..) {
                let PendingChange { proposed, derived_predecessors } = pending_change;
                let mut predecessors = derived_predecessors;
                predecessors.extend(proposed.extra_predecessors.iter().copied());
                let id = world.mint_change_id();
                let kind = proposed.kind;

                match proposed.subject {
                    ChangeSubject::Locus(locus_id) => {
                        // Drop changes targeting non-existent loci.
                        if world.locus(locus_id).is_none() {
                            continue;
                        }
                        let before = world
                            .locus(locus_id)
                            .map(|l| l.state.clone())
                            .unwrap_or_default();
                        let cross_locus_preds: Vec<(LocusId, f32)> = predecessors
                            .iter()
                            .filter_map(|pid| world.log().get(*pid))
                            .filter_map(|pred| match pred.subject {
                                ChangeSubject::Locus(pl) if pl != locus_id => {
                                    let pre = pred.after.as_slice().first().copied().unwrap_or(0.0);
                                    Some((pl, pre))
                                }
                                _ => None,
                            })
                            .collect();
                        let kind_cfg = influence_registry.get(kind);
                        let stabilized_after = match kind_cfg {
                            Some(cfg) => cfg.stabilization.stabilize(&before, proposed.after),
                            None => proposed.after,
                        };
                        let post_signal = stabilized_after.as_slice().first().copied().unwrap_or(0.0);
                        let change = Change {
                            id,
                            subject: ChangeSubject::Locus(locus_id),
                            kind,
                            predecessors,
                            before,
                            after: stabilized_after.clone(),
                            batch,
                        };
                        if let Some(locus) = world.locus_mut(locus_id) {
                            locus.state = stabilized_after;
                        }
                        world.append_change(change);
                        let plasticity_active = kind_cfg
                            .map(|cfg| cfg.plasticity.is_active())
                            .unwrap_or(false);
                        let decay = kind_cfg
                            .map(|cfg| DecayParams {
                                activity: cfg.decay_per_batch,
                                weight: cfg.plasticity.weight_decay,
                            })
                            .unwrap_or(DecayParams { activity: 1.0, weight: 1.0 });
                        for (from_locus, pre_signal) in cross_locus_preds {
                            let rel_id = batch::auto_emerge_relationship(
                                world, from_locus, locus_id, kind, id, batch.0, &decay,
                            );
                            if plasticity_active {
                                plasticity_obs.push((rel_id, kind, pre_signal, post_signal));
                            }
                        }
                        committed_ids_by_locus.entry(locus_id).or_default().push(id);
                        if affected_loci_set.insert(locus_id) {
                            affected_loci.push(locus_id);
                        }
                    }
                    ChangeSubject::Relationship(rel_id) => {
                        let before = world
                            .relationships()
                            .get(rel_id)
                            .map(|r| r.state.clone())
                            .unwrap_or_default();
                        let stabilized_after = match influence_registry.get(kind) {
                            Some(cfg) => cfg.stabilization.stabilize(&before, proposed.after),
                            None => proposed.after,
                        };
                        let change = Change {
                            id,
                            subject: ChangeSubject::Relationship(rel_id),
                            kind,
                            predecessors,
                            before,
                            after: stabilized_after.clone(),
                            batch,
                        };
                        if let Some(rel) = world.relationships_mut().get_mut(rel_id) {
                            rel.state = stabilized_after;
                            rel.lineage.last_touched_by = Some(id);
                            rel.lineage.change_count += 1;
                        }
                        world.append_change(change);
                    }
                }
                result.changes_committed += 1;
            }

            // Dispatch programs for every locus that just received at
            // least one change.
            let batch_num = batch.0;
            let dispatch_inputs: Vec<DispatchInput> = affected_loci
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
                    let inbox: Vec<&Change> = committed_ids_by_locus
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
            // All references are valid for the dispatch phase (no mutations
            // until apply_structural_proposals below).
            let batch_ctx = graph_world::BatchContext::new(
                world.loci(), world.relationships(), world.log(),
                world.entities(), world.coheres(), batch,
            );

            let dispatch_results: Vec<DispatchResult> = dispatch_inputs
                .iter()
                .map(|inp| {
                    let state = inp.program.process(inp.locus, &inp.inbox, &batch_ctx);
                    let structural = inp.program.structural_proposals(inp.locus, &inp.inbox, &batch_ctx);
                    (state, structural, inp.derived.clone())
                })
                .collect();

            for (idx, (state_proposals, structural, derived)) in dispatch_results.into_iter().enumerate() {
                if !state_proposals.is_empty() {
                    // Record that this locus fired for refractory tracking.
                    last_fired.insert(dispatch_inputs[idx].locus.id, batch_num);
                }
                pending.extend(state_proposals.into_iter().map(|p| PendingChange {
                    proposed: p,
                    derived_predecessors: derived.clone(),
                }));
                structural_proposals.extend(structural);
            }

            // Apply structural proposals at end-of-batch.
            batch::apply_structural_proposals(world, std::mem::take(&mut structural_proposals));

            // End-of-batch: apply Hebbian plasticity updates. Each
            // observation (rel_id, kind, pre, post) contributes
            // Δweight = η * pre * post, clamped to [0, max_weight].
            for (rel_id, kind, pre, post) in plasticity_obs.drain(..) {
                if let Some(cfg) = influence_registry.get(kind)
                    && let Some(rel) = world.relationships_mut().get_mut(rel_id)
                    && let Some(w) = rel.state.as_mut_slice().get_mut(Relationship::WEIGHT_SLOT)
                {
                    *w = (*w + cfg.plasticity.learning_rate * pre * post)
                        .clamp(0.0, cfg.plasticity.max_weight);
                }
            }

            // Decay is now lazy: accumulated decay is applied in
            // auto_emerge_relationship (on touch) and flushed before entity
            // recognition via flush_relationship_decay. No per-batch
            // O(all_relationships) scan needed.

            world.advance_batch();
            result.batches_committed += 1;
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
}

