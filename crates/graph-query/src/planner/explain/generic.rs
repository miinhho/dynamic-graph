use graph_world::World;

use super::generic_stages::{
    GenericQueryDescription, StageBlueprint, TraversalBlueprint, WorldStats, assemble_query_plan,
};
use crate::planner::{CostClass, QueryPlan};
use crate::query_api::Query;

pub(super) fn explain_non_find_query(world: &World, query: &Query) -> QueryPlan {
    let stats = WorldStats::from_world(world);
    let description = describe_non_find_query(query, stats, world);
    assemble_query_plan(description)
}

fn describe_non_find_query(
    query: &Query,
    stats: WorldStats,
    world: &World,
) -> GenericQueryDescription {
    match query {
        Query::CausalDirection { .. }
        | Query::DominantCauses { .. }
        | Query::DominantEffects { .. }
        | Query::CausalInStrength { .. }
        | Query::CausalOutStrength { .. }
        | Query::FeedbackPairs { .. } => describe_weighted_causality_query(query, stats),
        Query::GrangerScore { .. }
        | Query::GrangerDominantCauses { .. }
        | Query::GrangerDominantEffects { .. } => describe_granger_query(query, stats),
        Query::TimeTravel { .. }
        | Query::CounterfactualReplay { .. }
        | Query::EntityTransitionCause { .. }
        | Query::EntityUpstreamTransitions { .. }
        | Query::EntityLayersInRange { .. } => describe_temporal_entity_query(query, stats, world),
        Query::AllBetweenness { .. }
        | Query::AllCloseness { .. }
        | Query::AllConstraints { .. }
        | Query::PageRank { .. }
        | Query::Louvain
        | Query::LouvainWithResolution(_) => describe_centrality_query(query, stats),
        _ => GenericQueryDescription::Fallback(format!("{:?}", std::mem::discriminant(query))),
    }
}

fn describe_weighted_causality_query(query: &Query, stats: WorldStats) -> GenericQueryDescription {
    match query {
        Query::CausalDirection { kind, .. } => relationship_scan(
            stats,
            format!(
                "causal_direction scan over relationships of kind {:?}",
                kind
            ),
            Some(1),
        ),
        Query::DominantCauses { kind, n, .. } => relationship_scan(
            stats,
            format!("dominant_causes scan for kind {:?}, top {}", kind, n),
            Some(*n),
        ),
        Query::DominantEffects { kind, n, .. } => relationship_scan(
            stats,
            format!("dominant_effects scan for kind {:?}, top {}", kind, n),
            Some(*n),
        ),
        Query::CausalInStrength { kind, .. } | Query::CausalOutStrength { kind, .. } => {
            relationship_scan(
                stats,
                format!("causal strength scan over relationships of kind {:?}", kind),
                Some(1),
            )
        }
        Query::FeedbackPairs { kind, .. } => relationship_scan(
            stats,
            format!("feedback_pairs scan for kind {:?} (two passes)", kind),
            None,
        ),
        _ => unreachable!("describe_weighted_causality_query called with non-causality query"),
    }
}

fn describe_granger_query(query: &Query, stats: WorldStats) -> GenericQueryDescription {
    match query {
        Query::GrangerScore { kind, .. } => changelog_scan(
            stats,
            format!("granger_score ChangeLog scan for kind {:?}", kind),
            Some(1),
        ),
        Query::GrangerDominantCauses { kind, n, .. } => changelog_scan(
            stats,
            format!(
                "granger_dominant_causes ChangeLog scan for kind {:?}, top {}",
                kind, n
            ),
            Some(*n),
        ),
        Query::GrangerDominantEffects { kind, n, .. } => changelog_scan(
            stats,
            format!(
                "granger_dominant_effects ChangeLog scan for kind {:?}, top {}",
                kind, n
            ),
            Some(*n),
        ),
        _ => unreachable!("describe_granger_query called with non-granger query"),
    }
}

fn describe_temporal_entity_query(
    query: &Query,
    stats: WorldStats,
    world: &World,
) -> GenericQueryDescription {
    match query {
        Query::TimeTravel { .. } => scan(
            stats.log_entries + stats.relationships + stats.entities,
            "time_travel: O(changes + R + E·layers) — WorldDiff inversion".to_string(),
            None,
        ),
        Query::CounterfactualReplay { remove_changes } => scan(
            stats.log_entries + stats.relationships,
            format!(
                "counterfactual_replay: O(descendants of {} roots + R={})",
                remove_changes.len(),
                stats.relationships
            ),
            None,
        ),
        Query::EntityTransitionCause { .. } => scan(
            stats.entities,
            "entity_transition_cause: O(entity layers)".to_string(),
            None,
        ),
        Query::EntityUpstreamTransitions { .. } => scan(
            stats.log_entries + stats.entities,
            "entity_upstream_transitions: O(ChangeLog ancestors + entity scan)".to_string(),
            None,
        ),
        Query::EntityLayersInRange { entity_id, .. } => scan(
            world
                .entities()
                .get(*entity_id)
                .map_or(0, |entity| entity.layers.len()),
            "entity_layers_in_range: O(entity layer count)".to_string(),
            None,
        ),
        _ => unreachable!("describe_temporal_entity_query called with unrelated query"),
    }
}

fn describe_centrality_query(query: &Query, stats: WorldStats) -> GenericQueryDescription {
    match query {
        Query::AllBetweenness { limit } => traversal(
            stats.relationships,
            "Brandes betweenness centrality over all loci".to_string(),
            *limit,
        ),
        Query::AllCloseness { limit } => traversal(
            stats.loci,
            "Harmonic closeness centrality over all loci".to_string(),
            *limit,
        ),
        Query::AllConstraints { limit } => traversal(
            stats.loci,
            "Burt structural constraint over all loci".to_string(),
            *limit,
        ),
        Query::PageRank { limit, .. } => {
            traversal(stats.loci, "PageRank over all loci".to_string(), *limit)
        }
        Query::Louvain | Query::LouvainWithResolution(_) => traversal(
            stats.relationships,
            "Louvain community detection".to_string(),
            None,
        ),
        _ => unreachable!("describe_centrality_query called with non-centrality query"),
    }
}

fn relationship_scan(
    stats: WorldStats,
    description: String,
    limit: Option<usize>,
) -> GenericQueryDescription {
    scan(stats.relationships, description, limit)
}

fn changelog_scan(
    stats: WorldStats,
    description: String,
    limit: Option<usize>,
) -> GenericQueryDescription {
    scan(stats.log_entries, description, limit)
}

fn scan(
    initial_candidates: usize,
    description: String,
    limit: Option<usize>,
) -> GenericQueryDescription {
    GenericQueryDescription::SingleStage(StageBlueprint {
        initial_candidates,
        description,
        cost_class: CostClass::Scan,
        limit,
    })
}

fn traversal(
    initial_candidates: usize,
    description: String,
    limit: Option<usize>,
) -> GenericQueryDescription {
    GenericQueryDescription::Traversal(TraversalBlueprint {
        initial_candidates,
        description,
        limit,
    })
}
