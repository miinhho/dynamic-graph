use graph_core::{BatchId, ChangeId, EntityId, LifecycleCause, RelationshipId};
use graph_world::World;

pub fn entity_transition_cause(
    world: &World,
    entity_id: EntityId,
    at_batch: BatchId,
) -> Option<LifecycleCause> {
    let layer = world.entities().layer_at_batch(entity_id, at_batch)?;
    Some(layer.cause.clone())
}

pub fn cause_seed_changes(
    world: &World,
    cause: &LifecycleCause,
    before_batch: BatchId,
) -> Vec<ChangeId> {
    relationship_ids(cause)
        .iter()
        .filter_map(|&rel_id| latest_relationship_change(world, rel_id, before_batch))
        .collect()
}

fn relationship_ids(cause: &LifecycleCause) -> &[RelationshipId] {
    match cause {
        LifecycleCause::RelationshipCluster { key_relationships } => key_relationships,
        LifecycleCause::RelationshipDecay {
            decayed_relationships,
        } => decayed_relationships,
        LifecycleCause::ComponentSplit { weak_bridges } => weak_bridges,
        _ => &[],
    }
}

fn latest_relationship_change(
    world: &World,
    rel_id: RelationshipId,
    before_batch: BatchId,
) -> Option<ChangeId> {
    world
        .log()
        .changes_to_relationship(rel_id)
        .find(|change| change.batch < before_batch)
        .map(|change| change.id)
}
