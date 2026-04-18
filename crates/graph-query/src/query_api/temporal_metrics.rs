use graph_world::World;

use super::{Query, QueryResult, WorldMetricsResult};

pub(super) fn execute_temporal_and_metrics(world: &World, query: &Query) -> Option<QueryResult> {
    match query {
        Query::TimeTravel { target_batch } => Some(QueryResult::TimeTravelResult(Box::new(
            crate::time_travel::time_travel(world, *target_batch),
        ))),
        Query::CounterfactualReplay { remove_changes } => Some(QueryResult::Counterfactual(
            crate::counterfactual_replay(world, remove_changes.clone()),
        )),
        Query::WorldMetrics => Some(QueryResult::WorldMetrics(world_metrics_result(world))),
        _ => None,
    }
}

fn world_metrics_result(world: &World) -> WorldMetricsResult {
    let metrics = world.metrics();
    WorldMetricsResult {
        locus_count: metrics.locus_count,
        relationship_count: metrics.relationship_count,
        active_relationship_count: metrics.active_relationship_count,
        mean_activity: metrics.mean_activity,
        max_activity: metrics.max_activity,
        component_count: metrics.component_count,
        largest_component_size: metrics.largest_component_size,
        max_degree: metrics.max_degree,
    }
}
