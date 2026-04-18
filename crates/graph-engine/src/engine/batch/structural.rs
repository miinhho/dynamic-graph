use graph_core::{
    BatchId, ChangeSubject, Endpoints, InfluenceKindId, KindObservation, Locus, LocusId,
    LocusKindId, Properties, ProposedChange, Relationship, RelationshipId, RelationshipLineage,
    StateVector, StructuralProposal,
};
use graph_world::World;

use crate::registry::InfluenceKindRegistry;

use super::PendingChange;

pub(crate) fn apply_structural_proposals(
    world: &mut World,
    proposals: Vec<StructuralProposal>,
    influence_registry: &InfluenceKindRegistry,
) -> Vec<PendingChange> {
    let current_batch = world.current_batch().0;
    let batch_id = BatchId(current_batch);
    let mut tombstones = Vec::new();

    for proposal in proposals {
        apply_structural_proposal(
            world,
            proposal,
            influence_registry,
            current_batch,
            batch_id,
            &mut tombstones,
        );
    }

    tombstones
}

fn apply_structural_proposal(
    world: &mut World,
    proposal: StructuralProposal,
    influence_registry: &InfluenceKindRegistry,
    current_batch: u64,
    batch_id: BatchId,
    tombstones: &mut Vec<PendingChange>,
) {
    match proposal {
        StructuralProposal::CreateRelationship {
            endpoints,
            kind,
            initial_activity,
            initial_state,
        } => apply_create_relationship_proposal(
            world,
            influence_registry,
            current_batch,
            endpoints,
            kind,
            initial_activity,
            initial_state,
        ),
        StructuralProposal::DeleteRelationship { rel_id } => {
            tombstones.extend(apply_delete_relationship_proposal(world, rel_id));
        }
        StructuralProposal::DeleteLocus { locus_id } => {
            tombstones.extend(apply_delete_locus_proposal(world, locus_id));
        }
        StructuralProposal::CreateLocus {
            locus_id,
            kind,
            state,
            name,
            properties,
        } => apply_create_locus_proposal(world, locus_id, kind, state, name, properties),
        subscription => apply_subscription_proposal(world, subscription, batch_id),
    }
}

fn apply_subscription_proposal(world: &mut World, proposal: StructuralProposal, batch_id: BatchId) {
    let Some(operation) = resolve_subscription_operation(proposal, batch_id) else {
        return;
    };
    apply_subscription_operation(world, operation);
}

enum SubscriptionOperation {
    SubscribeRelationship {
        subscriber: LocusId,
        rel_id: RelationshipId,
        batch_id: BatchId,
    },
    UnsubscribeRelationship {
        subscriber: LocusId,
        rel_id: RelationshipId,
        batch_id: BatchId,
    },
    SubscribeKind {
        subscriber: LocusId,
        kind: InfluenceKindId,
    },
    UnsubscribeKind {
        subscriber: LocusId,
        kind: InfluenceKindId,
    },
    SubscribeAnchorKind {
        subscriber: LocusId,
        anchor: LocusId,
        kind: InfluenceKindId,
    },
    UnsubscribeAnchorKind {
        subscriber: LocusId,
        anchor: LocusId,
        kind: InfluenceKindId,
    },
}

fn resolve_subscription_operation(
    proposal: StructuralProposal,
    batch_id: BatchId,
) -> Option<SubscriptionOperation> {
    match proposal {
        StructuralProposal::SubscribeToRelationship { subscriber, rel_id } => {
            Some(SubscriptionOperation::SubscribeRelationship {
                subscriber,
                rel_id,
                batch_id,
            })
        }
        StructuralProposal::UnsubscribeFromRelationship { subscriber, rel_id } => {
            Some(SubscriptionOperation::UnsubscribeRelationship {
                subscriber,
                rel_id,
                batch_id,
            })
        }
        StructuralProposal::SubscribeToKind { subscriber, kind } => {
            Some(SubscriptionOperation::SubscribeKind { subscriber, kind })
        }
        StructuralProposal::UnsubscribeFromKind { subscriber, kind } => {
            Some(SubscriptionOperation::UnsubscribeKind { subscriber, kind })
        }
        StructuralProposal::SubscribeToAnchorKind {
            subscriber,
            anchor,
            kind,
        } => Some(SubscriptionOperation::SubscribeAnchorKind {
            subscriber,
            anchor,
            kind,
        }),
        StructuralProposal::UnsubscribeFromAnchorKind {
            subscriber,
            anchor,
            kind,
        } => Some(SubscriptionOperation::UnsubscribeAnchorKind {
            subscriber,
            anchor,
            kind,
        }),
        StructuralProposal::CreateRelationship { .. }
        | StructuralProposal::DeleteRelationship { .. }
        | StructuralProposal::DeleteLocus { .. }
        | StructuralProposal::CreateLocus { .. } => None,
    }
}

