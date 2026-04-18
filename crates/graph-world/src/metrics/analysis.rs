use std::collections::VecDeque;

use graph_core::LocusId;
use rustc_hash::FxHashSet;

use super::{ACTIVITY_THRESHOLD, DegreeMetrics, RelationshipActivityMetrics, TOP_N};

pub(super) fn relationship_activity_metrics<'a>(
    relationships: impl Iterator<Item = &'a graph_core::Relationship>,
    relationship_count: usize,
) -> RelationshipActivityMetrics {
    let mut total_activity = 0.0;
    let mut max_activity = 0.0;
    let mut active_relationship_count = 0;
    let mut top_relationships_by_activity = Vec::with_capacity(relationship_count.min(TOP_N * 4));

    for relationship in relationships {
        let activity = relationship.activity();
        total_activity += activity;
        if activity > max_activity {
            max_activity = activity;
        }
        if activity > ACTIVITY_THRESHOLD {
            active_relationship_count += 1;
        }
        top_relationships_by_activity.push((relationship.id, activity));
    }

    top_relationships_by_activity.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    top_relationships_by_activity.truncate(TOP_N);

    RelationshipActivityMetrics {
        total_activity,
        max_activity,
        mean_activity: if relationship_count > 0 {
            total_activity / relationship_count as f32
        } else {
            0.0
        },
        active_relationship_count,
        top_relationships_by_activity,
    }
}

pub(super) fn degree_metrics(mut degree_pairs: Vec<(LocusId, usize)>) -> DegreeMetrics {
    let degree_count = degree_pairs.len();
    let mut max_degree = 0usize;
    let mut total_degree = 0usize;
    for &(_, degree) in &degree_pairs {
        if degree > max_degree {
            max_degree = degree;
        }
        total_degree += degree;
    }
    degree_pairs.sort_by(|left, right| right.1.cmp(&left.1));
    degree_pairs.truncate(TOP_N);

    DegreeMetrics {
        max_degree,
        mean_degree: if degree_count == 0 {
            0.0
        } else {
            total_degree as f64 / degree_count as f64
        },
        top_loci_by_degree: degree_pairs,
    }
}

pub(super) fn connected_components_stats(world: &crate::world::World) -> (usize, usize) {
    let all_loci: Vec<LocusId> = world.loci().iter().map(|locus| locus.id).collect();
    let mut visited: FxHashSet<LocusId> = FxHashSet::default();
    let mut component_count = 0usize;
    let mut largest = 0usize;

    for &seed in &all_loci {
        if visited.contains(&seed) {
            continue;
        }
        let mut size = 0usize;
        let mut queue: VecDeque<LocusId> = VecDeque::new();
        visited.insert(seed);
        queue.push_back(seed);
        while let Some(current) = queue.pop_front() {
            size += 1;
            for relationship in world.relationships().relationships_for_locus(current) {
                let neighbor = relationship.endpoints.other_than(current);
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }
        component_count += 1;
        if size > largest {
            largest = size;
        }
    }

    (component_count, largest)
}
