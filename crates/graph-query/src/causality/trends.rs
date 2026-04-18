use graph_core::{BatchId, Change, Relationship, RelationshipId};
use graph_world::World;

use super::temporal::changes_to_relationship_in_range;
use super::types::Trend;

pub fn relationship_volatility(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> f32 {
    let changes = changes_to_relationship_in_range(world, rel, from_batch, to_batch);
    let n = changes.len();
    if n < 2 {
        return 0.0;
    }
    let nf = n as f32;
    let activity = |change: &&Change| change.after.as_slice().first().copied().unwrap_or(0.0);
    let mean = changes.iter().map(activity).sum::<f32>() / nf;
    let variance = changes
        .iter()
        .map(|change| (activity(change) - mean).powi(2))
        .sum::<f32>()
        / nf;
    variance.sqrt()
}

pub fn relationship_activity_trend_with_threshold(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
    stable_threshold: f32,
) -> Option<Trend> {
    let changes = changes_to_relationship_in_range(world, rel, from_batch, to_batch);
    if changes.len() < 2 {
        return None;
    }

    let Some(slope) = ols_activity_slope(&changes) else {
        return Some(Trend::Stable);
    };

    Some(classify_trend(slope, stable_threshold))
}

pub fn relationship_activity_trend(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Option<Trend> {
    relationship_activity_trend_with_threshold(world, rel, from_batch, to_batch, 0.05)
}

pub fn relationship_weight_delta(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Option<f32> {
    let changes = changes_to_relationship_in_range(world, rel, from_batch, to_batch);
    if changes.len() < 2 {
        return None;
    }
    let newest = changes
        .first()?
        .after
        .as_slice()
        .get(Relationship::WEIGHT_SLOT)
        .copied()?;
    let oldest = changes
        .last()?
        .after
        .as_slice()
        .get(Relationship::WEIGHT_SLOT)
        .copied()?;
    Some(newest - oldest)
}

pub fn relationship_weight_trend_delta(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
    stable_threshold: f32,
) -> Option<Trend> {
    let delta = relationship_weight_delta(world, rel, from_batch, to_batch)?;
    Some(classify_trend(delta, stable_threshold))
}

pub fn relationship_weight_trend_with_threshold(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
    stable_threshold: f32,
) -> Option<Trend> {
    let changes = changes_to_relationship_in_range(world, rel, from_batch, to_batch);
    if changes.len() < 2 {
        return None;
    }

    let Some(slope) = ols_slot_slope(&changes, 1) else {
        return Some(Trend::Stable);
    };

    Some(classify_trend(slope, stable_threshold))
}

pub fn relationship_weight_trend(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Option<Trend> {
    relationship_weight_trend_with_threshold(world, rel, from_batch, to_batch, 0.05)
}

pub fn relationship_volatility_all(world: &World, rel: RelationshipId) -> f32 {
    relationship_volatility(world, rel, BatchId(0), world.current_batch())
}

pub(crate) fn ols_slot_slope(changes: &[&Change], slot_idx: usize) -> Option<f32> {
    let n = changes.len();
    if n < 2 {
        return None;
    }
    let nf = n as f32;
    let sum_x = nf * (nf - 1.0) / 2.0;
    let sum_x2 = nf * (nf - 1.0) * (2.0 * nf - 1.0) / 6.0;
    let (sum_y, sum_xy) =
        changes
            .iter()
            .enumerate()
            .fold((0.0f32, 0.0f32), |(sum_y, sum_xy), (i, change)| {
                let value = change
                    .after
                    .as_slice()
                    .get(slot_idx)
                    .copied()
                    .unwrap_or(0.0);
                (sum_y + value, sum_xy + i as f32 * value)
            });
    let denom = nf * sum_x2 - sum_x * sum_x;
    if denom.abs() < 1e-12 {
        return None;
    }
    Some((nf * sum_xy - sum_x * sum_y) / denom)
}

pub(crate) fn ols_activity_slope(changes: &[&Change]) -> Option<f32> {
    ols_slot_slope(changes, 0)
}

fn classify_trend(slope: f32, stable_threshold: f32) -> Trend {
    if slope > stable_threshold {
        Trend::Rising { slope }
    } else if slope < -stable_threshold {
        Trend::Falling { slope }
    } else {
        Trend::Stable
    }
}
