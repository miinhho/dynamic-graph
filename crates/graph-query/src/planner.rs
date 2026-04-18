//! Query planner for the serializable Query API.
//!
//! ## What this does
//!
//! `explain()` returns a human-readable [`QueryPlan`] showing which steps
//! `execute()` will take and their cost class.  `execute()` and `explain()`
//! share the same planning logic so there is no drift between the two.
//!
//! ## The key optimisation
//!
//! For `FindRelationships`, if any of `From(id)`, `To(id)`, or `Touching(id)`
//! is present, the planner seeds the candidate set from the adjacency index
//! (`world.relationships_for_locus(id)`) rather than doing a full scan over
//! every relationship.  On a large graph this is O(degree) vs O(total) —
//! typically a 10-100× reduction in candidates before any filtering.
//!
//! Remaining predicates are applied in priority order:
//!
//! | Tier | Cost | Predicates |
//! |------|------|------------|
//! | `Index`     | O(degree) | `From`, `To`, `Touching` — used as seed |
//! | `Scan`      | O(candidates) | `OfKind`, value filters, range filters |
//! | `Traversal` | O(V+E)    | `ReachableFrom`, `DownstreamOf`, `UpstreamOf` |

mod explain;
mod predicates;
mod types;

pub use self::explain::explain;
pub(crate) use self::predicates::{SeedKind, plan_loci_predicates, plan_rel_predicates};
pub use self::types::{CostClass, PlanStep, QueryPlan};
use crate::query_api::Query;

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_api::{LocusPredicate, LocusSort, Query, RelSort, RelationshipPredicate};
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId, Relationship,
        RelationshipLineage, StateVector,
    };
    use graph_world::World;

    fn three_node_world() -> World {
        let mut w = World::new();
        let rk = InfluenceKindId(1);
        for id in 0u64..3 {
            w.insert_locus(Locus::new(
                graph_core::LocusId(id),
                LocusKindId(1),
                StateVector::from_slice(&[0.5]),
            ));
        }
        for (from, to) in [(0u64, 1u64), (1, 2), (0, 2)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::directed(graph_core::LocusId(from), graph_core::LocusId(to)),
                state: StateVector::from_slice(&[0.5, 0.5]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
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
    fn plan_rel_predicates_promotes_direct_lookup_when_from_to_kind_present() {
        let preds = vec![
            RelationshipPredicate::ActivityAbove(0.3),
            RelationshipPredicate::From(graph_core::LocusId(1)),
            RelationshipPredicate::To(graph_core::LocusId(2)),
            RelationshipPredicate::OfKind(InfluenceKindId(7)),
        ];
        let plan = plan_rel_predicates(&preds);
        assert!(
            matches!(plan.seed_locus, Some(SeedKind::DirectLookup { from, to, kind })
                if from.0 == 1 && to.0 == 2 && kind.0 == 7),
            "expected DirectLookup, got {:?}",
            plan.seed_locus.as_ref().map(|_| "other")
        );
        // Only ActivityAbove remains
        assert_eq!(plan.predicates_ordered.len(), 1);
        assert!(matches!(
            plan.predicates_ordered[0],
            RelationshipPredicate::ActivityAbove(_)
        ));
    }

    #[test]
    fn plan_rel_predicates_promotes_between_when_from_to_no_kind() {
        let preds = vec![
            RelationshipPredicate::From(graph_core::LocusId(3)),
            RelationshipPredicate::To(graph_core::LocusId(4)),
            RelationshipPredicate::ActivityAbove(0.5),
        ];
        let plan = plan_rel_predicates(&preds);
        assert!(
            matches!(plan.seed_locus, Some(SeedKind::Between { a, b })
                if a.0 == 3 && b.0 == 4),
            "expected Between"
        );
        // ActivityAbove remains; no OfKind was present
        assert_eq!(plan.predicates_ordered.len(), 1);
        assert!(matches!(
            plan.predicates_ordered[0],
            RelationshipPredicate::ActivityAbove(_)
        ));
    }

    #[test]
    fn explain_direct_lookup_reports_index_cost_and_exact_output() {
        let w = three_node_world();
        // three_node_world has edges (0→1), (1→2), (0→2) all of kind 1.
        let q = Query::FindRelationships {
            predicates: vec![
                RelationshipPredicate::From(graph_core::LocusId(0)),
                RelationshipPredicate::To(graph_core::LocusId(1)),
                RelationshipPredicate::OfKind(InfluenceKindId(1)),
            ],
            sort_by: None,
            limit: None,
        };
        let plan = explain(&w, &q);
        assert_eq!(plan.steps[0].cost_class, CostClass::Index);
        // DirectLookup does the actual lookup → exactly 1 result.
        assert_eq!(plan.steps[0].estimated_output, 1);
        assert_eq!(plan.estimated_output, 1);
    }

    #[test]
    fn explain_direct_lookup_reports_zero_for_nonexistent_edge() {
        let w = three_node_world();
        // No edge 1→0 in three_node_world.
        let q = Query::FindRelationships {
            predicates: vec![
                RelationshipPredicate::From(graph_core::LocusId(1)),
                RelationshipPredicate::To(graph_core::LocusId(0)),
                RelationshipPredicate::OfKind(InfluenceKindId(1)),
            ],
            sort_by: None,
            limit: None,
        };
        let plan = explain(&w, &q);
        assert_eq!(plan.steps[0].estimated_output, 0);
    }

    #[test]
    fn explain_full_scan_when_no_index_pred() {
        let w = three_node_world();
        let q = Query::FindRelationships {
            predicates: vec![RelationshipPredicate::ActivityAbove(0.3)],
            sort_by: None,
            limit: None,
        };
        let plan = explain(&w, &q);
        assert_eq!(plan.estimated_candidates_initial, 3);
        assert_eq!(plan.steps[0].cost_class, CostClass::Scan); // full scan
    }

    #[test]
    fn explain_index_seed_when_from_pred_present() {
        let w = three_node_world();
        let q = Query::FindRelationships {
            predicates: vec![
                RelationshipPredicate::From(graph_core::LocusId(0)),
                RelationshipPredicate::ActivityAbove(0.3),
            ],
            sort_by: Some(RelSort::ActivityDesc),
            limit: Some(1),
        };
        let plan = explain(&w, &q);
        assert_eq!(plan.steps[0].cost_class, CostClass::Index);
        // LocusId(0) has 2 outgoing edges
        assert_eq!(plan.steps[0].estimated_output, 2);
        // final output capped by limit
        assert_eq!(plan.estimated_output, 1);
    }

    #[test]
    fn plan_rel_predicates_promotes_from_to_seed() {
        let preds = vec![
            RelationshipPredicate::ActivityAbove(0.5),
            RelationshipPredicate::From(graph_core::LocusId(1)),
            RelationshipPredicate::OfKind(InfluenceKindId(1)),
        ];
        let plan = plan_rel_predicates(&preds);
        // From(1) promoted to seed
        assert!(matches!(plan.seed_locus, Some(SeedKind::From(id)) if id.0 == 1));
        // remaining predicates: OfKind before ActivityAbove
        assert_eq!(plan.predicates_ordered.len(), 2);
        assert!(matches!(
            plan.predicates_ordered[0],
            RelationshipPredicate::OfKind(_)
        ));
        assert!(matches!(
            plan.predicates_ordered[1],
            RelationshipPredicate::ActivityAbove(_)
        ));
    }

    #[test]
    fn plan_loci_predicates_traversal_last() {
        use graph_core::LocusId;
        let preds = vec![
            LocusPredicate::ReachableFrom {
                start: LocusId(0),
                depth: 2,
            },
            LocusPredicate::OfKind(LocusKindId(1)),
            LocusPredicate::StateAbove { slot: 0, min: 0.3 },
        ];
        let ordered = plan_loci_predicates(&preds);
        // OfKind first (10), StateAbove second (20), ReachableFrom last (90)
        assert!(matches!(ordered[0], LocusPredicate::OfKind(_)));
        assert!(matches!(ordered[1], LocusPredicate::StateAbove { .. }));
        assert!(matches!(ordered[2], LocusPredicate::ReachableFrom { .. }));
    }

    #[test]
    fn explain_find_loci_structure() {
        let w = three_node_world();
        let q = Query::FindLoci {
            predicates: vec![LocusPredicate::OfKind(LocusKindId(1))],
            sort_by: Some(LocusSort::StateDesc(0)),
            limit: Some(2),
        };
        let plan = explain(&w, &q);
        assert_eq!(plan.estimated_candidates_initial, 3);
        // should have: scan step + filter step + sort step + limit step
        assert!(plan.steps.len() >= 3);
        // estimated_output is a heuristic — just verify it's capped by limit
        assert!(plan.estimated_output <= 2);
    }

    #[test]
    fn no_index_pred_when_only_of_kind() {
        let preds = vec![RelationshipPredicate::OfKind(InfluenceKindId(1))];
        let plan = plan_rel_predicates(&preds);
        assert!(plan.seed_locus.is_none());
        assert_eq!(plan.predicates_ordered.len(), 1);
    }
}
