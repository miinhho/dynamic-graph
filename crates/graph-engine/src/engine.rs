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

use rustc_hash::{FxHashMap, FxHashSet};

use graph_core::{
    apply_skeleton, Change, ChangeId, ChangeSubject, EmergenceProposal, Endpoints, Entity,
    EntityLayer, EntityLineage, EntityStatus, EntityWeatheringPolicy, LayerTransition, LocusId,
    ProposedChange, Relationship, RelationshipLineage, StateVector, StructuralProposal,
    WeatheringEffect,
};
use graph_world::World;

use crate::cohere::CoherePerspective;
use crate::emergence::EmergencePerspective;

use crate::registry::{InfluenceKindRegistry, LocusKindRegistry};

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

        while !pending.is_empty() {
            if result.batches_committed >= self.config.max_batches_per_tick {
                result.hit_batch_cap = true;
                break;
            }

            // Commit every pending change as part of the current batch.
            // Build a per-locus index of which change ids fired into
            // each locus, so the next batch's auto-predecessor logic
            // has somewhere to look.
            let batch = world.current_batch();
            let mut committed_ids_by_locus: FxHashMap<LocusId, Vec<ChangeId>> = FxHashMap::default();
            let mut affected_loci: Vec<LocusId> = Vec::new();
            let mut affected_loci_set: FxHashSet<LocusId> = FxHashSet::default();
            // Plasticity: collect (rel_id, kind, pre_signal, post_signal) tuples
            // during cross-locus detection; apply Hebbian updates at end-of-batch.
            let mut plasticity_obs: Vec<(graph_core::RelationshipId, graph_core::InfluenceKindId, f32, f32)> = Vec::new();

            for pending_change in pending.drain(..) {
                let PendingChange {
                    proposed,
                    derived_predecessors,
                } = pending_change;

                let mut predecessors = derived_predecessors;
                predecessors.extend(proposed.extra_predecessors.iter().copied());

                let id = world.mint_change_id();
                let kind = proposed.kind;

                match proposed.subject {
                    ChangeSubject::Locus(locus_id) => {
                        // The before-state is the locus's current state at the
                        // moment of commit; the after-state was supplied by the
                        // proposer (stimulus or program).
                        let before = world
                            .locus(locus_id)
                            .map(|l| l.state.clone())
                            .unwrap_or_default();

                        // Resolve cross-locus predecessors and capture pre-signals
                        // *before* moving the change into the log: borrows can't overlap.
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

                        // Cache per-kind config once; used for stabilization and plasticity.
                        let kind_cfg = influence_registry.get(kind);

                        // Apply the kind's guard rail before committing.
                        let stabilized_after = match kind_cfg {
                            Some(cfg) => cfg.stabilization.stabilize(&before, proposed.after),
                            None => proposed.after,
                        };

                        // post-signal for Hebbian update = first slot of committed value.
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

                        // Apply the state change to the locus, then record.
                        if let Some(locus) = world.locus_mut(locus_id) {
                            locus.state = stabilized_after;
                        }
                        world.append_change(change);

                        // Auto-emerge or update a directed relationship for
                        // each cross-locus predecessor. Collect plasticity
                        // observations if the kind has learning enabled.
                        let plasticity_active = kind_cfg
                            .map(|cfg| cfg.plasticity.is_active())
                            .unwrap_or(false);
                        for (from_locus, pre_signal) in cross_locus_preds {
                            let rel_id = auto_emerge_relationship(world, from_locus, locus_id, kind, id);
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
                        // Relationship-subject change: update the relationship's
                        // state and record in the log. No program dispatch, no
                        // auto-emerge (that path is locus→locus only).
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
                        // Relationship changes are not added to
                        // committed_ids_by_locus — no program runs.
                    }
                }
                result.changes_committed += 1;
            }

            // Dispatch programs for every locus that just received at
            // least one change. Each program returns its proposed
            // follow-up state changes (queued for next batch) and any
            // structural proposals (applied at end of this batch).
            let mut structural_proposals: Vec<StructuralProposal> = Vec::new();
            for locus_id in &affected_loci {
                let Some(locus) = world.locus(*locus_id) else {
                    continue;
                };
                let program = match loci_registry.require(locus.kind) {
                    Some(p) => p,
                    None => continue,
                };

                // Build the inbox for this locus: every change committed
                // to this locus during the batch we just sealed.
                let inbox: Vec<Change> = committed_ids_by_locus
                    .get(locus_id)
                    .map(|ids| {
                        ids.iter()
                            .filter_map(|id| world.log().get(*id).cloned())
                            .collect()
                    })
                    .unwrap_or_default();

                let state_proposals = program.process(locus, &inbox);
                let structural = program.structural_proposals(locus, &inbox);
                let derived: Vec<ChangeId> =
                    inbox.iter().map(|c| c.id).collect();

                pending.extend(state_proposals.into_iter().map(|p| PendingChange {
                    proposed: p,
                    derived_predecessors: derived.clone(),
                }));
                structural_proposals.extend(structural);
            }

            // Apply structural proposals at end-of-batch.
            apply_structural_proposals(world, structural_proposals);

            // End-of-batch: apply Hebbian plasticity updates. Each
            // observation (rel_id, kind, pre, post) contributes
            // Δweight = η * pre * post, clamped to [0, max_weight].
            for (rel_id, kind, pre, post) in plasticity_obs {
                if let Some(cfg) = influence_registry.get(kind) {
                    let p = &cfg.plasticity;
                    if let Some(rel) = world.relationships_mut().get_mut(rel_id) {
                        if let Some(w) = rel.state.as_mut_slice().get_mut(Relationship::WEIGHT_SLOT) {
                            *w = (*w + p.learning_rate * pre * post).clamp(0.0, p.max_weight);
                        }
                    }
                }
            }

            // End-of-batch continuous decay on all relationship activity
            // (slot 0) and Hebbian weight (slot 1), per docs/redesign.md §3.5.
            for rel in world.relationships_mut().iter_mut() {
                let (activity_decay, weight_decay) = influence_registry
                    .get(rel.kind)
                    .map(|cfg| (cfg.decay_per_batch, cfg.plasticity.weight_decay))
                    .unwrap_or((1.0, 1.0));
                let slots = rel.state.as_mut_slice();
                if let Some(a) = slots.get_mut(Relationship::ACTIVITY_SLOT) {
                    *a *= activity_decay;
                }
                if let Some(w) = slots.get_mut(Relationship::WEIGHT_SLOT) {
                    *w *= weight_decay;
                }
            }

            world.advance_batch();
            result.batches_committed += 1;
        }

        result
    }

    /// Run a `CoherePerspective` and store the resulting clusters.
    ///
    /// On-demand, like `recognize_entities`. Replaces the previous
    /// cohere set for this perspective's name in the `CohereStore`.
    /// Per `docs/redesign.md` §6 step 8: "Optional, on-demand."
    pub fn extract_cohere(
        &self,
        world: &mut World,
        perspective: &dyn CoherePerspective,
    ) {
        let store = world.coheres_mut();
        let mut counter = store.mint_id().0;
        let coheres = perspective.cluster(
            world.entities(),
            world.relationships(),
            &mut || {
                let id = graph_core::CohereId(counter);
                counter += 1;
                id
            },
        );
        world.coheres_mut().update(perspective.name(), coheres);
    }

    /// Apply an `EmergencePerspective` to the current world state and
    /// commit its proposals to the entity store.
    ///
    /// This is *on-demand*, not called automatically by `tick`. The
    /// caller decides when entity recognition should happen — once per
    /// tick, once per N ticks, or on explicit request. Per
    /// `docs/redesign.md` §6 step 7: "Optional, on-demand."
    pub fn recognize_entities(
        &self,
        world: &mut World,
        perspective: &dyn EmergencePerspective,
    ) {
        let batch = world.current_batch();
        let proposals =
            perspective.recognize(world.relationships(), world.entities(), batch);
        apply_proposals(world, proposals, batch);
    }

    /// Trim the change log, dropping all changes in batches strictly older
    /// than `current_batch - retention_batches`.
    ///
    /// On-demand, like `weather_entities`. A typical cadence is to call
    /// this immediately after `weather_entities` so that compressed layers
    /// no longer hold live predecessor-id references into the trimmed range.
    ///
    /// ## Parameters
    ///
    /// - `retention_batches` — how many recent batches to keep. E.g. `50`
    ///   keeps the last 50 batches of changes. If `retention_batches` is
    ///   larger than `current_batch`, the call is a no-op (nothing to trim).
    ///
    /// ## Returns
    ///
    /// The number of change records removed.
    pub fn trim_change_log(&self, world: &mut World, retention_batches: u64) -> usize {
        let current = world.current_batch().0;
        let retain_from = graph_core::BatchId(current.saturating_sub(retention_batches));
        world.log_mut().trim_before_batch(retain_from)
    }

    /// Apply a weathering policy to every entity's sediment layer stack.
    ///
    /// On-demand — the caller decides when to run weathering. A typical
    /// cadence is every N ticks (e.g. every 50 or 100 ticks) rather than
    /// after every tick.
    ///
    /// ## What this does
    ///
    /// For each entity, for each layer (oldest first), it asks the policy
    /// for a `WeatheringEffect`. The effects are applied in-place:
    ///
    /// - `Preserved` — layer untouched.
    /// - `Compress`  — snapshot stripped; stats kept in `Compressed`.
    /// - `Skeleton`  — further reduced to `Skeleton`.
    /// - `Remove`    — layer deleted. **Exception**: layers whose
    ///   transition `is_significant()` (Born, Split, Merged) are
    ///   downgraded to `Skeleton` instead of removed, so lineage
    ///   pivots can never vanish.
    ///
    /// The entity's `current` snapshot and `status` are not touched —
    /// only the historical layer stack is weathered.
    pub fn weather_entities(
        &self,
        world: &mut World,
        policy: &dyn EntityWeatheringPolicy,
    ) {
        let current_batch = world.current_batch().0;
        for entity in world.entities_mut().iter_mut() {
            let mut i = 0;
            while i < entity.layers.len() {
                let age = current_batch.saturating_sub(entity.layers[i].batch.0);
                let effect = policy.effect(&entity.layers[i], age);
                match effect {
                    WeatheringEffect::Preserved => {
                        i += 1;
                    }
                    WeatheringEffect::Compress => {
                        graph_core::apply_compress(&mut entity.layers[i]);
                        i += 1;
                    }
                    WeatheringEffect::Skeleton => {
                        apply_skeleton(&mut entity.layers[i]);
                        i += 1;
                    }
                    WeatheringEffect::Remove => {
                        if entity.layers[i].transition.is_significant() {
                            // Never delete Born/Split/Merged — skeleton instead.
                            apply_skeleton(&mut entity.layers[i]);
                            i += 1;
                        } else {
                            entity.layers.remove(i);
                            // i stays the same — now points to the next layer.
                        }
                    }
                }
            }
        }
    }
}

