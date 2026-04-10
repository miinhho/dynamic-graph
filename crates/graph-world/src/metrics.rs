//! Aggregate metrics snapshot for a `World`.
//!
//! `WorldMetrics` captures a point-in-time summary of the world's state:
//! counts, relationship activity statistics, and degree centrality
//! highlights. It is computed on demand via `World::metrics()` and is
//! cheap to clone.
//!
//! ## Degree centrality
//!
//! Degree here means the number of distinct relationships a locus
//! participates in (any endpoint, any kind). This uses the `by_locus`
//! reverse index added in `RelationshipStore`, so the pass is O(R)
//! where R is the total number of relationships.
//!
//! ## Activity statistics
//!
//! Relationship activity (slot 0 of `state`) is the primary signal the
//! engine writes. Mean and max activity give a quick sense of how "live"
//! the relationship graph is at this moment.

use std::collections::VecDeque;

use graph_core::{BatchId, LocusId, RelationshipId};
use rustc_hash::FxHashSet;

/// Point-in-time aggregate statistics for a `World`.
#[derive(Debug, Clone)]
pub struct WorldMetrics {
    // ── counts ────────────────────────────────────────────────────────────
    pub locus_count: usize,
    pub relationship_count: usize,
    pub entity_count: usize,
    pub active_entity_count: usize,
    /// Number of changes currently held in the change log
    /// (may be less than total committed if the log has been trimmed).
    pub change_log_len: usize,
    pub current_batch: BatchId,

    // ── relationship activity ─────────────────────────────────────────────
    /// Sum of all relationship activity scores at this snapshot.
    pub total_activity: f32,
    /// Maximum activity score across all relationships (0.0 if none).
    pub max_activity: f32,
    /// Mean activity score across all relationships (0.0 if none).
    pub mean_activity: f32,
    /// Number of relationships with activity strictly above
    /// `ACTIVITY_THRESHOLD`. A quick count of "live" relationships.
    pub active_relationship_count: usize,

    // ── degree centrality ─────────────────────────────────────────────────
    /// Maximum relationship degree across all loci (0 if no relationships).
    pub max_degree: usize,
    /// Mean relationship degree across loci that have at least one
    /// relationship (0.0 if none).
    pub mean_degree: f64,
    /// Up to `TOP_N` loci with the highest degree, sorted descending.
    pub top_loci_by_degree: Vec<(LocusId, usize)>,
    /// Up to `TOP_N` relationships with the highest activity, sorted
    /// descending.
    pub top_relationships_by_activity: Vec<(RelationshipId, f32)>,

    // ── connectivity ──────────────────────────────────────────────────────
    /// Number of weakly connected components in the relationship graph.
    /// Isolated loci (no relationships) each count as their own component.
    /// `1` means the graph is fully connected; equal to `locus_count`
    /// means no locus has any relationship.
    pub component_count: usize,
    /// Size of the largest connected component (in loci).
    pub largest_component_size: usize,
}

/// Number of entries returned in top-N lists.
pub const TOP_N: usize = 10;

/// Default threshold for counting "active" relationships in `WorldMetrics`.
/// Relationships with activity above this value are considered live.
pub const ACTIVITY_THRESHOLD: f32 = 0.1;

impl WorldMetrics {
    /// Compute metrics from the given world stores.
    ///
    /// This is an associated function rather than a method on `World` so
    /// that it can live in this module — `World::metrics()` delegates here.
    pub(crate) fn compute(world: &crate::world::World) -> Self {
        let loci = world.loci();
        let rels = world.relationships();
        let entities = world.entities();
        let log = world.log();

        // ── counts ────────────────────────────────────────────────────────
        let locus_count = loci.len();
        let relationship_count = rels.len();
        let entity_count = entities.len();
        let active_entity_count = entities.active_count();
        let change_log_len = log.len();
        let current_batch = world.current_batch();

        // ── relationship activity ─────────────────────────────────────────
        let mut total_activity = 0.0f32;
        let mut max_activity = 0.0f32;
        let mut active_relationship_count = 0usize;
        let mut activity_vec: Vec<(RelationshipId, f32)> = Vec::with_capacity(relationship_count.min(TOP_N * 4));
        for rel in rels.iter() {
            let a = rel.activity();
            total_activity += a;
            if a > max_activity {
                max_activity = a;
            }
            if a > ACTIVITY_THRESHOLD {
                active_relationship_count += 1;
            }
            activity_vec.push((rel.id, a));
        }
        let mean_activity = if relationship_count > 0 {
            total_activity / relationship_count as f32
        } else {
            0.0
        };
        activity_vec.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        activity_vec.truncate(TOP_N);
        let top_relationships_by_activity = activity_vec;

        // ── degree centrality ─────────────────────────────────────────────
        let mut degree_pairs: Vec<(LocusId, usize)> = rels.degree_iter().collect();
        let mut max_degree = 0usize;
        let mut total_degree = 0usize;
        for &(_, d) in &degree_pairs {
            if d > max_degree {
                max_degree = d;
            }
            total_degree += d;
        }
        let mean_degree = if degree_pairs.is_empty() {
            0.0
        } else {
            total_degree as f64 / degree_pairs.len() as f64
        };
        degree_pairs.sort_by(|a, b| b.1.cmp(&a.1));
        degree_pairs.truncate(TOP_N);
        let top_loci_by_degree = degree_pairs;

        // ── connectivity ──────────────────────────────────────────────────
        let (component_count, largest_component_size) = connected_components_stats(world);

        WorldMetrics {
            locus_count,
            relationship_count,
            entity_count,
            active_entity_count,
            change_log_len,
            current_batch,
            total_activity,
            max_activity,
            mean_activity,
            active_relationship_count,
            max_degree,
            mean_degree,
            top_loci_by_degree,
            top_relationships_by_activity,
            component_count,
            largest_component_size,
        }
    }
}

