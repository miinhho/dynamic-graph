use graph_core::Relationship;

use super::ActivityStats;

pub(super) fn activity_stats_for(rels: Vec<&Relationship>) -> Option<ActivityStats> {
    let count = rels.len();
    if count == 0 {
        return None;
    }
    let activities: Vec<f32> = rels
        .iter()
        .map(|relationship| relationship.activity())
        .collect();
    Some(strongest_activity_stats(activities, count))
}

pub(super) fn strongest_activity_stats(activities: Vec<f32>, count: usize) -> ActivityStats {
    let sum: f32 = activities.iter().sum();
    let min = activities.iter().copied().fold(f32::INFINITY, f32::min);
    let max = activities.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    ActivityStats {
        count,
        sum,
        mean: sum / count as f32,
        min,
        max,
    }
}
