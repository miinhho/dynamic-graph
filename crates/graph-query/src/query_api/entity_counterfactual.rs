use graph_world::World;

use super::{EntityDiffSummary, Query, QueryResult, coheres_to_results, rel_to_summary};

pub(super) fn execute_entity_and_counterfactual(
    world: &World,
    query: &Query,
) -> Option<QueryResult> {
    match query {
        Query::EntityDeviationsSince(baseline) => Some(QueryResult::EntityDeviations(
            entity_deviation_summaries(world, *baseline),
        )),
        Query::RelationshipsAbsentWithout(root_changes) => Some(
            QueryResult::RelationshipSummaries(absent_relationship_summaries(world, root_changes)),
        ),
        Query::Coheres => Some(QueryResult::Coheres(coheres_to_results(
            world.coheres().get("default").unwrap_or(&[]),
        ))),
        Query::CoheresNamed(key) => Some(QueryResult::Coheres(coheres_to_results(
            world.coheres().get(key.as_str()).unwrap_or(&[]),
        ))),
        _ => None,
    }
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