fn apply_subscription_operation(world: &mut World, operation: SubscriptionOperation) {
    match operation {
        SubscriptionOperation::SubscribeRelationship {
            subscriber,
            rel_id,
            batch_id,
        } => {
            world
                .subscriptions_mut()
                .subscribe_at(subscriber, rel_id, Some(batch_id));
        }
        SubscriptionOperation::UnsubscribeRelationship {
            subscriber,
            rel_id,
            batch_id,
        } => {
            world
                .subscriptions_mut()
                .unsubscribe_at(subscriber, rel_id, Some(batch_id));
        }
        SubscriptionOperation::SubscribeKind { subscriber, kind } => {
            world
                .subscriptions_mut()
                .subscribe_to_kind(subscriber, kind);
        }
        SubscriptionOperation::UnsubscribeKind { subscriber, kind } => {
            world
                .subscriptions_mut()
                .unsubscribe_from_kind(subscriber, kind);
        }
        SubscriptionOperation::SubscribeAnchorKind {
            subscriber,
            anchor,
            kind,
        } => {
            world
                .subscriptions_mut()
                .subscribe_to_anchor_kind(subscriber, anchor, kind);
        }
        SubscriptionOperation::UnsubscribeAnchorKind {
            subscriber,
            anchor,
            kind,
        } => {
            world
                .subscriptions_mut()
                .unsubscribe_from_anchor_kind(subscriber, anchor, kind);
        }
    }
}

fn apply_create_relationship_proposal(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    current_batch: u64,
    endpoints: Endpoints,
    kind: InfluenceKindId,
    initial_activity: Option<f32>,
    initial_state: Option<StateVector>,
) {
    match resolve_relationship_creation(
        world,
        influence_registry,
        current_batch,
        endpoints,
        kind,
        initial_activity,
        initial_state,
    ) {
        RelationshipCreation::UpdateExisting {
            rel_id,
            activity_delta,
        } => apply_relationship_creation_update(world, rel_id, activity_delta),
        RelationshipCreation::CreateNew { relationship } => {
            world.relationships_mut().insert(relationship)
        }
    }
}

enum RelationshipCreation {
    UpdateExisting {
        rel_id: RelationshipId,
        activity_delta: f32,
    },
    CreateNew {
        relationship: Relationship,
    },
}

fn resolve_relationship_creation(
    world: &mut World,
    influence_registry: &InfluenceKindRegistry,
    current_batch: u64,
    endpoints: Endpoints,
    kind: InfluenceKindId,
    initial_activity: Option<f32>,
    initial_state: Option<StateVector>,
) -> RelationshipCreation {
    let key = endpoints.key();
    if let Some(rel_id) = world.relationships().lookup(&key, kind) {
        let activity_delta = influence_registry
            .get(kind)
            .map(|c| c.activity_contribution)
            .unwrap_or(1.0);
        return RelationshipCreation::UpdateExisting {
            rel_id,
            activity_delta,
        };
    }

    let state = resolve_relationship_initial_state(
        influence_registry,
        kind,
        initial_activity,
        initial_state,
    );
    let new_id = world.relationships_mut().mint_id();
    RelationshipCreation::CreateNew {
        relationship: Relationship {
            id: new_id,
            kind,
            endpoints,
            state,
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(kind)],
            },
            created_batch: BatchId(current_batch),
            last_decayed_batch: current_batch,
            metadata: None,
        },
    }
}

