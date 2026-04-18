use graph_core::{BatchId, Change, LocusId};
use graph_world::World;

use super::{PairObservationTargets, PairObservationWindow, PairPredictionObjective};

pub(super) struct WindowObservationContext<'a> {
    pub predictions: Vec<(LocusId, LocusId)>,
    pub observed_targets: PairObservationTargets,
    pub observed_changes: Vec<&'a Change>,
}

pub(super) fn window_observation_context<'a>(
    objective: &PairPredictionObjective,
    world: &'a World,
    event_log: &[Vec<Vec<u64>>],
    from_batch: BatchId,
    to_batch: BatchId,
) -> WindowObservationContext<'a> {
    let window = PairObservationWindow::bounded(from_batch, to_batch, objective.horizon_batches);
    let bounded_events = event_log_window(event_log, from_batch, to_batch);
    WindowObservationContext {
        predictions: objective.rank(world).top_k_pairs(objective.k),
        observed_targets: PairObservationTargets::from_event_log(window, bounded_events),
        observed_changes: window_changes(world, from_batch, to_batch),
    }
}

fn window_changes(world: &World, from_batch: BatchId, to_batch: BatchId) -> Vec<&Change> {
    (from_batch.0..=to_batch.0)
        .flat_map(|batch| world.log().batch(BatchId(batch)))
        .collect()
}

fn event_log_window(
    event_log: &[Vec<Vec<u64>>],
    from_batch: BatchId,
    to_batch: BatchId,
) -> &[Vec<Vec<u64>>] {
    let window_len = to_batch.0.saturating_sub(from_batch.0).saturating_add(1) as usize;
    if event_log.len() <= window_len {
        return event_log;
    }
    let start = from_batch.0 as usize;
    let end = (to_batch.0 as usize).saturating_add(1);
    event_log.get(start..end).unwrap_or(&[])
}
