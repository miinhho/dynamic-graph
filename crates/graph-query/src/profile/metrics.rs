use graph_core::{BatchId, InfluenceKindId, InteractionEffect};
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{RelationshipBundle, adapter};

pub(super) fn activity_by_kind(bundle: &RelationshipBundle<'_>) -> Vec<(InfluenceKindId, f32)> {
    sort_kind_map(adapter::activity_map(bundle))
}

pub(super) fn net_activity_with_interactions<F>(
    bundle: &RelationshipBundle<'_>,
    interaction_fn: F,
) -> f32
where
    F: Fn(InfluenceKindId, InfluenceKindId) -> Option<InteractionEffect>,
{
    let by_kind = adapter::activity_map(bundle);
    if by_kind.is_empty() {
        return 0.0;
    }

    let kinds: Vec<InfluenceKindId> = by_kind.keys().copied().collect();
    let mut merged: FxHashSet<InfluenceKindId> = FxHashSet::default();
    let mut total = 0.0;

    for i in 0..kinds.len() {
        for j in (i + 1)..kinds.len() {
            let kind_a = kinds[i];
            let kind_b = kinds[j];
            if let Some(effect) = interaction_fn(kind_a, kind_b) {
                let combined = by_kind[&kind_a] + by_kind[&kind_b];
                total += match effect {
                    InteractionEffect::Synergistic { boost } => combined * boost,
                    InteractionEffect::Antagonistic { dampen } => combined * dampen,
                    InteractionEffect::Neutral => combined,
                };
                merged.insert(kind_a);
                merged.insert(kind_b);
            }
        }
    }

    for (kind, activity) in &by_kind {
        if !merged.contains(kind) {
            total += activity;
        }
    }
    total
}

pub(super) fn profile_similarity(
    left: &RelationshipBundle<'_>,
    right: &RelationshipBundle<'_>,
) -> f32 {
    let left_activity = adapter::activity_map(left);
    let right_activity = adapter::activity_map(right);
    let all_kinds = adapter::union_kinds(&left_activity, &right_activity);

    let left_vector: Vec<f32> = all_kinds
        .iter()
        .map(|kind| *left_activity.get(kind).unwrap_or(&0.0))
        .collect();
    let right_vector: Vec<f32> = all_kinds
        .iter()
        .map(|kind| *right_activity.get(kind).unwrap_or(&0.0))
        .collect();

    let left_state = graph_core::StateVector::from_slice(&left_vector);
    let right_state = graph_core::StateVector::from_slice(&right_vector);
    left_state.cosine_similarity(&right_state)
}

pub(super) fn profile_trend_similarity(
    left: &RelationshipBundle<'_>,
    right: &RelationshipBundle<'_>,
    world: &World,
    from_batch: BatchId,
    to_batch: BatchId,
) -> f32 {
    let left_slopes = adapter::trend_map(left, world, from_batch, to_batch);
    let right_slopes = adapter::trend_map(right, world, from_batch, to_batch);

    if left_slopes.is_empty() && right_slopes.is_empty() {
        return 0.0;
    }

    let all_kinds = adapter::union_kinds(&left_slopes, &right_slopes);
    let left_vector: Vec<f32> = all_kinds
        .iter()
        .map(|kind| *left_slopes.get(kind).unwrap_or(&0.0))
        .collect();
    let right_vector: Vec<f32> = all_kinds
        .iter()
        .map(|kind| *right_slopes.get(kind).unwrap_or(&0.0))
        .collect();

    let left_state = graph_core::StateVector::from_slice(&left_vector);
    let right_state = graph_core::StateVector::from_slice(&right_vector);
    left_state.cosine_similarity(&right_state)
}

fn sort_kind_map(by_kind: FxHashMap<InfluenceKindId, f32>) -> Vec<(InfluenceKindId, f32)> {
    let mut pairs: Vec<_> = by_kind.into_iter().collect();
    pairs.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pairs
}
