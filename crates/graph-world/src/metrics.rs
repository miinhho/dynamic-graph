//! Aggregate metrics snapshot for a `World`.

mod analysis;

use graph_core::{BatchId, LocusId, RelationshipId};

#[derive(Debug, Clone)]
pub struct WorldMetrics {
    pub locus_count: usize,
    pub relationship_count: usize,
    pub entity_count: usize,
    pub active_entity_count: usize,
    pub change_log_len: usize,
    pub current_batch: BatchId,
    pub total_activity: f32,
    pub max_activity: f32,
    pub mean_activity: f32,
    pub active_relationship_count: usize,
    pub max_degree: usize,
    pub mean_degree: f64,
    pub top_loci_by_degree: Vec<(LocusId, usize)>,
    pub top_relationships_by_activity: Vec<(RelationshipId, f32)>,
    pub component_count: usize,
    pub largest_component_size: usize,
}

pub const TOP_N: usize = 10;
pub const ACTIVITY_THRESHOLD: f32 = 0.1;

impl WorldMetrics {
    pub(crate) fn compute(world: &crate::world::World) -> Self {
        let loci = world.loci();
        let rels = world.relationships();
        let entities = world.entities();
        let log = world.log();
        let relationship_count = rels.len();
        let activity = analysis::relationship_activity_metrics(rels.iter(), relationship_count);
        let degree = analysis::degree_metrics(rels.degree_iter().collect());
        let (component_count, largest_component_size) = analysis::connected_components_stats(world);

        WorldMetrics {
            locus_count: loci.len(),
            relationship_count,
            entity_count: entities.len(),
            active_entity_count: entities.active_count(),
            change_log_len: log.len(),
            current_batch: world.current_batch(),
            total_activity: activity.total_activity,
            max_activity: activity.max_activity,
            mean_activity: activity.mean_activity,
            active_relationship_count: activity.active_relationship_count,
            max_degree: degree.max_degree,
            mean_degree: degree.mean_degree,
            top_loci_by_degree: degree.top_loci_by_degree,
            top_relationships_by_activity: activity.top_relationships_by_activity,
            component_count,
            largest_component_size,
        }
    }
}

struct RelationshipActivityMetrics {
    total_activity: f32,
    max_activity: f32,
    mean_activity: f32,
    active_relationship_count: usize,
    top_relationships_by_activity: Vec<(RelationshipId, f32)>,
}

struct DegreeMetrics {
    max_degree: usize,
    mean_degree: f64,
    top_loci_by_degree: Vec<(LocusId, usize)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use graph_core::{
        Endpoints, InfluenceKindId, KindObservation, Locus, LocusId, LocusKindId, Relationship,
        RelationshipLineage, StateVector,
    };

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
                endpoints: Endpoints::Directed {
                    from: LocusId(0),
                    to: LocusId(i),
                },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rel_kind)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
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
