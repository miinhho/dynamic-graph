use graph_world::World;

use crate::planner::{CostClass, PlanStep, QueryPlan};
use crate::query_api::Query;

pub(super) fn explain_non_find_query(world: &World, query: &Query) -> QueryPlan {
    match query {
        Query::CausalDirection { kind, .. } => relationship_scan_plan(
            world,
            format!(
                "causal_direction scan over relationships of kind {:?}",
                kind
            ),
            Some(1),
        ),
        Query::DominantCauses { kind, n, .. } => relationship_scan_plan(
            world,
            format!("dominant_causes scan for kind {:?}, top {}", kind, n),
            Some(*n),
        ),
        Query::DominantEffects { kind, n, .. } => relationship_scan_plan(
            world,
            format!("dominant_effects scan for kind {:?}, top {}", kind, n),
            Some(*n),
        ),
        Query::CausalInStrength { kind, .. } | Query::CausalOutStrength { kind, .. } => {
            relationship_scan_plan(
                world,
                format!("causal strength scan over relationships of kind {:?}", kind),
                Some(1),
            )
        }
        Query::FeedbackPairs { kind, .. } => relationship_scan_plan(
            world,
            format!("feedback_pairs scan for kind {:?} (two passes)", kind),
            None,
        ),
        Query::GrangerScore { kind, .. } => changelog_scan_plan(
            world,
            format!("granger_score ChangeLog scan for kind {:?}", kind),
            Some(1),
        ),
        Query::GrangerDominantCauses { kind, n, .. } => changelog_scan_plan(
            world,
            format!(
                "granger_dominant_causes ChangeLog scan for kind {:?}, top {}",
                kind, n
            ),
            Some(*n),
        ),
        Query::GrangerDominantEffects { kind, n, .. } => changelog_scan_plan(
            world,
            format!(
                "granger_dominant_effects ChangeLog scan for kind {:?}, top {}",
                kind, n
            ),
            Some(*n),
        ),
        Query::TimeTravel { .. } => single_scan_plan(
            world.log().len() + world.relationships().len() + world.entities().len(),
            "time_travel: O(changes + R + E·layers) — WorldDiff inversion",
            None,
        ),
        Query::CounterfactualReplay { remove_changes } => single_scan_plan(
            world.log().len() + world.relationships().len(),
            &format!(
                "counterfactual_replay: O(descendants of {} roots + R={})",
                remove_changes.len(),
                world.relationships().len()
            ),
            None,
        ),
        Query::EntityTransitionCause { .. } => single_scan_plan(
            world.entities().len(),
            "entity_transition_cause: O(entity layers)",
            None,
        ),
        Query::EntityUpstreamTransitions { .. } => single_scan_plan(
            world.log().len() + world.entities().len(),
            "entity_upstream_transitions: O(ChangeLog ancestors + entity scan)",
            None,
        ),
        Query::EntityLayersInRange { entity_id, .. } => single_scan_plan(
            world
                .entities()
                .get(*entity_id)
                .map_or(0, |e| e.layers.len()),
            "entity_layers_in_range: O(entity layer count)",
            None,
        ),
        Query::AllBetweenness { limit } => single_traversal_plan(
            world.relationships().len(),
            "Brandes betweenness centrality over all loci",
            *limit,
        ),
        Query::AllCloseness { limit } => single_traversal_plan(
            world.loci().len(),
            "Harmonic closeness centrality over all loci",
            *limit,
        ),
        Query::AllConstraints { limit } => single_traversal_plan(
            world.loci().len(),
            "Burt structural constraint over all loci",
            *limit,
        ),
        Query::PageRank { limit, .. } => {
            single_traversal_plan(world.loci().len(), "PageRank over all loci", *limit)
        }
        Query::Louvain | Query::LouvainWithResolution(_) => single_traversal_plan(
            world.relationships().len(),
            "Louvain community detection",
            None,
        ),
        _ => default_single_step_plan(query),
    }
}

fn relationship_scan_plan(world: &World, desc: String, limit: Option<usize>) -> QueryPlan {
    single_scan_plan(world.relationships().len(), &desc, limit)
}

fn changelog_scan_plan(world: &World, desc: String, limit: Option<usize>) -> QueryPlan {
    single_scan_plan(world.log().len(), &desc, limit)
}

fn single_scan_plan(initial: usize, desc: &str, limit: Option<usize>) -> QueryPlan {
    let est = limit.map_or(initial, |n| n.min(initial));
    QueryPlan {
        steps: vec![PlanStep {
            description: desc.to_string(),
            cost_class: CostClass::Scan,
            estimated_output: est,
        }],
        estimated_candidates_initial: initial,
        estimated_output: est,
    }
}

fn single_traversal_plan(initial: usize, desc: &str, limit: Option<usize>) -> QueryPlan {
    let est = limit.unwrap_or(initial);
    let mut steps = vec![
        PlanStep {
            description: desc.to_string(),
            cost_class: CostClass::Traversal,
            estimated_output: initial,
        },
        PlanStep {
            description: "sort descending".to_string(),
            cost_class: CostClass::Scan,
            estimated_output: initial,
        },
    ];
    if let Some(n) = limit {
        steps.push(PlanStep {
            description: format!("limit {}", n),
            cost_class: CostClass::Scan,
            estimated_output: n.min(initial),
        });
    }
    QueryPlan {
        steps,
        estimated_candidates_initial: initial,
        estimated_output: est,
    }
}

fn default_single_step_plan(query: &Query) -> QueryPlan {
    QueryPlan {
        estimated_candidates_initial: 1,
        estimated_output: 1,
        steps: vec![PlanStep {
            description: format!("{:?}", std::mem::discriminant(query)),
            cost_class: CostClass::Scan,
            estimated_output: 1,
        }],
    }
}
