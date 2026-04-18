mod context;
mod ranking;
mod scoring;
mod types;

use std::collections::HashSet;

use graph_core::{BatchId, Change, InfluenceKindId, LocusId};
use graph_world::World;

use self::context::window_observation_context;
pub use self::types::{
    PairObservationTargets, PairObservationWindow, PairPredictionRanking, PlasticityObservation,
    RankedPair,
};

/// Domain declaration: pair-prediction objective for plasticity tuning.
///
/// Ranking is based on relationship `strength = activity + weight`, which is
/// the minimal signal aligned with both:
/// - recency of contact (`activity`)
/// - learned long-term reinforcement (`weight`)
#[derive(Debug, Clone, Copy)]
pub struct PairPredictionObjective {
    pub kind: InfluenceKindId,
    pub k: usize,
    pub horizon_batches: u64,
    pub recall_weight: f32,
}

impl PairPredictionObjective {
    /// Rank symmetric pairs by descending `Relationship::strength()`.
    pub fn rank(&self, world: &World) -> PairPredictionRanking {
        ranking::rank_pairs(world, self.kind)
    }

    /// Score a predicted pair list against observed pairs for a holdout window.
    pub fn score<'a, I: IntoIterator<Item = &'a Change>>(
        &self,
        predictions: &[(LocusId, LocusId)],
        observed: I,
        all_observed_pairs: &HashSet<(LocusId, LocusId)>,
    ) -> PlasticityObservation {
        scoring::score_predictions(
            predictions,
            observed,
            all_observed_pairs,
            self.horizon_batches,
            self.recall_weight,
        )
    }

    /// Collect observed symmetric pairs of this kind from a held-out event log.
    ///
    /// This helper is intentionally world-independent because the benchmark
    /// signal for pair prediction comes from future events, not from replaying
    /// relationship-subject changes.
    pub fn observed_pairs_from_events(
        &self,
        event_log: &[Vec<Vec<u64>>],
    ) -> HashSet<(LocusId, LocusId)> {
        PairObservationTargets::from_event_log(
            PairObservationWindow::horizon(self.horizon_batches),
            event_log,
        )
        .pairs
    }

    pub fn score_window(
        &self,
        world: &World,
        event_log: &[Vec<Vec<u64>>],
        from_batch: BatchId,
        to_batch: BatchId,
    ) -> PlasticityObservation {
        let context = window_observation_context(self, world, event_log, from_batch, to_batch);
        scoring::score_predictions(
            &context.predictions,
            context.observed_changes,
            &context.observed_targets.pairs,
            context.observed_targets.window_batches,
            self.recall_weight,
        )
    }
}