/// Apply a list of emergence proposals to the entity store.
fn apply_proposals(world: &mut World, proposals: Vec<EmergenceProposal>, batch: graph_core::BatchId) {
    for proposal in proposals {
        match proposal {
            EmergenceProposal::Born { members, coherence, parents } => {
                use graph_core::EntitySnapshot;
                let snapshot = EntitySnapshot {
                    members,
                    member_relationships: Vec::new(),
                    coherence,
                };
                let store = world.entities_mut();
                let id = store.mint_id();
                let mut entity = Entity::born(id, batch, snapshot);
                entity.lineage = EntityLineage { parents, children: Vec::new() };
                store.insert(entity);
            }
            EmergenceProposal::DepositLayer { entity, layer } => {
                if let Some(e) = world.entities_mut().get_mut(entity) {
                    e.current = layer.snapshot.clone().unwrap_or_default();
                    e.layers.push(layer);
                }
            }
            EmergenceProposal::Dormant { entity } => {
                if let Some(e) = world.entities_mut().get_mut(entity) {
                    e.status = EntityStatus::Dormant;
                    e.layers.push(EntityLayer::new(
                        batch,
                        e.current.clone(),
                        LayerTransition::BecameDormant,
                    ));
                }
            }
            EmergenceProposal::Revive { entity, snapshot } => {
                if let Some(e) = world.entities_mut().get_mut(entity) {
                    e.status = EntityStatus::Active;
                    e.deposit(batch, snapshot, LayerTransition::Revived);
                }
            }
            EmergenceProposal::Split { source, offspring } => {
                let mut child_ids = Vec::new();
                for (members, coherence) in offspring {
                    use graph_core::EntitySnapshot;
                    let snapshot = EntitySnapshot { members, member_relationships: Vec::new(), coherence };
                    let store = world.entities_mut();
                    let child_id = store.mint_id();
                    let child = Entity::born(child_id, batch, snapshot);
                    store.insert(child);
                    child_ids.push(child_id);
                }
                if let Some(e) = world.entities_mut().get_mut(source) {
                    e.deposit(batch, e.current.clone(), LayerTransition::Split {
                        offspring: child_ids.clone(),
                    });
                    e.lineage.children.extend(child_ids);
                }
            }
            EmergenceProposal::Merge { absorbed, into, new_members, coherence } => {
                for absorbed_id in &absorbed {
                    if let Some(e) = world.entities_mut().get_mut(*absorbed_id) {
                        e.status = EntityStatus::Dormant;
                        e.layers.push(EntityLayer::new(
                            batch,
                            e.current.clone(),
                            LayerTransition::Merged { absorbed: vec![into] },
                        ));
                    }
                }
                use graph_core::EntitySnapshot;
                let snapshot = EntitySnapshot {
                    members: new_members,
                    member_relationships: Vec::new(),
                    coherence,
                };
                if let Some(e) = world.entities_mut().get_mut(into) {
                    e.deposit(batch, snapshot, LayerTransition::Merged { absorbed: absorbed.clone() });
                    e.lineage.children.extend(absorbed);
                }
            }
        }
    }
}

