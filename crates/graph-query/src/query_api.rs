//! Serializable query API for graph-query.
//!
//! Provides a single [`Query`] enum that covers all query operations in this
//! crate, a matching [`QueryResult`] enum with owned return values, and an
//! [`execute`] function that dispatches to the underlying implementations.

mod builders;
mod causal_analysis;
mod causal_log;
mod centrality;
mod entity_counterfactual;
mod filtered;
mod helpers;
mod results;
mod state_profile;
mod structural;
mod temporal_metrics;
mod types;

use graph_world::World;

pub use self::builders::{FindEntitiesBuilder, FindLociBuilder, FindRelationshipsBuilder};
pub(super) use self::helpers::{coheres_to_results, rel_to_summary};
pub use self::types::{
    CohereResult, EntityDiffSummary, EntityPredicate, EntitySort, LocusPredicate, LocusSort,
    LocusSummary, Query, QueryResult, RelSort, RelationshipPredicate, RelationshipProfileResult,
    RelationshipSummary, TrendResult, WorldMetricsResult,
};

use self::causal_analysis::execute_causal_analysis;
use self::causal_log::execute_causal_log;
use self::centrality::execute_centrality;
use self::entity_counterfactual::execute_entity_and_counterfactual;
use self::filtered::execute_filtered_lookup;
use self::state_profile::execute_state_and_profile;
use self::structural::execute_structural;
use self::temporal_metrics::execute_temporal_and_metrics;

