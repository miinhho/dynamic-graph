use graph_core::{BatchId, EntityId, LayerTransition, LifecycleCause};
use graph_world::World;

pub fn entity_layers_in_range(
    world: &World,
    entity_id: EntityId,
    from: BatchId,
    to: BatchId,
) -> Vec<(BatchId, LayerTransition, LifecycleCause)> {
    let Some(entity) = world.entities().get(entity_id) else {
        return Vec::new();
    };

    entity
        .layers
        .iter()
        .filter(|layer| layer.batch >= from && layer.batch < to)
        .map(|layer| (layer.batch, layer.transition.clone(), layer.cause.clone()))
        .collect()
}
