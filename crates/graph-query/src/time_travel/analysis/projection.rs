use graph_core::{BatchId, EntityId, RelationshipId};
use graph_world::{World, WorldDiff};

pub(super) fn relationships_to_remove(forward_diff: &WorldDiff) -> Vec<RelationshipId> {
    forward_diff.relationships_created.clone()
}

pub(super) fn relationships_irrecoverable(forward_diff: &WorldDiff) -> Vec<RelationshipId> {
    forward_diff.relationships_pruned.clone()
}

pub(super) fn approximate_entities(
    world: &World,
    effective_target: BatchId,
    current_batch: BatchId,
) -> Vec<EntityId> {
    world
        .entities()
        .iter()
        .filter(|entity| {
            entity.layers.iter().any(|layer| {
                layer.batch >= effective_target
                    && layer.batch < current_batch
                    && !matches!(layer.compression, graph_core::CompressionLevel::Full)
            })
        })
        .map(|entity| entity.id)
        .collect()
}
