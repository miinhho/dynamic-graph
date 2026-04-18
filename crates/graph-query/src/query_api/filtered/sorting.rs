use graph_core::{EntityId, RelationshipId};
use graph_world::World;

use crate::query_api::{EntitySort, LocusSort, LocusSummary, RelSort};

pub(super) fn sort_relationship_ids(
    world: &World,
    ids: Vec<RelationshipId>,
    sort: &RelSort,
    limit: Option<usize>,
) -> Vec<RelationshipId> {
    match sort {
        RelSort::ActivityDesc => sort_relationship_ids_by_f32(
            world,
            ids,
            limit,
            |relationship| relationship.activity(),
            true,
        ),
        RelSort::StrengthDesc => sort_relationship_ids_by_f32(
            world,
            ids,
            limit,
            |relationship| relationship.strength(),
            true,
        ),
        RelSort::WeightDesc => sort_relationship_ids_by_f32(
            world,
            ids,
            limit,
            |relationship| relationship.weight(),
            true,
        ),
        RelSort::ChangeCountDesc => sort_relationship_ids_by_u64(
            world,
            ids,
            limit,
            |relationship| relationship.lineage.change_count,
            true,
        ),
        RelSort::CreatedBatchAsc => sort_relationship_ids_by_u64(
            world,
            ids,
            limit,
            |relationship| relationship.created_batch.0,
            false,
        ),
    }
}

fn sort_relationship_ids_by_f32<F>(
    world: &World,
    ids: Vec<RelationshipId>,
    limit: Option<usize>,
    score: F,
    descending: bool,
) -> Vec<RelationshipId>
where
    F: Fn(&graph_core::Relationship) -> f32,
{
    let mut keyed: Vec<(RelationshipId, f32)> = ids
        .into_iter()
        .filter_map(|id| {
            world
                .relationships()
                .get(id)
                .map(|relationship| (id, score(relationship)))
        })
        .collect();
    if descending {
        keyed.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
    } else {
        keyed.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
    }
    truncate_keyed(&mut keyed, limit);
    keyed.into_iter().map(|(id, _)| id).collect()
}

fn sort_relationship_ids_by_u64<F>(
    world: &World,
    ids: Vec<RelationshipId>,
    limit: Option<usize>,
    score: F,
    descending: bool,
) -> Vec<RelationshipId>
where
    F: Fn(&graph_core::Relationship) -> u64,
{
    let mut keyed: Vec<(RelationshipId, u64)> = ids
        .into_iter()
        .filter_map(|id| {
            world
                .relationships()
                .get(id)
                .map(|relationship| (id, score(relationship)))
        })
        .collect();
    if descending {
        keyed.sort_unstable_by_key(|(_, value)| std::cmp::Reverse(*value));
    } else {
        keyed.sort_unstable_by_key(|(_, value)| *value);
    }
    truncate_keyed(&mut keyed, limit);
    keyed.into_iter().map(|(id, _)| id).collect()
}

fn truncate_keyed<T>(items: &mut Vec<T>, limit: Option<usize>) {
    if let Some(n) = limit {
        items.truncate(n);
    }
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

fn locus_sort_value(summary: &LocusSummary, slot: usize) -> f32 {
    summary
        .state
        .get(slot)
        .copied()
        .unwrap_or(f32::NEG_INFINITY)
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
