use graph_world::World;

use super::{EntityDiffSummary, Query, QueryResult, coheres_to_results, rel_to_summary};

pub(super) fn execute_entity_and_counterfactual(
    world: &World,
    query: &Query,
) -> Option<QueryResult> {
    match query {
        Query::EntityDeviationsSince(_) => Some(execute_entity_deviation_query(world, query)),
        Query::RelationshipsAbsentWithout(_) => Some(execute_counterfactual_query(world, query)),
        Query::Coheres | Query::CoheresNamed(_) => Some(execute_cohere_query(world, query)),
        _ => None,
    }
}

fn execute_entity_deviation_query(world: &World, query: &Query) -> QueryResult {
    match query {
        Query::EntityDeviationsSince(baseline) => {
            QueryResult::EntityDeviations(entity_deviation_summaries(world, *baseline))
        }
        _ => unreachable!("entity deviation dispatcher received non-entity query"),
    }
}

fn execute_counterfactual_query(world: &World, query: &Query) -> QueryResult {
    match query {
        Query::RelationshipsAbsentWithout(root_changes) => {
            QueryResult::RelationshipSummaries(absent_relationship_summaries(world, root_changes))
        }
        _ => unreachable!("counterfactual dispatcher received non-counterfactual query"),
    }
}

fn execute_cohere_query(world: &World, query: &Query) -> QueryResult {
    let coheres = match query {
        Query::Coheres => world.coheres().get("default").unwrap_or(&[]),
        Query::CoheresNamed(key) => world.coheres().get(key.as_str()).unwrap_or(&[]),
        _ => unreachable!("cohere dispatcher received non-cohere query"),
    };
    QueryResult::Coheres(coheres_to_results(coheres))
}

fn entity_deviation_summaries(
    world: &World,
    baseline: graph_core::BatchId,
) -> Vec<EntityDiffSummary> {
    crate::entity_deviations_since(world, baseline)
        .into_iter()
        .map(|diff| EntityDiffSummary {
            entity_id: diff.entity_id,
            born_after_baseline: diff.born_after_baseline,
            went_dormant: diff.went_dormant,
            revived: diff.revived,
            members_added: diff.members_added,
            members_removed: diff.members_removed,
            membership_event_count: diff.membership_event_count,
            coherence_at_baseline: diff.coherence_at_baseline,
            coherence_now: diff.coherence_now,
            coherence_delta: diff.coherence_delta,
            member_count_delta: diff.member_count_delta,
            latest_change_batch: diff.latest_change_batch,
        })
        .collect()
}

fn absent_relationship_summaries(
    world: &World,
    root_changes: &[graph_core::ChangeId],
) -> Vec<super::RelationshipSummary> {
    crate::relationships_absent_without(world, root_changes)
        .iter()
        .filter_map(|&id| world.relationships().get(id))
        .map(rel_to_summary)
        .collect()
}