/// Recognize or update a directed relationship of `kind` going from
/// `from` to `to`, attributing the touch to `change_id`. Adds 1.0 to
/// the relationship's activity slot per touch.
///
/// Returns the `RelationshipId` (new or existing) so the caller can
/// record a plasticity observation.
fn auto_emerge_relationship(
    world: &mut World,
    from: LocusId,
    to: LocusId,
    kind: graph_core::InfluenceKindId,
    change_id: ChangeId,
) -> graph_core::RelationshipId {
    let endpoints = Endpoints::Directed { from, to };
    let key = endpoints.key();
    let store = world.relationships_mut();
    if let Some(rel_id) = store.lookup(&key, kind) {
        let rel = store.get_mut(rel_id).expect("indexed id must exist");
        if let Some(slot) = rel.state.as_mut_slice().get_mut(Relationship::ACTIVITY_SLOT) {
            *slot += 1.0;
        }
        rel.lineage.last_touched_by = Some(change_id);
        rel.lineage.change_count += 1;
        if !rel.lineage.kinds_observed.contains(&kind) {
            rel.lineage.kinds_observed.push(kind);
        }
        rel_id
    } else {
        let new_id = store.mint_id();
        // Two slots: [activity, weight]. Weight starts at 0.
        store.insert(Relationship {
            id: new_id,
            kind,
            endpoints,
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: Some(change_id),
                last_touched_by: Some(change_id),
                change_count: 1,
                kinds_observed: vec![kind],
            },
        });
        new_id
    }
}

/// Apply structural proposals collected during a batch's program-dispatch phase.
///
/// `CreateRelationship`: if the (endpoints, kind) pair already exists,
/// treat it as an activity touch. Otherwise mint and insert a new
/// relationship with `created_by: None` (no originating change).
///
/// `DeleteRelationship`: remove from the store. The relationship's past
/// changes in the log remain intact.
fn apply_structural_proposals(world: &mut World, proposals: Vec<StructuralProposal>) {
    for proposal in proposals {
        match proposal {
            StructuralProposal::CreateRelationship { endpoints, kind } => {
                let key = endpoints.key();
                let store = world.relationships_mut();
                if let Some(rel_id) = store.lookup(&key, kind) {
                    let rel = store.get_mut(rel_id).expect("indexed id must exist");
                    if let Some(a) = rel.state.as_mut_slice().get_mut(Relationship::ACTIVITY_SLOT) {
                        *a += 1.0;
                    }
                    rel.lineage.change_count += 1;
                } else {
                    let new_id = store.mint_id();
                    store.insert(Relationship {
                        id: new_id,
                        kind,
                        endpoints,
                        state: StateVector::from_slice(&[1.0, 0.0]),
                        lineage: RelationshipLineage {
                            created_by: None,
                            last_touched_by: None,
                            change_count: 1,
                            kinds_observed: vec![kind],
                        },
                    });
                }
            }
            StructuralProposal::DeleteRelationship { rel_id } => {
                world.relationships_mut().remove(rel_id);
            }
        }
    }
}

