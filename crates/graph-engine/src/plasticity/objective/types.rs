use std::collections::HashSet;

use graph_core::{BatchId, LocusId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankedPair {
    pub pair: (LocusId, LocusId),
    pub strength: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PairPredictionRanking {
    pub entries: Vec<RankedPair>,
}

impl PairPredictionRanking {
    pub fn top_k_pairs(&self, k: usize) -> Vec<(LocusId, LocusId)> {
        self.entries
            .iter()
            .take(k)
            .map(|entry| entry.pair)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairObservationWindow {
    pub from_batch: Option<BatchId>,
    pub to_batch: Option<BatchId>,
    pub fallback_horizon_batches: u64,
}

impl PairObservationWindow {
    pub fn bounded(from_batch: BatchId, to_batch: BatchId, fallback_horizon_batches: u64) -> Self {
        Self {
            from_batch: Some(from_batch),
            to_batch: Some(to_batch),
            fallback_horizon_batches,
        }
    }

    pub fn horizon(fallback_horizon_batches: u64) -> Self {
        Self {
            from_batch: None,
            to_batch: None,
            fallback_horizon_batches,
        }
    }

    pub fn batch_count(self) -> u64 {
        match (self.from_batch, self.to_batch) {
            (Some(from_batch), Some(to_batch)) => to_batch.0.saturating_sub(from_batch.0) + 1,
            _ => self.fallback_horizon_batches,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairObservationTargets {
    pub pairs: HashSet<(LocusId, LocusId)>,
    pub window_batches: u64,
}

impl PairObservationTargets {
    pub fn from_event_log(window: PairObservationWindow, event_log: &[Vec<Vec<u64>>]) -> Self {
        let mut pairs = HashSet::new();
        for block in event_log {
            for event in block {
                for left in 0..event.len() {
                    for right in (left + 1)..event.len() {
                        let a = LocusId(event[left]);
                        let b = LocusId(event[right]);
                        pairs.insert(if a.0 < b.0 { (a, b) } else { (b, a) });
                    }
                }
            }
        }

        Self {
            pairs,
            window_batches: window.batch_count(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlasticityObservation {
    pub loss: f32,
    pub precision_at_k: f32,
    pub recall: f32,
    pub k_used: usize,
    pub window_batches: u64,
}

impl PlasticityObservation {
    pub fn from_hits(
        hits: usize,
        k_used: usize,
        observed_pair_count: usize,
        window_batches: u64,
        recall_weight: f32,
    ) -> Self {
        let precision_at_k = if k_used == 0 {
            0.0
        } else {
            hits as f32 / k_used as f32
        };
        let recall = if observed_pair_count == 0 {
            0.0
        } else {
            hits as f32 / observed_pair_count as f32
        };

        Self {
            loss: (1.0 - precision_at_k) + recall_weight * (1.0 - recall),
            precision_at_k,
            recall,
            k_used,
            window_batches,
        }
    }

    pub fn adaptation_confidence(self) -> f32 {
        let k_confidence = (self.k_used as f32 / 20.0).clamp(0.0, 1.0);
        let window_confidence = (self.window_batches as f32 / 4.0).clamp(0.0, 1.0);
        k_confidence * window_confidence
    }

    pub fn adaptation_signal(self) -> f32 {
        let precision_term = self.precision_at_k - 0.5;
        let recall_term = self.recall - 0.5;
        let loss_term = 0.5 - self.loss;
        (0.45 * precision_term + 0.35 * recall_term + 0.20 * loss_term).clamp(-0.5, 0.5) * 2.0
    }
}
