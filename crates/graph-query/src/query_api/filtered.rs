mod candidates;
mod predicates;
mod sorting;

use graph_core::{EntityId, LocusId};
use graph_world::World;
use rustc_hash::FxHashSet;

use self::{
    candidates::relationship_candidates,
    predicates::{
        entity_predicate_matches, graph_locus_members, locus_predicate_matches, rel_pred_matches,
    },
    sorting::{sort_entity_ids, sort_loci_summaries, sort_relationship_summaries},
};
use super::{
    EntityPredicate, LocusPredicate, LocusSummary, Query, QueryResult, RelSort,
    RelationshipPredicate, RelationshipSummary, rel_to_summary,
};

pub(super) fn execute_filtered_lookup(world: &World, query: &Query) -> Option<QueryResult> {
    match query {
        Query::FindLoci { .. } => execute_find_loci(world, query),
        Query::FindRelationships { .. } => execute_find_relationships(world, query),
        Query::FindEntities { .. } => execute_find_entities(world, query),
        _ => None,
    }
}

fn execute_find_loci(world: &World, query: &Query) -> Option<QueryResult> {
    let Query::FindLoci {
        predicates,
        sort_by,
        limit,
    } = query
    else {
        return None;
    };

    let summaries = sort_loci_summaries(world, find_loci_summaries(world, predicates), sort_by);
    Some(QueryResult::LocusSummaries(limit_items(summaries, *limit)))
}

fn execute_find_relationships(world: &World, query: &Query) -> Option<QueryResult> {
    let Query::FindRelationships {
        predicates,
        sort_by,
        limit,
    } = query
    else {
        return None;
    };

    Some(QueryResult::RelationshipSummaries(
        find_relationship_summaries(world, predicates, sort_by.as_ref(), *limit),
    ))
}

fn execute_find_entities(world: &World, query: &Query) -> Option<QueryResult> {
    let Query::FindEntities {
        predicates,
        sort_by,
        limit,
    } = query
    else {
        return None;
    };

    let ids = sort_entity_ids(world, find_entities_inner(world, predicates), sort_by);
    Some(QueryResult::Entities(limit_items(ids, *limit)))
}

pub(super) fn find_loci_summaries(
    world: &World,
    predicates: &[LocusPredicate],
) -> Vec<LocusSummary> {
    use crate::planner::plan_loci_predicates;

    plan_loci_predicates(predicates)
        .iter()
        .fold(seed_locus_candidates(world), |candidates, predicate| {
            apply_locus_predicate(world, candidates, predicate)
        })
        .into_iter()
        .filter_map(|id| project_locus_summary(world, id))
        .collect()
}

pub(super) fn find_relationship_summaries(
    world: &World,
    predicates: &[RelationshipPredicate],
    sort_by: Option<&RelSort>,
    limit: Option<usize>,
) -> Vec<RelationshipSummary> {
    use crate::planner::plan_rel_predicates;

    let plan = plan_rel_predicates(predicates);
    let filtered =
        filtered_relationship_summaries(world, &plan.seed_locus, &plan.predicates_ordered);

    match sort_by {
        None => match limit {
            Some(n) => filtered.take(n).collect(),
            None => filtered.collect(),
        },
        Some(sort) => sort_relationship_summaries(filtered.collect(), sort, limit),
    }
}

fn filtered_relationship_summaries<'a>(
    world: &'a World,
    seed_locus: &'a Option<crate::planner::SeedKind>,
    predicates_ordered: &'a [&'a RelationshipPredicate],
) -> impl Iterator<Item = RelationshipSummary> + 'a {
    relationship_candidates(world, seed_locus)
        .into_iter()
        .filter_map(|id| world.relationships().get(id))
        .filter(|relationship| {
            predicates_ordered
                .iter()
                .all(|predicate| rel_pred_matches(relationship, predicate))
        })
        .map(rel_to_summary)
}

pub(super) fn find_entities_inner(world: &World, predicates: &[EntityPredicate]) -> Vec<EntityId> {
    predicates
        .iter()
        .fold(seed_entity_candidates(world), |candidates, predicate| {
            apply_entity_predicate(world, candidates, predicate)
        })
}

fn retain_in_set(candidates: &mut Vec<LocusId>, members: Vec<LocusId>) {
    let set: FxHashSet<LocusId> = members.into_iter().collect();
    candidates.retain(|id| set.contains(id));
}

fn seed_locus_candidates(world: &World) -> Vec<LocusId> {
    world.loci().iter().map(|l| l.id).collect()
}

fn seed_entity_candidates(world: &World) -> Vec<EntityId> {
    world.entities().active().map(|e| e.id).collect()
}

fn apply_locus_predicate(
    world: &World,
    candidates: Vec<LocusId>,
    predicate: &LocusPredicate,
) -> Vec<LocusId> {
    if let Some(members) = graph_locus_members(world, predicate) {
        return retain_members(candidates, members);
    }

    retain_loci_matching(world, candidates, predicate)
}

fn retain_members(mut candidates: Vec<LocusId>, members: Vec<LocusId>) -> Vec<LocusId> {
    retain_in_set(&mut candidates, members);
    candidates
}

fn retain_loci_matching(
    world: &World,
    candidates: Vec<LocusId>,
    predicate: &LocusPredicate,
) -> Vec<LocusId> {
    retain_matching(candidates, |id| locus_predicate_matches(world, id, predicate))
}

fn apply_entity_predicate(
    world: &World,
    candidates: Vec<EntityId>,
    predicate: &EntityPredicate,
) -> Vec<EntityId> {
    retain_matching(candidates, |id| entity_predicate_matches(world, id, predicate))
}

fn retain_matching<T, F>(mut candidates: Vec<T>, mut predicate: F) -> Vec<T>
where
    F: FnMut(T) -> bool,
    T: Copy,
{
    candidates.retain(|candidate| predicate(*candidate));
    candidates
}

fn project_locus_summary(world: &World, id: LocusId) -> Option<LocusSummary> {
    world.locus(id).map(|locus| LocusSummary {
        id: locus.id,
        kind: locus.kind,
        state: locus.state.as_slice().to_vec(),
    })
}


fn limit_items<T>(mut items: Vec<T>, limit: Option<usize>) -> Vec<T> {
    if let Some(n) = limit {
        items.truncate(n);
    }
    items
}
