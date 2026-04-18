use graph_core::BatchId;
use graph_world::{World, WorldDiff};

use crate::time_travel::TimeTravelResult;

pub(super) fn effective_target_batch(
    world: &World,
    target_batch: BatchId,
) -> (BatchId, Option<BatchId>) {
    let log_start = world
        .log()
        .iter()
        .next()
        .map(|change| change.batch)
        .unwrap_or(BatchId(0));
    if target_batch < log_start {
        (log_start, Some(log_start))
    } else {
        (target_batch, None)
    }
}

pub(super) fn empty_result(target_batch: BatchId, trimmed_at: Option<BatchId>) -> TimeTravelResult {
    TimeTravelResult {
        target_batch,
        forward_diff: WorldDiff::default(),
        relationships_to_remove: Vec::new(),
        relationships_irrecoverable: Vec::new(),
        entities_approximate: Vec::new(),
        trimmed_at,
    }
}