pub fn execute(world: &World, query: &Query) -> QueryResult {
    execute_structural(world, query)
        .or_else(|| execute_centrality(world, query))
        .or_else(|| execute_causal_log(world, query))
        .or_else(|| execute_filtered_lookup(world, query))
        .or_else(|| execute_state_and_profile(world, query))
        .or_else(|| execute_entity_and_counterfactual(world, query))
        .or_else(|| execute_causal_analysis(world, query))
        .or_else(|| execute_temporal_and_metrics(world, query))
        .expect("all query variants must be handled")
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, KindObservation, Locus, LocusId, LocusKindId,
        Relationship, RelationshipLineage, StateVector,
    };
    use graph_world::World;

    fn simple_world() -> World {
        let mut w = World::new();
        let rk = InfluenceKindId(1);
        for (id, kind, v) in [(0u64, 1u64, 0.9f32), (1, 1, 0.4), (2, 2, 0.7)] {
            w.insert_locus(Locus::new(
                LocusId(id),
                LocusKindId(kind),
                StateVector::from_slice(&[v]),
            ));
        }
        for (from, to, activity) in [(0u64, 1u64, 0.8f32), (1, 2, 0.3)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::directed(LocusId(from), LocusId(to)),
                state: StateVector::from_slice(&[activity, 0.5]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 3,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    #[test]
    fn path_between_finds_path() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::PathBetween {
                from: LocusId(0),
                to: LocusId(2),
            },
        );
        match result {
            QueryResult::Path(Some(path)) => {
                assert!(path.contains(&LocusId(0)));
                assert!(path.contains(&LocusId(2)));
            }
            _ => panic!("expected Some(path)"),
        }
    }

    #[test]
    fn reachable_from_returns_loci() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::ReachableFrom {
                start: LocusId(0),
                depth: 2,
            },
        );
        match result {
            QueryResult::Loci(ids) => {
                assert!(ids.contains(&LocusId(1)));
                assert!(ids.contains(&LocusId(2)));
            }
            _ => panic!("expected Loci"),
        }
    }

    #[test]
    fn connected_components_returns_components() {
        let w = simple_world();
        let result = execute(&w, &Query::ConnectedComponents);
        match result {
            QueryResult::Components(comps) => {
                assert_eq!(comps.len(), 1);
                assert_eq!(comps[0].len(), 3);
            }
            _ => panic!("expected Components"),
        }
    }

    #[test]
    fn find_loci_returns_summaries_with_state() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::FindLoci {
                predicates: vec![LocusPredicate::OfKind(LocusKindId(1))],
                sort_by: None,
                limit: None,
            },
        );
        match result {
            QueryResult::LocusSummaries(rows) => {
                assert_eq!(rows.len(), 2);
                for row in &rows {
                    assert!(!row.state.is_empty());
                }
            }
            _ => panic!("expected LocusSummaries"),
        }
    }

    #[test]
    fn find_loci_sort_state_desc() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::FindLoci {
                predicates: vec![],
                sort_by: Some(LocusSort::StateDesc(0)),
                limit: Some(2),
            },
        );
        match result {
            QueryResult::LocusSummaries(rows) => {
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].id, LocusId(0));
                assert_eq!(rows[1].id, LocusId(2));
            }
            _ => panic!("expected LocusSummaries"),
        }
    }

    #[test]
    fn find_relationships_returns_summaries() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::FindRelationships {
                predicates: vec![RelationshipPredicate::ActivityAbove(0.5)],
                sort_by: None,
                limit: None,
            },
        );
        match result {
            QueryResult::RelationshipSummaries(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].from, LocusId(0));
                assert_eq!(rows[0].to, LocusId(1));
                assert!(rows[0].activity > 0.5);
                assert!(rows[0].directed);
            }
            _ => panic!("expected RelationshipSummaries"),
        }
    }

    #[test]
    fn find_relationships_sort_activity_desc_with_limit() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::FindRelationships {
                predicates: vec![],
                sort_by: Some(RelSort::ActivityDesc),
                limit: Some(1),
            },
        );
        match result {
            QueryResult::RelationshipSummaries(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].from, LocusId(0));
                assert!((rows[0].activity - 0.8).abs() < 1e-5);
            }
            _ => panic!("expected RelationshipSummaries"),
        }
    }

    #[test]
    fn find_relationships_compound_predicate() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::FindRelationships {
                predicates: vec![
                    RelationshipPredicate::OfKind(InfluenceKindId(1)),
                    RelationshipPredicate::ActivityAbove(0.5),
                ],
                sort_by: Some(RelSort::ActivityDesc),
                limit: None,
            },
        );
        match result {
            QueryResult::RelationshipSummaries(rows) => assert_eq!(rows.len(), 1),
            _ => panic!(),
        }
    }

    #[test]
    fn locus_state_slot_returns_value() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::LocusStateSlot {
                locus: LocusId(0),
                slot: 0,
            },
        );
        match result {
            QueryResult::MaybeScore(Some(v)) => assert!((v - 0.9).abs() < 1e-5),
            _ => panic!("expected MaybeScore(Some)"),
        }
    }

    #[test]
    fn locus_state_slot_missing_returns_none() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::LocusStateSlot {
                locus: LocusId(99),
                slot: 0,
            },
        );
        assert_eq!(result, QueryResult::MaybeScore(None));
    }

    #[test]
    fn relationship_profile_includes_dominant_kind() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::RelationshipProfile {
                from: LocusId(0),
                to: LocusId(1),
            },
        );
        match result {
            QueryResult::RelationshipProfile(p) => {
                assert_eq!(p.relationship_ids.len(), 1);
                assert_eq!(p.dominant_kind, Some(InfluenceKindId(1)));
                assert_eq!(p.activity_by_kind.len(), 1);
                assert_eq!(p.activity_by_kind[0].0, InfluenceKindId(1));
            }
            _ => panic!("expected RelationshipProfile"),
        }
    }

    #[test]
    fn all_betweenness_with_limit() {
        let w = simple_world();
        let result = execute(&w, &Query::AllBetweenness { limit: Some(1) });
        match result {
            QueryResult::LocusScores(scores) => {
                assert_eq!(scores.len(), 1);
                assert!(scores[0].1 >= 0.0);
            }
            _ => panic!("expected LocusScores"),
        }
    }

    #[test]
    fn all_betweenness_no_limit_returns_all() {
        let w = simple_world();
        let result = execute(&w, &Query::AllBetweenness { limit: None });
        match result {
            QueryResult::LocusScores(scores) => {
                assert_eq!(scores.len(), 3);
                for w in scores.windows(2) {
                    assert!(w[0].1 >= w[1].1);
                }
            }
            _ => panic!("expected LocusScores"),
        }
    }

    #[test]
    fn pagerank_with_limit_returns_top_n() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::PageRank {
                damping: 0.85,
                iterations: 20,
                tolerance: 1e-4,
                limit: Some(2),
            },
        );
        match result {
            QueryResult::LocusScores(scores) => {
                assert_eq!(scores.len(), 2);
                assert!(scores[0].1 >= scores[1].1);
            }
            _ => panic!("expected LocusScores"),
        }
    }

    #[test]
    fn betweenness_for_middle_locus() {
        let w = simple_world();
        let result = execute(&w, &Query::BetweennessFor(LocusId(1)));
        match result {
            QueryResult::Score(v) => assert!(v >= 0.0),
            _ => panic!("expected Score"),
        }
    }

    #[test]
    fn has_cycle_false_for_dag() {
        let w = simple_world();
        let result = execute(&w, &Query::HasCycle);
        match result {
            QueryResult::Bool(v) => assert!(!v),
            _ => panic!("expected Bool"),
        }
    }

    #[test]
    fn world_metrics_returns_correct_counts() {
        let w = simple_world();
        let result = execute(&w, &Query::WorldMetrics);
        match result {
            QueryResult::WorldMetrics(m) => {
                assert_eq!(m.locus_count, 3);
                assert_eq!(m.relationship_count, 2);
            }
            _ => panic!("expected WorldMetrics"),
        }
    }

    #[test]
    fn builder_find_relationships_equals_enum() {
        let via_builder = Query::find_relationships()
            .of_kind(InfluenceKindId(1))
            .activity_above(0.5)
            .sort_by(RelSort::ActivityDesc)
            .limit(10)
            .build();
        let via_enum = Query::FindRelationships {
            predicates: vec![
                RelationshipPredicate::OfKind(InfluenceKindId(1)),
                RelationshipPredicate::ActivityAbove(0.5),
            ],
            sort_by: Some(RelSort::ActivityDesc),
            limit: Some(10),
        };
        assert_eq!(via_builder, via_enum);
    }

    #[test]
    fn builder_find_relationships_run_returns_summaries() {
        let w = simple_world();
        let rows = Query::find_relationships()
            .activity_above(0.5)
            .run(&w)
            .into_relationship_summaries()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].from, LocusId(0));
    }

    #[test]
    fn builder_find_loci_equals_enum() {
        let via_builder = Query::find_loci()
            .of_kind(LocusKindId(1))
            .state_above(0, 0.5)
            .sort_by(LocusSort::StateDesc(0))
            .limit(5)
            .build();
        let via_enum = Query::FindLoci {
            predicates: vec![
                LocusPredicate::OfKind(LocusKindId(1)),
                LocusPredicate::StateAbove { slot: 0, min: 0.5 },
            ],
            sort_by: Some(LocusSort::StateDesc(0)),
            limit: Some(5),
        };
        assert_eq!(via_builder, via_enum);
    }

    #[test]
    fn builder_find_loci_run_returns_summaries() {
        let w = simple_world();
        let summaries = Query::find_loci()
            .of_kind(LocusKindId(1))
            .run(&w)
            .into_locus_summaries()
            .unwrap();
        assert_eq!(summaries.len(), 2);
    }

    #[test]
    fn builder_find_entities_run_returns_entities() {
        let w = simple_world();
        let ids = Query::find_entities().run(&w).into_entities().unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn into_loci_extracts_correct_variant() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::ReachableFrom {
                start: LocusId(0),
                depth: 2,
            },
        );
        assert!(result.into_loci().is_some());
    }

    #[test]
    fn into_loci_returns_none_for_wrong_variant() {
        let w = simple_world();
        let result = execute(&w, &Query::HasCycle);
        assert!(result.into_loci().is_none());
    }

    #[test]
    fn into_bool_extracts_correct_variant() {
        let w = simple_world();
        let result = execute(&w, &Query::HasCycle);
        assert_eq!(result.into_bool(), Some(false));
    }

    #[test]
    fn into_path_extracts_correct_variant() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::PathBetween {
                from: LocusId(0),
                to: LocusId(2),
            },
        );
        let path = result.into_path().unwrap();
        assert!(path.is_some());
    }

    #[test]
    fn into_world_metrics_extracts_correct_variant() {
        let w = simple_world();
        let m = execute(&w, &Query::WorldMetrics)
            .into_world_metrics()
            .unwrap();
        assert_eq!(m.locus_count, 3);
    }

    fn world_with_stdp_weights() -> World {
        use graph_core::{Endpoints, StateVector};
        let mut w = World::new();
        w.add_relationship(
            Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            InfluenceKindId(1),
            StateVector::from_slice(&[0.0, 0.8]),
        );
        w.add_relationship(
            Endpoints::Directed {
                from: LocusId(1),
                to: LocusId(0),
            },
            InfluenceKindId(1),
            StateVector::from_slice(&[0.0, 0.2]),
        );
        w
    }

    #[test]
    fn query_causal_direction_forward() {
        let w = world_with_stdp_weights();
        let score = execute(
            &w,
            &Query::CausalDirection {
                from: LocusId(0),
                to: LocusId(1),
                kind: InfluenceKindId(1),
            },
        )
        .into_score()
        .unwrap();
        assert!(score > 0.5, "expected A→B direction, got {score}");
    }

    #[test]
    fn query_dominant_causes_returns_locus_scores() {
        let w = world_with_stdp_weights();
        let scores = execute(
            &w,
            &Query::DominantCauses {
                target: LocusId(1),
                kind: InfluenceKindId(1),
                n: 5,
            },
        )
        .into_scores()
        .unwrap();
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0].0, LocusId(0));
    }

    #[test]
    fn query_causal_in_out_strength() {
        let w = world_with_stdp_weights();
        let in_s = execute(
            &w,
            &Query::CausalInStrength {
                locus: LocusId(1),
                kind: InfluenceKindId(1),
            },
        )
        .into_score()
        .unwrap();
        let out_s = execute(
            &w,
            &Query::CausalOutStrength {
                locus: LocusId(0),
                kind: InfluenceKindId(1),
            },
        )
        .into_score()
        .unwrap();
        assert!((in_s - 0.8).abs() < 1e-5, "in_strength={in_s}");
        assert!((out_s - 0.8).abs() < 1e-5, "out_strength={out_s}");
    }

    #[test]
    fn query_feedback_pairs_detects_loop() {
        use graph_core::{Endpoints, StateVector};
        let mut w = World::new();
        w.add_relationship(
            Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            InfluenceKindId(1),
            StateVector::from_slice(&[0.0, 0.8]),
        );
        w.add_relationship(
            Endpoints::Directed {
                from: LocusId(1),
                to: LocusId(0),
            },
            InfluenceKindId(1),
            StateVector::from_slice(&[0.0, 0.7]),
        );
        let pairs = execute(
            &w,
            &Query::FeedbackPairs {
                kind: InfluenceKindId(1),
                min_weight: 0.1,
                min_balance: 0.5,
            },
        )
        .into_feedback_pairs()
        .unwrap();
        assert_eq!(pairs.len(), 1);
        let (_, _, balance) = pairs[0];
        assert!(balance >= 0.5 && balance <= 1.0);
    }

    #[test]
    fn explain_causal_direction_is_scan() {
        use crate::api::explain;
        let w = world_with_stdp_weights();
        let plan = explain(
            &w,
            &Query::CausalDirection {
                from: LocusId(0),
                to: LocusId(1),
                kind: InfluenceKindId(1),
            },
        );
        use crate::api::CostClass;
        assert!(plan.steps.iter().any(|s| s.cost_class == CostClass::Scan));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn query_round_trips_through_json() {
        let q = Query::FindLoci {
            predicates: vec![
                LocusPredicate::OfKind(LocusKindId(1)),
                LocusPredicate::StateAbove { slot: 0, min: 0.5 },
            ],
            sort_by: Some(LocusSort::StateDesc(0)),
            limit: Some(5),
        };
        let json = serde_json::to_string(&q).unwrap();
        let q2: Query = serde_json::from_str(&json).unwrap();
        assert_eq!(q, q2);
    }
}
