#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ObservationMetrics {
    pub precision_at_k: f32,
    pub recall: f32,
    pub loss: f32,
}

pub(super) fn observation_metrics(
    hits: usize,
    k_used: usize,
    observed_pair_count: usize,
    recall_weight: f32,
) -> ObservationMetrics {
    let precision_at_k = ratio(hits, k_used);
    let recall = ratio(hits, observed_pair_count);
    ObservationMetrics {
        precision_at_k,
        recall,
        loss: prediction_loss(precision_at_k, recall, recall_weight),
    }
}

fn ratio(numerator: usize, denominator: usize) -> f32 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f32 / denominator as f32
    }
}

fn prediction_loss(precision_at_k: f32, recall: f32, recall_weight: f32) -> f32 {
    (1.0 - precision_at_k) + recall_weight * (1.0 - recall)
}