fn apply_relationship_creation_update(
    world: &mut World,
    rel_id: RelationshipId,
    activity_delta: f32,
) {
    let rel = world
        .relationships_mut()
        .get_mut(rel_id)
        .expect("indexed id must exist");
    if let Some(activity) = rel
        .state
        .as_mut_slice()
        .get_mut(Relationship::ACTIVITY_SLOT)
    {
        *activity += activity_delta;
    }
    rel.lineage.change_count += 1;
}

fn resolve_relationship_initial_state(
    influence_registry: &InfluenceKindRegistry,
    kind: InfluenceKindId,
    initial_activity: Option<f32>,
    initial_state: Option<StateVector>,
) -> StateVector {
    if let Some(state) = initial_state {
        return state;
    }
    let mut state = influence_registry.initial_state_for(kind);
    if let Some(act) = initial_activity
        && let Some(a) = state.as_mut_slice().get_mut(Relationship::ACTIVITY_SLOT)
    {
        *a = act;
    }
    state
}

fn apply_delete_relationship_proposal(
    world: &mut World,
    rel_id: RelationshipId,
) -> Vec<PendingChange> {
    let rel_kind = world.relationships().get(rel_id).map(|r| r.kind);
    let specific_subs = world.subscriptions_mut().remove_relationship(rel_id);
    world.relationships_mut().remove(rel_id);
    rel_kind
        .map(|kind| make_tombstones(world, rel_id, kind, specific_subs))
        .unwrap_or_default()
}

fn apply_delete_locus_proposal(world: &mut World, locus_id: LocusId) -> Vec<PendingChange> {
    let rel_ids: Vec<RelationshipId> = world
        .relationships()
        .relationships_for_locus(locus_id)
        .map(|r| r.id)
        .collect();
    let mut tombstones = Vec::new();
    for rel_id in rel_ids {
        let rel_kind = world.relationships().get(rel_id).map(|r| r.kind);
        let specific_subs = world.subscriptions_mut().remove_relationship(rel_id);
        world.relationships_mut().remove(rel_id);
        if let Some(kind) = rel_kind {
            let external: Vec<_> = specific_subs
                .into_iter()
                .filter(|&s| s != locus_id)
                .collect();
            tombstones.extend(make_tombstones(world, rel_id, kind, external));
        }
    }
    world.subscriptions_mut().remove_locus(locus_id);
    world.subscriptions_mut().remove_anchor_locus(locus_id);
    world.properties_mut().remove(locus_id);
    world.names_mut().remove(locus_id);
    world.loci_mut().remove(locus_id);
    tombstones
}

fn apply_create_locus_proposal(
    world: &mut World,
    locus_id: Option<LocusId>,
    kind: LocusKindId,
    state: StateVector,
    name: Option<String>,
    properties: Option<Properties>,
) {
    let id = locus_id.unwrap_or_else(|| world.loci().next_id());
    world.insert_locus(Locus::new(id, kind, state));
    if let Some(name) = name {
        world.names_mut().insert(name, id);
    }
    if let Some(properties) = properties {
        world.properties_mut().insert(id, properties);
    }
}

fn make_tombstones(
    world: &World,
    rel_id: RelationshipId,
    kind: InfluenceKindId,
    subscribers: Vec<LocusId>,
) -> Vec<PendingChange> {
    subscribers
        .into_iter()
        .filter_map(|sub| {
            let after = world.locus(sub)?.state.clone();
            let mut meta = Properties::new();
            meta.set("tombstone", true);
            meta.set("rel_id", rel_id.0 as f64);
            Some(PendingChange {
                proposed: ProposedChange {
                    subject: ChangeSubject::Locus(sub),
                    kind,
                    after,
                    extra_predecessors: Vec::new(),
                    wall_time: None,
                    metadata: Some(meta),
                    property_patch: None,
                    slot_patches: None,
                },
                derived_predecessors: Vec::new(),
            })
        })
        .collect()
}