/// Compute `(component_count, largest_component_size)` using BFS over the
/// relationship graph. Isolated loci count as singleton components.
/// Called by `WorldMetrics::compute` — kept here to avoid a circular dep
/// on `graph-query`.
fn connected_components_stats(world: &crate::world::World) -> (usize, usize) {
    let loci = world.loci();
    let rels = world.relationships();
    let all_loci: Vec<LocusId> = loci.iter().map(|l| l.id).collect();
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
            for rel in rels.relationships_for_locus(current) {
                let neighbor = rel.endpoints.other_than(current);
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

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{Endpoints, InfluenceKindId, Locus, LocusId, LocusKindId, Relationship,
                     RelationshipLineage, StateVector};
    use crate::world::World;

    fn make_world_with_star(arms: u64) -> World {
        let kind = LocusKindId(1);
        let rel_kind = InfluenceKindId(1);
        let mut w = World::new();
        // hub = 0, arms = 1..=arms
        for i in 0..=arms {
            w.insert_locus(Locus::new(LocusId(i), kind, StateVector::zeros(1)));
        }
        for i in 1..=arms {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rel_kind,
                endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(i) },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: vec![rel_kind],
                },
                last_decayed_batch: 0,
            });
        }
        w
    }

    #[test]
    fn empty_world_metrics_are_zero() {
        let w = World::new();
        let m = w.metrics();
        assert_eq!(m.locus_count, 0);
        assert_eq!(m.relationship_count, 0);
        assert_eq!(m.max_degree, 0);
        assert_eq!(m.mean_activity, 0.0);
        assert!(m.top_loci_by_degree.is_empty());
    }

    #[test]
    fn star_hub_has_highest_degree() {
        let arms = 5;
        let w = make_world_with_star(arms);
        let m = w.metrics();

        assert_eq!(m.locus_count, arms as usize + 1);
        assert_eq!(m.relationship_count, arms as usize);
        assert_eq!(m.max_degree, arms as usize);

        // Hub (LocusId(0)) must be top of degree list.
        assert_eq!(m.top_loci_by_degree[0].0, LocusId(0));
        assert_eq!(m.top_loci_by_degree[0].1, arms as usize);
    }

    #[test]
    fn activity_stats_match_manual_sum() {
        let w = make_world_with_star(3);
        let m = w.metrics();
        // All 3 relationships have activity 1.0 (from state slot 0).
        assert!((m.total_activity - 3.0).abs() < 1e-5);
        assert!((m.mean_activity - 1.0).abs() < 1e-5);
        assert!((m.max_activity - 1.0).abs() < 1e-5);
    }

    #[test]
    fn component_count_in_empty_world_is_zero() {
        let w = World::new();
        let m = w.metrics();
        assert_eq!(m.component_count, 0);
        assert_eq!(m.largest_component_size, 0);
    }

    #[test]
    fn star_world_is_one_component() {
        let w = make_world_with_star(4);
        let m = w.metrics();
        assert_eq!(m.component_count, 1);
        assert_eq!(m.largest_component_size, 5); // hub + 4 arms
    }

    #[test]
    fn top_n_list_is_bounded_by_top_n_constant() {
        // Build a world with TOP_N + 5 spokes.
        let arms = (TOP_N + 5) as u64;
        let w = make_world_with_star(arms);
        let m = w.metrics();
        assert!(m.top_loci_by_degree.len() <= TOP_N);
        assert!(m.top_relationships_by_activity.len() <= TOP_N);
    }
}