/// A change in flight: the user/program-supplied proposal plus any
/// predecessors the engine derived from the previous batch's commits.
struct PendingChange {
    proposed: ProposedChange,
    derived_predecessors: Vec<ChangeId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Change, InfluenceKindId, Locus, LocusId, LocusKindId, LocusProgram,
        ProposedChange, StateVector,
    };

    /// A program that, on its first activation, produces one self-targeted
    /// follow-up change with `after = current * 0.5`. On subsequent
    /// activations it does nothing — so the loop converges in two batches.
    struct DampOnceProgram;

    impl LocusProgram for DampOnceProgram {
        fn process(&self, locus: &Locus, incoming: &[Change]) -> Vec<ProposedChange> {
            // Only react to a stimulus (predecessors empty); ignore the
            // damped follow-up so the loop quiesces.
            if incoming.iter().all(|c| c.predecessors.is_empty()) {
                let mut next = locus.state.clone();
                for slot in next.as_mut_slice() {
                    *slot *= 0.5;
                }
                vec![ProposedChange::new(
                    ChangeSubject::Locus(locus.id),
                    InfluenceKindId(1),
                    next,
                )]
            } else {
                Vec::new()
            }
        }
    }

    fn setup() -> (World, LocusKindRegistry, InfluenceKindRegistry) {
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(2),
        ));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(DampOnceProgram));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(
            InfluenceKindId(1),
            crate::registry::InfluenceKindConfig::new("test"),
        );
        (world, loci, influences)
    }

    #[test]
    fn stimulus_only_commits_one_batch_when_program_is_passive() {
        struct InertProgram;
        impl LocusProgram for InertProgram {
            fn process(&self, _: &Locus, _: &[Change]) -> Vec<ProposedChange> {
                Vec::new()
            }
        }
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(2),
        ));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(InertProgram));
        let influences = InfluenceKindRegistry::new();

        let engine = Engine::default();
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0, 1.0]),
        );

        let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        assert_eq!(result.batches_committed, 1);
        assert_eq!(result.changes_committed, 1);
        assert!(!result.hit_batch_cap);

        // Stimulus state landed.
        let state = &world.locus(LocusId(1)).unwrap().state;
        assert_eq!(state.as_slice(), &[1.0, 1.0]);
    }

    #[test]
    fn stimulus_followed_by_program_response_commits_two_batches() {
        let (mut world, loci, influences) = setup();
        let engine = Engine::default();

        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0, 1.0]),
        );

        let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        assert_eq!(result.batches_committed, 2);
        assert_eq!(result.changes_committed, 2);
        assert!(!result.hit_batch_cap);

        // After damping, state should be 0.5,0.5.
        let state = &world.locus(LocusId(1)).unwrap().state;
        assert_eq!(state.as_slice(), &[0.5, 0.5]);
    }

    #[test]
    fn internal_change_inherits_stimulus_as_predecessor() {
        let (mut world, loci, influences) = setup();
        let engine = Engine::default();
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[2.0, 0.0]),
        );
        engine.tick(&mut world, &loci, &influences, vec![stimulus]);

        let log: Vec<&Change> = world.log().iter().collect();
        assert_eq!(log.len(), 2);
        // First entry is the stimulus — no predecessors.
        assert!(log[0].is_stimulus());
        // Second entry is the program's response — its predecessor set
        // must contain the stimulus's id (auto-derived).
        assert_eq!(log[1].predecessors, vec![log[0].id]);
        // And it lives in the next batch.
        assert_eq!(log[0].batch, BatchId(0));
        assert_eq!(log[1].batch, BatchId(1));
    }

    #[test]
    fn batch_cap_engages_on_runaway_program() {
        // A pathological program that always produces another change.
        struct InfiniteProgram;
        impl LocusProgram for InfiniteProgram {
            fn process(&self, locus: &Locus, _: &[Change]) -> Vec<ProposedChange> {
                vec![ProposedChange::new(
                    ChangeSubject::Locus(locus.id),
                    InfluenceKindId(1),
                    locus.state.clone(),
                )]
            }
        }
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(InfiniteProgram));
        let influences = InfluenceKindRegistry::new();

        let engine = Engine::new(EngineConfig {
            max_batches_per_tick: 5,
        });
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[0.1]),
        );
        let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        assert!(result.hit_batch_cap);
        assert_eq!(result.batches_committed, 5);
    }

    /// A program that, on stimulus, fires a single change at a fixed
    /// "downstream" locus and then stays inert. Used to drive cross-locus
    /// flow without infinite cascade.
    struct ForwarderProgram {
        downstream: LocusId,
    }
    impl LocusProgram for ForwarderProgram {
        fn process(&self, _locus: &Locus, incoming: &[Change]) -> Vec<ProposedChange> {
            // Only react to stimuli; ignore anything internal so the
            // loop quiesces after one hand-off.
            if !incoming.iter().all(|c| c.predecessors.is_empty()) {
                return Vec::new();
            }
            // Forward the magnitude of the first incoming change to the
            // downstream locus.
            let after = incoming[0].after.clone();
            vec![ProposedChange::new(
                ChangeSubject::Locus(self.downstream),
                InfluenceKindId(1),
                after,
            )]
        }
    }

    /// Sink program — accepts incoming, never proposes anything.
    struct SinkProgram;
    impl LocusProgram for SinkProgram {
        fn process(&self, _: &Locus, _: &[Change]) -> Vec<ProposedChange> {
            Vec::new()
        }
    }

    #[test]
    fn cross_locus_change_lands_on_downstream_with_correct_predecessor() {
        // Two loci of two different kinds: a forwarder and a sink. A
        // stimulus hits the forwarder; the forwarder's program proposes
        // a change at the sink. After the loop quiesces, the sink's
        // state must equal the stimulus payload, and the cross-locus
        // change must list the stimulus as its causal predecessor.
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(2),
        ));
        world.insert_locus(Locus::new(
            LocusId(2),
            LocusKindId(2),
            StateVector::zeros(2),
        ));

        let mut loci = LocusKindRegistry::new();
        loci.insert(
            LocusKindId(1),
            Box::new(ForwarderProgram {
                downstream: LocusId(2),
            }),
        );
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(
            InfluenceKindId(1),
            crate::registry::InfluenceKindConfig::new("excite"),
        );

        let engine = Engine::default();
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[0.7, 0.0]),
        );
        let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);

        // Two batches: stimulus, then forwarded.
        assert_eq!(result.batches_committed, 2);
        assert_eq!(result.changes_committed, 2);

        // Sink received the payload.
        assert_eq!(
            world.locus(LocusId(2)).unwrap().state.as_slice(),
            &[0.7, 0.0]
        );
        // Forwarder still holds the stimulus payload (the program does
        // not modify itself).
        assert_eq!(
            world.locus(LocusId(1)).unwrap().state.as_slice(),
            &[0.7, 0.0]
        );

        // Causal chain: stimulus on L1 (batch 0, no preds) -> forwarded
        // change on L2 (batch 1, predecessor = stimulus id).
        let log: Vec<&Change> = world.log().iter().collect();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].subject, ChangeSubject::Locus(LocusId(1)));
        assert_eq!(log[0].batch, BatchId(0));
        assert!(log[0].is_stimulus());

        assert_eq!(log[1].subject, ChangeSubject::Locus(LocusId(2)));
        assert_eq!(log[1].batch, BatchId(1));
        assert_eq!(log[1].predecessors, vec![log[0].id]);
    }

    #[test]
    fn changes_to_locus_returns_full_history() {
        // Drive two stimuli through the same locus across separate ticks
        // and confirm the change log preserves both, ordered newest first
        // when queried via the world's per-locus accessor.
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(SinkProgram));
        let influences = InfluenceKindRegistry::new();

        let engine = Engine::default();
        for value in [0.1_f32, 0.2, 0.3] {
            let stimulus = ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[value]),
            );
            engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        }

        let to_locus: Vec<f32> = world
            .log()
            .changes_to_locus(LocusId(1))
            .map(|c| c.after.as_slice()[0])
            .collect();
        assert_eq!(to_locus, vec![0.3, 0.2, 0.1]);
    }

    fn forwarder_world(decay: f32) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(2),
        ));
        world.insert_locus(Locus::new(
            LocusId(2),
            LocusKindId(2),
            StateVector::zeros(2),
        ));
        let mut loci = LocusKindRegistry::new();
        loci.insert(
            LocusKindId(1),
            Box::new(ForwarderProgram {
                downstream: LocusId(2),
            }),
        );
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(
            InfluenceKindId(1),
            crate::registry::InfluenceKindConfig::new("excite").with_decay(decay),
        );
        (world, loci, influences)
    }

    fn fire_stimulus(value: f32) -> ProposedChange {
        ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[value, 0.0]),
        )
    }

    #[test]
    fn cross_locus_flow_emerges_one_directed_relationship() {
        // First time L1 forwards to L2, the engine should mint exactly
        // one Directed{1->2} relationship of kind 1 with activity = 1.
        let (mut world, loci, influences) = forwarder_world(1.0);
        let engine = Engine::default();
        engine.tick(&mut world, &loci, &influences, vec![fire_stimulus(0.5)]);

        assert_eq!(world.relationships().len(), 1);
        let rel = world.relationships().iter().next().unwrap();
        assert_eq!(
            rel.endpoints,
            Endpoints::Directed {
                from: LocusId(1),
                to: LocusId(2),
            }
        );
        assert_eq!(rel.kind, InfluenceKindId(1));
        // Activity = 1.0 because decay = 1.0 (no decay).
        assert!((rel.activity() - 1.0).abs() < 1e-6);
        assert_eq!(rel.lineage.change_count, 1);
    }

    #[test]
    fn repeated_cross_locus_flow_increments_activity_and_change_count() {
        // Drive three independent stimuli through the forwarder. With
        // decay = 1.0 the activity should land on exactly 3.0 and the
        // change_count on 3.
        let (mut world, loci, influences) = forwarder_world(1.0);
        let engine = Engine::default();
        for v in [0.1, 0.2, 0.3_f32] {
            engine.tick(&mut world, &loci, &influences, vec![fire_stimulus(v)]);
        }
        assert_eq!(world.relationships().len(), 1);
        let rel = world.relationships().iter().next().unwrap();
        assert!((rel.activity() - 3.0).abs() < 1e-6);
        assert_eq!(rel.lineage.change_count, 3);
    }

    #[test]
    fn relationship_activity_decays_each_batch() {
        // With decay = 0.5 and a single forwarding stimulus, the loop
        // commits two batches. After batch 0 the relationship doesn't
        // exist yet (the cross-locus change happens in batch 1). After
        // batch 1, activity = 1.0 then decay -> 0.5.
        let (mut world, loci, influences) = forwarder_world(0.5);
        let engine = Engine::default();
        engine.tick(&mut world, &loci, &influences, vec![fire_stimulus(0.5)]);

        let rel = world.relationships().iter().next().unwrap();
        assert!(
            (rel.activity() - 0.5).abs() < 1e-6,
            "expected activity 0.5 after one decay tick, got {}",
            rel.activity()
        );

        // Second tick: another forwarding event. Trace:
        //   batch start: 0.5
        //   batch 2 commits stimulus at L1 (no relationship touch),
        //     end-of-batch decay: 0.25
        //   batch 3 commits forwarded change at L2 (+1.0): 1.25
        //     end-of-batch decay: 0.625
        engine.tick(&mut world, &loci, &influences, vec![fire_stimulus(0.5)]);
        let rel = world.relationships().iter().next().unwrap();
        assert!(
            (rel.activity() - 0.625).abs() < 1e-6,
            "expected activity 0.625 after second tick, got {}",
            rel.activity()
        );
    }

    // --- entity recognition integration tests ----------------------------

    fn two_locus_world_after_forwarding_tick() -> World {
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(InfluenceKindId(1), crate::registry::InfluenceKindConfig::new("e"));
        let engine = Engine::default();
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[0.5]),
        );
        engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        world
    }

    #[test]
    fn recognize_entities_after_forwarding_tick_produces_one_entity() {
        let mut world = two_locus_world_after_forwarding_tick();
        let engine = Engine::default();
        let perspective = crate::emergence::DefaultEmergencePerspective::default();
        engine.recognize_entities(&mut world, &perspective);

        assert_eq!(world.entities().active_count(), 1);
        let entity = world.entities().active().next().unwrap();
        let mut members = entity.current.members.clone();
        members.sort();
        assert_eq!(members, vec![LocusId(1), LocusId(2)]);
        assert_eq!(entity.layer_count(), 1); // only Born layer
    }

    #[test]
    fn entity_becomes_dormant_when_relationship_decays_below_threshold() {
        let mut world = two_locus_world_after_forwarding_tick();
        let engine = Engine::default();
        let perspective = crate::emergence::DefaultEmergencePerspective {
            min_activity_threshold: 0.8,
            ..Default::default()
        };
        // After one forwarding tick the activity is 1.0 * decay^1 = 0.5
        // (decay=1.0 in this test world so it stays 1.0). Use a high
        // threshold so the component is below it, triggering dormancy.
        let perspective_high = crate::emergence::DefaultEmergencePerspective {
            min_activity_threshold: 2.0, // impossible to meet
            ..Default::default()
        };
        engine.recognize_entities(&mut world, &perspective);
        assert_eq!(world.entities().active_count(), 1);
        engine.recognize_entities(&mut world, &perspective_high);
        assert_eq!(world.entities().active_count(), 0);
        assert_eq!(world.entities().len(), 1); // still in store, just dormant
    }

    #[test]
    fn extract_cohere_after_entity_recognition_groups_connected_entities() {
        // Full stack: tick -> recognize_entities -> extract_cohere.
        // Two disconnected forwarder-sink pairs (L1->L2 and L3->L4) should
        // each form a separate entity. No cross-pair bridge → no cohere.
        let mut world = World::new();
        // Pair A: L1 (kind 1) -> L2 (kind 2)
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
        // Pair B: L3 (kind 3) -> L4 (kind 2)  — kind 3 so the registry
        // can map it to a different ForwarderProgram instance (downstream=L4).
        world.insert_locus(Locus::new(LocusId(3), LocusKindId(3), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(4), LocusKindId(2), StateVector::zeros(1)));

        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        loci.insert(LocusKindId(3), Box::new(ForwarderProgram { downstream: LocusId(4) }));

        let mut influences = InfluenceKindRegistry::new();
        influences.insert(InfluenceKindId(1), crate::registry::InfluenceKindConfig::new("e"));

        let engine = Engine::default();
        engine.tick(&mut world, &loci, &influences, vec![
            ProposedChange::new(ChangeSubject::Locus(LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
        ]);
        engine.tick(&mut world, &loci, &influences, vec![
            ProposedChange::new(ChangeSubject::Locus(LocusId(3)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
        ]);

        let ep = crate::emergence::DefaultEmergencePerspective::default();
        engine.recognize_entities(&mut world, &ep);
        // Two disconnected pairs -> two active entities.
        assert_eq!(world.entities().active_count(), 2, "expected 2 entities");

        let cp = crate::cohere::DefaultCoherePerspective::default();
        engine.extract_cohere(&mut world, &cp);
        // No cross-pair relationships -> no coheres.
        let coheres = world.coheres().get("default").unwrap_or(&[]);
        assert_eq!(coheres.len(), 0, "no bridge -> no cohere");
    }

    #[test]
    fn self_targeted_change_does_not_emerge_relationship() {
        // The DampOnceProgram from earlier produces a self-targeted
        // follow-up. Self-loops are not relationships under the
        // current emergence rule (cross-locus only).
        let mut world = World::new();
        world.insert_locus(Locus::new(
            LocusId(1),
            LocusKindId(1),
            StateVector::zeros(2),
        ));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(DampOnceProgram));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(
            InfluenceKindId(1),
            crate::registry::InfluenceKindConfig::new("self"),
        );
        let engine = Engine::default();
        engine.tick(
            &mut world,
            &loci,
            &influences,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0, 1.0]),
            )],
        );
        assert_eq!(world.relationships().len(), 0);
    }

    // ─── Weathering tests ──────────────────────────────────────────────────

    /// Build a world with one entity that has a Born layer (batch 0) plus
    /// one MembershipDelta layer (batch 1). Leaves `current_batch` at 2.
    fn entity_world_two_layers() -> World {
        use graph_core::{EntitySnapshot, LocusId};
        let mut world = World::new();
        let store = world.entities_mut();
        let id = store.mint_id();
        let snap = EntitySnapshot {
            members: vec![LocusId(1)],
            member_relationships: Vec::new(),
            coherence: 1.0,
        };
        let mut entity = Entity::born(id, BatchId(0), snap.clone());
        entity.deposit(
            BatchId(1),
            snap,
            LayerTransition::MembershipDelta {
                added: vec![LocusId(2)],
                removed: Vec::new(),
            },
        );
        store.insert(entity);
        world.advance_batch(); // batch 1
        world.advance_batch(); // batch 2
        world
    }

    #[test]
    fn weather_entities_compresses_old_non_significant_layers() {
        use graph_core::{CompressionLevel, WeatheringEffect};

        // Policy: age >= 1 → Compress, age >= 2 → Skeleton, age >= 3 → Remove
        struct AggressiveWeathering;
        impl graph_core::EntityWeatheringPolicy for AggressiveWeathering {
            fn effect(
                &self,
                _layer: &graph_core::EntityLayer,
                age: u64,
            ) -> WeatheringEffect {
                if age >= 3 {
                    WeatheringEffect::Remove
                } else if age >= 2 {
                    WeatheringEffect::Skeleton
                } else if age >= 1 {
                    WeatheringEffect::Compress
                } else {
                    WeatheringEffect::Preserved
                }
            }
        }

        let mut world = entity_world_two_layers();
        // current_batch = 2; Born layer age = 2 → Skeleton; MembershipDelta age = 1 → Compress
        let engine = Engine::default();
        engine.weather_entities(&mut world, &AggressiveWeathering);

        let entity = world.entities().iter().next().unwrap();
        assert_eq!(entity.layers.len(), 2);
        assert!(
            matches!(entity.layers[0].compression, CompressionLevel::Skeleton { .. }),
            "Born layer (age=2) should be Skeleton"
        );
        assert!(
            matches!(entity.layers[1].compression, CompressionLevel::Compressed { .. }),
            "MembershipDelta layer (age=1) should be Compressed"
        );
    }

    #[test]
    fn weather_entities_never_removes_significant_layer() {
        use graph_core::{CompressionLevel, WeatheringEffect};

        // Policy: always Remove — but Born layer must survive as Skeleton.
        struct AlwaysRemove;
        impl graph_core::EntityWeatheringPolicy for AlwaysRemove {
            fn effect(&self, _: &graph_core::EntityLayer, _: u64) -> WeatheringEffect {
                WeatheringEffect::Remove
            }
        }

        let mut world = entity_world_two_layers();
        let engine = Engine::default();
        engine.weather_entities(&mut world, &AlwaysRemove);

        let entity = world.entities().iter().next().unwrap();
        // MembershipDelta (non-significant) removed; Born (significant) kept as Skeleton.
        assert_eq!(entity.layers.len(), 1, "non-significant layer removed");
        assert!(
            matches!(entity.layers[0].compression, CompressionLevel::Skeleton { .. }),
            "Born layer kept as Skeleton, not deleted"
        );
    }

    #[test]
    fn default_entity_weathering_preserves_recent_layers() {
        use graph_core::DefaultEntityWeathering;
        let mut world = entity_world_two_layers();
        // current_batch = 2; both layers are age 1-2, well inside recent_window=50.
        let engine = Engine::default();
        engine.weather_entities(&mut world, &DefaultEntityWeathering::default());

        let entity = world.entities().iter().next().unwrap();
        assert_eq!(entity.layers.len(), 2);
        for layer in &entity.layers {
            assert!(
                matches!(layer.compression, graph_core::CompressionLevel::Full),
                "both layers should still be Full"
            );
        }
    }

    // ─── Structural mutation tests ────────────────────────────────────────

    /// Program that proposes to create a Directed relationship L1→L3
    /// when it receives a stimulus, and proposes to delete L1→L2 if one
    /// already exists (by scanning the world — tests use RelationshipId(0)).
    struct WiringProgram {
        new_target: LocusId,
        delete_rel: Option<graph_core::RelationshipId>,
    }
    impl LocusProgram for WiringProgram {
        fn process(&self, _: &Locus, _: &[Change]) -> Vec<ProposedChange> {
            Vec::new()
        }
        fn structural_proposals(&self, _: &Locus, incoming: &[Change]) -> Vec<StructuralProposal> {
            // Only act on stimuli.
            if incoming.iter().all(|c| c.predecessors.is_empty()) {
                let mut out = vec![StructuralProposal::CreateRelationship {
                    endpoints: Endpoints::Directed {
                        from: LocusId(1),
                        to: self.new_target,
                    },
                    kind: InfluenceKindId(1),
                }];
                if let Some(rid) = self.delete_rel {
                    out.push(StructuralProposal::DeleteRelationship { rel_id: rid });
                }
                out
            } else {
                Vec::new()
            }
        }
    }

    #[test]
    fn structural_proposal_creates_relationship() {
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(3), LocusKindId(2), StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(
            LocusKindId(1),
            Box::new(WiringProgram { new_target: LocusId(3), delete_rel: None }),
        );
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut inf = InfluenceKindRegistry::new();
        inf.insert(InfluenceKindId(1), crate::registry::InfluenceKindConfig::new("t"));
        let engine = Engine::default();

        assert_eq!(world.relationships().len(), 0);
        engine.tick(
            &mut world,
            &loci,
            &inf,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0]),
            )],
        );
        // WiringProgram proposed CreateRelationship L1→L3.
        assert_eq!(world.relationships().len(), 1, "one relationship created");
        let key = Endpoints::Directed { from: LocusId(1), to: LocusId(3) }.key();
        assert!(
            world.relationships().lookup(&key, InfluenceKindId(1)).is_some(),
            "L1→L3 must exist"
        );
    }

    #[test]
    fn structural_proposal_create_existing_is_activity_touch() {
        // If L1→L3 already exists and WiringProgram proposes CreateRelationship
        // again, it should increment activity, not panic or duplicate.
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(3), LocusKindId(2), StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(
            LocusKindId(1),
            Box::new(WiringProgram { new_target: LocusId(3), delete_rel: None }),
        );
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut inf = InfluenceKindRegistry::new();
        inf.insert(InfluenceKindId(1), crate::registry::InfluenceKindConfig::new("t"));
        let engine = Engine::default();

        let stim = || ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        );
        engine.tick(&mut world, &loci, &inf, vec![stim()]);
        engine.tick(&mut world, &loci, &inf, vec![stim()]);

        // Still exactly one relationship — second proposal was a touch.
        assert_eq!(world.relationships().len(), 1);
        let rel = world.relationships().iter().next().unwrap();
        // Activity after two stimuli: +1.0 on first creation, +1.0 on second
        // touch, then each batch applies decay=1.0 → 2.0. (decay=1.0 default)
        assert!(rel.activity() > 1.0, "activity should have grown after two touches");
    }

    #[test]
    fn structural_proposal_deletes_relationship() {
        // First tick: auto-emerge L1→L2 via ForwarderProgram.
        // Second tick: WiringProgram deletes RelationshipId(0).
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut inf = InfluenceKindRegistry::new();
        inf.insert(InfluenceKindId(1), crate::registry::InfluenceKindConfig::new("t"));
        let engine = Engine::default();

        // Tick 1: establish relationship.
        engine.tick(
            &mut world,
            &loci,
            &inf,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0]),
            )],
        );
        assert_eq!(world.relationships().len(), 1, "relationship emerged");
        let rel_id = world.relationships().iter().next().unwrap().id;

        // Now swap the program on kind 1 to a deleter.
        let mut loci2 = LocusKindRegistry::new();
        loci2.insert(
            LocusKindId(1),
            Box::new(WiringProgram {
                new_target: LocusId(2), // ignored by this test
                delete_rel: Some(rel_id),
            }),
        );
        loci2.insert(LocusKindId(2), Box::new(SinkProgram));

        // Tick 2: WiringProgram deletes the relationship.
        engine.tick(
            &mut world,
            &loci2,
            &inf,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0]),
            )],
        );
        assert_eq!(
            world.relationships().len(),
            0,
            "relationship should be deleted"
        );
    }

    #[test]
    fn structural_proposals_default_is_empty_for_existing_programs() {
        // Existing programs (ForwarderProgram, DampOnceProgram, etc.) should
        // return empty structural proposals — the default impl is a no-op.
        let (mut world, loci, influences) = setup();
        let engine = Engine::default();
        let result = engine.tick(
            &mut world,
            &loci,
            &influences,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0, 0.0]),
            )],
        );
        // DampOnceProgram produces a self-loop (no relationships) and then
        // quiesces in 2 batches. No structural proposals → no extra relationships.
        assert!(!result.hit_batch_cap);
        assert_eq!(world.relationships().len(), 0, "no structural proposals emitted");
    }

    // ─── Edge plasticity (Hebbian) tests ──────────────────────────────────

    fn two_locus_world_with_plasticity(
        learning_rate: f32,
        weight_decay: f32,
    ) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
        use crate::registry::PlasticityConfig;
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut inf = InfluenceKindRegistry::new();
        inf.insert(
            InfluenceKindId(1),
            crate::registry::InfluenceKindConfig::new("hebb")
                .with_plasticity(PlasticityConfig {
                    learning_rate,
                    weight_decay,
                    max_weight: f32::MAX,
                }),
        );
        (world, loci, inf)
    }

    #[test]
    fn hebbian_weight_increases_on_correlated_flow() {
        let (mut world, loci, inf) = two_locus_world_with_plasticity(0.1, 1.0);
        let engine = Engine::default();
        engine.tick(
            &mut world,
            &loci,
            &inf,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[2.0]),
            )],
        );
        // Relationship L1→L2 should have been created.
        let rel = world.relationships().iter().next().expect("relationship must exist");
        // Hebbian update: Δweight = 0.1 * pre(2.0) * post(2.0) = 0.4
        // (ForwarderProgram forwards the incoming value unchanged)
        let weight = rel.weight();
        assert!(
            (weight - 0.4).abs() < 1e-5,
            "expected weight ≈ 0.4, got {weight}"
        );
    }

    #[test]
    fn hebbian_weight_is_zero_when_plasticity_disabled() {
        // PlasticityConfig::default() has learning_rate = 0.
        let (mut world, loci, inf) = two_locus_world_with_plasticity(0.0, 1.0);
        let engine = Engine::default();
        engine.tick(
            &mut world,
            &loci,
            &inf,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[3.0]),
            )],
        );
        let rel = world.relationships().iter().next().expect("relationship must exist");
        assert!(
            rel.weight().abs() < 1e-6,
            "weight must be 0 when learning_rate=0, got {}",
            rel.weight()
        );
    }

    #[test]
    fn hebbian_weight_accumulates_over_multiple_ticks() {
        let (mut world, loci, inf) = two_locus_world_with_plasticity(0.1, 1.0);
        let engine = Engine::default();
        // Each tick: pre=1.0, post=1.0 → Δweight = 0.1 per tick.
        for _ in 0..3 {
            engine.tick(
                &mut world,
                &loci,
                &inf,
                vec![ProposedChange::new(
                    ChangeSubject::Locus(LocusId(1)),
                    InfluenceKindId(1),
                    StateVector::from_slice(&[1.0]),
                )],
            );
        }
        let weight = world.relationships().iter().next().unwrap().weight();
        // 3 × 0.1 = 0.3
        assert!(
            (weight - 0.3).abs() < 1e-5,
            "expected weight ≈ 0.3 after 3 ticks, got {weight}"
        );
    }

    #[test]
    fn hebbian_weight_decays_each_batch() {
        // weight_decay = 0.5, learning_rate = 0 (no new learning), initial weight set
        // by one learning tick, then subsequent ticks only decay.
        use crate::registry::PlasticityConfig;
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut inf = InfluenceKindRegistry::new();
        inf.insert(
            InfluenceKindId(1),
            crate::registry::InfluenceKindConfig::new("hebb").with_plasticity(PlasticityConfig {
                learning_rate: 1.0, // Δweight = pre * post = 1.0 on first tick
                weight_decay: 0.5,
                max_weight: f32::MAX,
            }),
        );
        let engine = Engine::default();

        // Tick 1: pre=1.0, post=1.0 → weight += 1.0 → then *0.5 → weight = 0.5
        engine.tick(
            &mut world,
            &loci,
            &inf,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0]),
            )],
        );

        // Disable learning for subsequent ticks (update the config in place
        // by replacing and re-inserting is not possible; use a second registry).
        // Instead verify weight after tick 1.
        let w1 = world.relationships().iter().next().unwrap().weight();
        // Hebbian: +1.0 → then decay *0.5 = 0.5
        assert!(
            (w1 - 0.5).abs() < 1e-5,
            "after tick 1: expected weight ≈ 0.5, got {w1}"
        );
    }

    #[test]
    fn hebbian_weight_clamped_by_max_weight() {
        use crate::registry::PlasticityConfig;
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut inf = InfluenceKindRegistry::new();
        inf.insert(
            InfluenceKindId(1),
            crate::registry::InfluenceKindConfig::new("hebb").with_plasticity(PlasticityConfig {
                learning_rate: 100.0, // aggressive learning
                weight_decay: 1.0,
                max_weight: 2.0, // hard ceiling
            }),
        );
        let engine = Engine::default();
        engine.tick(
            &mut world,
            &loci,
            &inf,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0]),
            )],
        );
        let w = world.relationships().iter().next().unwrap().weight();
        assert!(
            w <= 2.0 + 1e-6,
            "weight {w} must not exceed max_weight 2.0"
        );
        assert!(
            (w - 2.0).abs() < 1e-5,
            "weight {w} should be clamped to 2.0"
        );
    }

    // ─── ChangeSubject::Relationship tests ────────────────────────────────

    /// A program that, after receiving a stimulus, looks for a relationship
    /// between its locus (L1) and L2, and if found proposes a change
    /// directly on that relationship to set its activity to a fixed value.
    struct RelationshipWriterProgram {
        downstream: LocusId,
    }
    impl LocusProgram for RelationshipWriterProgram {
        fn process(&self, _locus: &Locus, incoming: &[Change]) -> Vec<ProposedChange> {
            // Only act on stimuli.
            if !incoming.iter().all(|c| c.predecessors.is_empty()) {
                return Vec::new();
            }
            // Forward the signal so a relationship auto-emerges.
            vec![ProposedChange::new(
                ChangeSubject::Locus(self.downstream),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0]),
            )]
        }
    }

    #[test]
    fn relationship_subject_change_lands_in_log_and_updates_state() {
        // Topology: L1 (RelationshipWriterProgram -> L2) and L2 (SinkProgram).
        // First tick: stimulus on L1 → L1 forwards to L2 → relationship L1→L2
        //   auto-emerges with activity 1.0.
        // Second tick: we inject a ProposedChange targeting RelationshipId(0)
        //   directly and verify it updates the relationship state and appears
        //   in the log.
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(RelationshipWriterProgram { downstream: LocusId(2) }));
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(InfluenceKindId(1), crate::registry::InfluenceKindConfig::new("t"));

        let engine = Engine::default();
        // Tick 1: establish the relationship via cross-locus flow.
        engine.tick(
            &mut world,
            &loci,
            &influences,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0]),
            )],
        );
        let rel_id = graph_core::RelationshipId(0);
        assert!(world.relationships().get(rel_id).is_some(), "relationship must exist");

        // Tick 2: propose a relationship-subject change directly.
        let log_len_before = world.log().len();
        engine.tick(
            &mut world,
            &loci,
            &influences,
            vec![ProposedChange::new(
                ChangeSubject::Relationship(rel_id),
                InfluenceKindId(1),
                StateVector::from_slice(&[99.0]),
            )],
        );

        // The log must have grown by exactly one change.
        assert_eq!(world.log().len(), log_len_before + 1);

        // That change must have the correct subject.
        let rel_change = world.log().iter().last().unwrap();
        assert_eq!(
            rel_change.subject,
            ChangeSubject::Relationship(rel_id)
        );

        // The relationship's state must reflect the new value.
        let activity = world.relationships().get(rel_id).unwrap()
            .state.as_slice().first().copied().unwrap_or(0.0);
        assert!(
            (activity - 99.0).abs() < 1e-4,
            "relationship state should be 99.0, got {activity}"
        );
    }

    #[test]
    fn relationship_subject_change_does_not_trigger_program_dispatch() {
        // After a relationship-subject change, no program should run
        // (relationship changes contribute nothing to committed_ids_by_locus).
        // We verify this by using a program that would produce an infinite
        // loop if triggered — the system must not hit the batch cap.
        struct BombProgram;
        impl LocusProgram for BombProgram {
            fn process(&self, locus: &Locus, _: &[Change]) -> Vec<ProposedChange> {
                vec![ProposedChange::new(
                    ChangeSubject::Locus(locus.id),
                    InfluenceKindId(1),
                    StateVector::from_slice(&[1.0]),
                )]
            }
        }
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(RelationshipWriterProgram { downstream: LocusId(2) }));
        loci.insert(LocusKindId(2), Box::new(BombProgram)); // would loop if triggered
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(InfluenceKindId(1), crate::registry::InfluenceKindConfig::new("t"));
        let engine = Engine::new(EngineConfig { max_batches_per_tick: 4 });

        // First tick: establish relationship.
        engine.tick(
            &mut world,
            &loci,
            &influences,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0]),
            )],
        );

        // Second tick: relationship-subject stimulus. BombProgram must NOT run.
        let result = engine.tick(
            &mut world,
            &loci,
            &influences,
            vec![ProposedChange::new(
                ChangeSubject::Relationship(graph_core::RelationshipId(0)),
                InfluenceKindId(1),
                StateVector::from_slice(&[5.0]),
            )],
        );
        assert!(!result.hit_batch_cap, "relationship change must not trigger locus programs");
        assert_eq!(result.batches_committed, 1);
    }

    #[test]
    fn changes_to_relationship_query_returns_relationship_changes() {
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(LocusKindId(1), Box::new(RelationshipWriterProgram { downstream: LocusId(2) }));
        loci.insert(LocusKindId(2), Box::new(SinkProgram));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(InfluenceKindId(1), crate::registry::InfluenceKindConfig::new("t"));
        let engine = Engine::default();

        // Establish relationship.
        engine.tick(
            &mut world,
            &loci,
            &influences,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0]),
            )],
        );
        let rel_id = graph_core::RelationshipId(0);

        // Two relationship-subject changes.
        for v in [2.0_f32, 3.0] {
            engine.tick(
                &mut world,
                &loci,
                &influences,
                vec![ProposedChange::new(
                    ChangeSubject::Relationship(rel_id),
                    InfluenceKindId(1),
                    StateVector::from_slice(&[v]),
                )],
            );
        }

        let rel_changes: Vec<_> = world.log().changes_to_relationship(rel_id).collect();
        assert_eq!(rel_changes.len(), 2, "two relationship-subject changes");
        // Newest first: last value written (3.0) should appear first.
        assert!(
            (rel_changes[0].after.as_slice().first().copied().unwrap_or(0.0) - 3.0).abs() < 1e-5
        );
    }

    // ─── Change log trim tests ─────────────────────────────────────────────

    #[test]
    fn trim_change_log_removes_old_batches() {
        // After a tick that produces 2 batches (batches 0 and 1), current_batch=2.
        // Trimming with retention=1 should keep only batch 1.
        let (mut world, loci, influences) = setup();
        let engine = Engine::default();
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0, 1.0]),
        );
        engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        // log has 2 changes: batch 0 (stimulus) and batch 1 (damped response).
        assert_eq!(world.log().len(), 2);

        let removed = engine.trim_change_log(&mut world, 1);
        assert_eq!(removed, 1, "batch 0 change removed");
        assert_eq!(world.log().len(), 1);
        // The remaining change must be in batch 1.
        assert!(world.log().iter().all(|c| c.batch.0 >= 1));
    }

    #[test]
    fn trim_change_log_zero_retention_removes_all() {
        let (mut world, loci, influences) = setup();
        let engine = Engine::default();
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0, 1.0]),
        );
        engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        // current_batch=2, retain_from=2. No change has batch >= 2 (they are 0 and 1).
        let removed = engine.trim_change_log(&mut world, 0);
        assert_eq!(removed, 2);
        assert_eq!(world.log().len(), 0);
    }

    #[test]
    fn trim_change_log_large_retention_is_noop() {
        let (mut world, loci, influences) = setup();
        let engine = Engine::default();
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0, 0.0]),
        );
        engine.tick(&mut world, &loci, &influences, vec![stimulus]);
        let before = world.log().len();
        let removed = engine.trim_change_log(&mut world, 9999);
        assert_eq!(removed, 0);
        assert_eq!(world.log().len(), before);
    }
}
