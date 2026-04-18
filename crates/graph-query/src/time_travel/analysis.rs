mod projection;
mod target;

use graph_core::BatchId;
use graph_world::World;

use self::{
    projection::{approximate_entities, relationships_irrecoverable, relationships_to_remove},
    target::{effective_target_batch, empty_result},
};
use super::TimeTravelResult;

pub(super) fn build_time_travel_result(world: &World, target_batch: BatchId) -> TimeTravelResult {
    let current_batch = world.current_batch();
    let (effective_target, trimmed_at) = effective_target_batch(world, target_batch);

    if effective_target >= current_batch {
        return empty_result(target_batch, trimmed_at);
    }

    let forward_diff = world.diff_between(effective_target, current_batch);
    TimeTravelResult {
        target_batch,
        relationships_to_remove: relationships_to_remove(&forward_diff),
        relationships_irrecoverable: relationships_irrecoverable(&forward_diff),
        entities_approximate: approximate_entities(world, effective_target, current_batch),
        forward_diff,
        trimmed_at,
    }
}
