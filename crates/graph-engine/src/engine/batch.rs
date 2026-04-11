//! Batch loop internals: pending-change bookkeeping, per-locus dispatch
//! staging, and the two relationship-graph mutations that fire inside tick.

use graph_core::{
    BatchId, ChangeId, Endpoints, ProposedChange, Relationship, RelationshipLineage,
    StateVector, StructuralProposal,
};
use graph_world::World;

use crate::registry::InfluenceKindConfig;

/// A change queued for the next batch: the user/program-supplied proposal
/// plus any predecessor `ChangeId`s the engine derived from the previous
/// batch's commits.
pub(crate) struct PendingChange {
    pub(crate) proposed: ProposedChange,
    pub(crate) derived_predecessors: Vec<ChangeId>,
}

/// Per-locus dispatch input assembled after a batch commit. Holds
/// immutable references into the world valid for the duration of one
/// batch's program-dispatch phase.
pub(crate) struct DispatchInput<'a> {
    pub(crate) locus: &'a graph_core::Locus,
    pub(crate) program: &'a dyn graph_core::LocusProgram,
    pub(crate) inbox: Vec<&'a graph_core::Change>,
    pub(crate) derived: Vec<ChangeId>,
}

/// Output of one locus's program dispatch: proposed state changes,
/// structural topology proposals, and the derived predecessor ids to
/// attach to each follow-up change.
pub(crate) type DispatchResult = (Vec<ProposedChange>, Vec<StructuralProposal>, Vec<ChangeId>);

/// Recognize or update a directed relationship of `kind` going from
/// `from` to `to`, attributing the touch to `change_id`. Adds 1.0 to
/// the relationship's activity slot per touch.
///
/// `cfg` is the per-kind config, used to derive decay rates and to build
/// the initial `StateVector` (with extra slots) for newly created relationships.
///
/// Returns the `RelationshipId` (new or existing) so the caller can
/// record a plasticity observation.
pub(crate) fn auto_emerge_relationship(
    world: &mut World,
    from: graph_core::LocusId,
    to: graph_core::LocusId,
    kind: graph_core::InfluenceKindId,
    change_id: ChangeId,
    current_batch: u64,
    cfg: Option<&InfluenceKindConfig>,
) -> graph_core::RelationshipId {
    let activity_decay = cfg.map(|c| c.decay_per_batch).unwrap_or(1.0);
    let weight_decay = cfg.map(|c| c.plasticity.weight_decay).unwrap_or(1.0);

    let endpoints = Endpoints::Directed { from, to };
    let key = endpoints.key();
    let store = world.relationships_mut();
    if let Some(rel_id) = store.lookup(&key, kind) {
        let rel = store.get_mut(rel_id).expect("indexed id must exist");
        // Apply accumulated lazy decay before bumping activity so that the
        // increment lands on the correct decayed baseline.
        let delta = current_batch.saturating_sub(rel.last_decayed_batch);
        if delta > 0 {
            let slots = rel.state.as_mut_slice();
            if let Some(a) = slots.get_mut(Relationship::ACTIVITY_SLOT) {
                *a *= activity_decay.powi(delta as i32);
            }
            if let Some(w) = slots.get_mut(Relationship::WEIGHT_SLOT) {
                *w *= weight_decay.powi(delta as i32);
            }
            // Decay extra slots with their per-slot rates.
            if let Some(cfg) = cfg {
                for (i, slot_def) in cfg.extra_slots.iter().enumerate() {
                    if let Some(factor) = slot_def.decay {
                        let idx = 2 + i;
                        if let Some(v) = slots.get_mut(idx) {
                            *v *= factor.powi(delta as i32);
                        }
                    }
                }
            }
            rel.last_decayed_batch = current_batch;
        }
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
        let initial_state = cfg
            .map(|c| c.initial_relationship_state())
            .unwrap_or_else(|| StateVector::from_slice(&[1.0, 0.0]));
        store.insert(Relationship {
            id: new_id,
            kind,
            endpoints,
            state: initial_state,
            lineage: RelationshipLineage {
                created_by: Some(change_id),
                last_touched_by: Some(change_id),
                change_count: 1,
                kinds_observed: vec![kind],
            },
            last_decayed_batch: current_batch,
        });
        new_id
    }
}

/// Apply structural proposals collected during a batch's program-dispatch phase.
///
/// `CreateRelationship`: if the (endpoints, kind) pair already exists,
/// treat it as an activity touch. Otherwise mint and insert a new
/// relationship with `created_by: None` (no originating change). Extra
/// slots are initialised from the kind's `InfluenceKindConfig`.
///
/// `DeleteRelationship`: remove from the store and clean up any
/// subscriptions to the deleted relationship. The relationship's past
/// changes in the log remain intact.
///
/// `SubscribeToRelationship` / `UnsubscribeFromRelationship`: update the
/// world's subscription store so the subscriber locus receives inbox
/// entries when the relationship's state changes.
pub(crate) fn apply_structural_proposals(
    world: &mut World,
    proposals: Vec<StructuralProposal>,
    influence_registry: &crate::registry::InfluenceKindRegistry,
) {
    let current_batch = world.current_batch().0;
    let batch_id = BatchId(current_batch);
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
                    let initial_state = influence_registry
                        .get(kind)
                        .map(|c| c.initial_relationship_state())
                        .unwrap_or_else(|| StateVector::from_slice(&[1.0, 0.0]));
                    let new_id = store.mint_id();
                    store.insert(Relationship {
                        id: new_id,
                        kind,
                        endpoints,
                        state: initial_state,
                        lineage: RelationshipLineage {
                            created_by: None,
                            last_touched_by: None,
                            change_count: 1,
                            kinds_observed: vec![kind],
                        },
                        last_decayed_batch: current_batch,
                    });
                }
            }
            StructuralProposal::DeleteRelationship { rel_id } => {
                world.subscriptions_mut().remove_relationship(rel_id);
                world.relationships_mut().remove(rel_id);
            }
            StructuralProposal::SubscribeToRelationship { subscriber, rel_id } => {
                world.subscriptions_mut().subscribe_at(subscriber, rel_id, Some(batch_id));
            }
            StructuralProposal::UnsubscribeFromRelationship { subscriber, rel_id } => {
                world.subscriptions_mut().unsubscribe_at(subscriber, rel_id, Some(batch_id));
            }
        }
    }
}
