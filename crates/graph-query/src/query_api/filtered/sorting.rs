use graph_core::EntityId;
use graph_world::World;

use crate::query_api::{
    EntitySort, LocusSort, LocusSummary, RelSort, RelationshipSummary,
};

pub(super) fn sort_relationship_summaries(
    mut summaries: Vec<RelationshipSummary>,
    sort: &RelSort,
    limit: Option<usize>,
) -> Vec<RelationshipSummary> {
    match sort {
        RelSort::ActivityDesc => summaries.sort_unstable_by(compare_activity_desc),
        RelSort::StrengthDesc => summaries.sort_unstable_by(compare_strength_desc),
        RelSort::WeightDesc => summaries.sort_unstable_by(compare_weight_desc),
        RelSort::ChangeCountDesc => {
            summaries.sort_unstable_by(|a, b| b.change_count.cmp(&a.change_count))
        }
        RelSort::CreatedBatchAsc => summaries.sort_unstable_by_key(|s| s.created_batch.0),
    }
    if let Some(n) = limit {
        summaries.truncate(n);
    }
    summaries
}

pub(super) fn sort_loci_summaries(
    world: &World,
    mut summaries: Vec<LocusSummary>,
    sort_by: &Option<LocusSort>,
) -> Vec<LocusSummary> {
    if let Some(sort) = sort_by {
        match sort {
            LocusSort::StateDesc(slot) => {
                summaries.sort_unstable_by(|a, b| {
                    locus_sort_value(b, *slot).total_cmp(&locus_sort_value(a, *slot))
                });
            }
            LocusSort::DegreeDesc => {
                summaries
                    .sort_unstable_by_key(|summary| std::cmp::Reverse(world.degree(summary.id)));
            }
        }
    }
    summaries
}

pub(super) fn sort_entity_ids(
    world: &World,
    mut ids: Vec<EntityId>,
    sort_by: &Option<EntitySort>,
) -> Vec<EntityId> {
    if let Some(sort) = sort_by {
        match sort {
            EntitySort::CoherenceDesc => {
                ids.sort_unstable_by(|a, b| {
                    entity_coherence(world, *b).total_cmp(&entity_coherence(world, *a))
                });
            }
            EntitySort::MemberCountDesc => {
                ids.sort_unstable_by_key(|id| std::cmp::Reverse(entity_member_count(world, *id)));
            }
        }
    }
    ids
}

fn compare_activity_desc(a: &RelationshipSummary, b: &RelationshipSummary) -> std::cmp::Ordering {
    b.activity.total_cmp(&a.activity)
}

fn compare_strength_desc(a: &RelationshipSummary, b: &RelationshipSummary) -> std::cmp::Ordering {
    relationship_strength(b).total_cmp(&relationship_strength(a))
}

fn compare_weight_desc(a: &RelationshipSummary, b: &RelationshipSummary) -> std::cmp::Ordering {
    b.weight.total_cmp(&a.weight)
}

fn relationship_strength(summary: &RelationshipSummary) -> f32 {
    summary.activity + summary.weight
}

fn locus_sort_value(summary: &LocusSummary, slot: usize) -> f32 {
    summary.state.get(slot).copied().unwrap_or(f32::NEG_INFINITY)
}

fn entity_coherence(world: &World, id: EntityId) -> f32 {
    world
        .entities()
        .get(id)
        .map(|entity| entity.current.coherence)
        .unwrap_or(0.0)
}

fn entity_member_count(world: &World, id: EntityId) -> usize {
    world
        .entities()
        .get(id)
        .map(|entity| entity.current.members.len())
        .unwrap_or(0)
}
