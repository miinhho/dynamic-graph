use std::collections::HashSet;

use graph_core::{Change, LocusId};

use super::types::PlasticityObservation;

pub(super) fn score_predictions<'a, I: IntoIterator<Item = &'a Change>>(
    predictions: &[(LocusId, LocusId)],
    observed: I,
    observed_pairs: &HashSet<(LocusId, LocusId)>,
    fallback_window_batches: u64,
    recall_weight: f32,
) -> PlasticityObservation {
    let hits = prediction_hits(predictions, observed_pairs);
    let window_batches = observed_window_batches(observed, fallback_window_batches);
    PlasticityObservation::from_hits(
        hits,
        predictions.len(),
        observed_pairs.len(),
        window_batches,
        recall_weight,
    )
}

fn prediction_hits(
    predictions: &[(LocusId, LocusId)],
    observed_pairs: &HashSet<(LocusId, LocusId)>,
) -> usize {
    predictions
        .iter()
        .filter(|pair| observed_pairs.contains(pair))
        .count()
}

fn observed_window_batches<'a, I: IntoIterator<Item = &'a Change>>(
    observed: I,
    fallback_window_batches: u64,
) -> u64 {
    let mut min_batch = None::<u64>;
    let mut max_batch = None::<u64>;
    for change in observed {
        let batch = change.batch.0;
        min_batch = Some(min_batch.map_or(batch, |current| current.min(batch)));
        max_batch = Some(max_batch.map_or(batch, |current| current.max(batch)));
    }
    match (min_batch, max_batch) {
        (Some(start), Some(end)) => end.saturating_sub(start) + 1,
        _ => fallback_window_batches,
    }
}
