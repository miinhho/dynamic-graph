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
    WindowObservationContext {
        predictions: objective.rank(world).top_k_pairs(objective.k),
        observed_targets: PairObservationTargets::from_event_log(window, event_log),
        observed_changes: window_changes(world, from_batch, to_batch),
    }
}

fn window_changes(world: &World, from_batch: BatchId, to_batch: BatchId) -> Vec<&Change> {
    world
        .log()
        .iter()
        .filter(|change| change.batch.0 >= from_batch.0 && change.batch.0 <= to_batch.0)
        .collect()
}
