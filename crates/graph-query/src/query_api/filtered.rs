use graph_core::{EndpointKey, Endpoints, EntityId, LocusId, RelationshipId};
use graph_world::World;
use rustc_hash::FxHashSet;

use super::{
    EntityPredicate, EntitySort, LocusPredicate, LocusSort, LocusSummary, Query, QueryResult,
    RelSort, RelationshipPredicate, RelationshipSummary, rel_to_summary,
};

pub(super) fn execute_filtered_lookup(world: &World, query: &Query) -> Option<QueryResult> {
    match query {
        Query::FindLoci {
            predicates,
            sort_by,
            limit,
        } => {
            let summaries =
                sort_loci_summaries(world, find_loci_summaries(world, predicates), sort_by);
            Some(QueryResult::LocusSummaries(limit_items(summaries, *limit)))
        }
        Query::FindRelationships {
            predicates,
            sort_by,
            limit,
        } => Some(QueryResult::RelationshipSummaries(
            find_relationship_summaries(world, predicates, sort_by.as_ref(), *limit),
        )),
        Query::FindEntities {
            predicates,
            sort_by,
            limit,
        } => {
            let ids = sort_entity_ids(world, find_entities_inner(world, predicates), sort_by);
            Some(QueryResult::Entities(limit_items(ids, *limit)))
        }
        _ => None,
    }
}

pub(super) fn find_loci_summaries(
    world: &World,
    predicates: &[LocusPredicate],
) -> Vec<LocusSummary> {
    use crate::planner::plan_loci_predicates;
    use crate::traversal::{
        downstream_of, downstream_of_active, reachable_from, reachable_from_active, upstream_of,
        upstream_of_active,
    };

    let mut candidates: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();
    for pred in plan_loci_predicates(predicates) {
        match pred {
            LocusPredicate::OfKind(kind) => {
                candidates.retain(|&id| world.locus(id).is_some_and(|l| l.kind == *kind));
            }
            LocusPredicate::StateAbove { slot, min } => {
                candidates.retain(|&id| {
                    world
                        .locus(id)
                        .and_then(|l| l.state.as_slice().get(*slot).copied())
                        .is_some_and(|v| v >= *min)
                });
            }
            LocusPredicate::StateBelow { slot, max } => {
                candidates.retain(|&id| {
                    world
                        .locus(id)
                        .and_then(|l| l.state.as_slice().get(*slot).copied())
                        .is_some_and(|v| v <= *max)
                });
            }
            LocusPredicate::StrPropertyEq { key, value } => {
                candidates.retain(|&id| {
                    world
                        .properties()
                        .get(id)
                        .and_then(|p| p.get_str(key))
                        .is_some_and(|v| v == value.as_str())
                });
            }
            LocusPredicate::F64PropertyAbove { key, min } => {
                candidates.retain(|&id| {
                    world
                        .properties()
                        .get(id)
                        .and_then(|p| p.get_f64(key))
                        .is_some_and(|v| v >= *min)
                });
            }
            LocusPredicate::MinDegree(min) => candidates.retain(|&id| world.degree(id) >= *min),
            LocusPredicate::ReachableFromActive {
                start,
                depth,
                min_activity,
            } => retain_in_set(
                &mut candidates,
                reachable_from_active(world, *start, *depth, *min_activity),
            ),
            LocusPredicate::DownstreamOfActive {
                start,
                depth,
                min_activity,
            } => retain_in_set(
                &mut candidates,
                downstream_of_active(world, *start, *depth, *min_activity),
            ),
            LocusPredicate::UpstreamOfActive {
                start,
                depth,
                min_activity,
            } => retain_in_set(
                &mut candidates,
                upstream_of_active(world, *start, *depth, *min_activity),
            ),
            LocusPredicate::ReachableFrom { start, depth } => {
                retain_in_set(&mut candidates, reachable_from(world, *start, *depth))
            }
            LocusPredicate::DownstreamOf { start, depth } => {
                retain_in_set(&mut candidates, downstream_of(world, *start, *depth))
            }
            LocusPredicate::UpstreamOf { start, depth } => {
                retain_in_set(&mut candidates, upstream_of(world, *start, *depth))
            }
        }
    }

    candidates
        .into_iter()
        .filter_map(|id| {
            world.locus(id).map(|l| LocusSummary {
                id: l.id,
                kind: l.kind,
                state: l.state.as_slice().to_vec(),
            })
        })
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
    let candidates = relationship_candidates(world, &plan.seed_locus);
    let filtered = candidates.into_iter().filter_map(|id| {
        world.relationships().get(id).and_then(|r| {
            plan.predicates_ordered
                .iter()
                .all(|pred| rel_pred_matches(r, pred))
                .then(|| rel_to_summary(r))
        })
    });

    match sort_by {
        None => match limit {
            Some(n) => filtered.take(n).collect(),
            None => filtered.collect(),
        },
        Some(sort) => sort_relationship_summaries(filtered.collect(), sort, limit),
    }
}

pub(super) fn find_entities_inner(world: &World, predicates: &[EntityPredicate]) -> Vec<EntityId> {
    let mut candidates: Vec<EntityId> = world.entities().active().map(|e| e.id).collect();
    for pred in predicates {
        match pred {
            EntityPredicate::CoherenceAbove(min) => {
                candidates.retain(|&id| {
                    world
                        .entities()
                        .get(id)
                        .is_some_and(|e| e.current.coherence >= *min)
                });
            }
            EntityPredicate::HasMember(locus) => {
                candidates.retain(|&id| {
                    world
                        .entities()
                        .get(id)
                        .is_some_and(|e| e.current.members.contains(locus))
                });
            }
            EntityPredicate::MinMembers(min) => {
                candidates.retain(|&id| {
                    world
                        .entities()
                        .get(id)
                        .is_some_and(|e| e.current.members.len() >= *min)
                });
            }
        }
    }
    candidates
}

fn retain_in_set(candidates: &mut Vec<LocusId>, members: Vec<LocusId>) {
    let set: FxHashSet<LocusId> = members.into_iter().collect();
    candidates.retain(|id| set.contains(id));
}

fn relationship_candidates(
    world: &World,
    seed: &Option<crate::planner::SeedKind>,
) -> Vec<RelationshipId> {
    use crate::planner::SeedKind;

    match seed {
        Some(SeedKind::DirectLookup { from, to, kind }) => {
            let key = EndpointKey::Directed(*from, *to);
            world
                .relationships()
                .lookup(&key, *kind)
                .map(|id| vec![id])
                .unwrap_or_default()
        }
        Some(SeedKind::Between { a, b }) => {
            world.relationships_between(*a, *b).map(|r| r.id).collect()
        }
        Some(SeedKind::From(locus)) => world
            .relationships_for_locus(*locus)
            .filter(|r| matches!(r.endpoints, Endpoints::Directed { from, .. } if from == *locus))
            .map(|r| r.id)
            .collect(),
        Some(SeedKind::To(locus)) => world
            .relationships_for_locus(*locus)
            .filter(|r| matches!(r.endpoints, Endpoints::Directed { to, .. } if to == *locus))
            .map(|r| r.id)
            .collect(),
        Some(SeedKind::Touching(locus)) => world
            .relationships_for_locus(*locus)
            .map(|r| r.id)
            .collect(),
        None => world.relationships().iter().map(|r| r.id).collect(),
    }
}

fn rel_pred_matches(r: &graph_core::Relationship, pred: &RelationshipPredicate) -> bool {
    match pred {
        RelationshipPredicate::OfKind(kind) => r.kind == *kind,
        RelationshipPredicate::From(locus) => {
            matches!(r.endpoints, Endpoints::Directed { from, .. } if from == *locus)
        }
        RelationshipPredicate::To(locus) => {
            matches!(r.endpoints, Endpoints::Directed { to, .. } if to == *locus)
        }
        RelationshipPredicate::Touching(locus) => r.endpoints.involves(*locus),
        RelationshipPredicate::ActivityAbove(min) => r.activity() > *min,
        RelationshipPredicate::StrengthAbove(min) => r.strength() > *min,
        RelationshipPredicate::SlotAbove { slot, min } => {
            r.state.as_slice().get(*slot).is_some_and(|&v| v >= *min)
        }
        RelationshipPredicate::CreatedInRange { from, to } => {
            r.created_batch >= *from && r.created_batch <= *to
        }
        RelationshipPredicate::OlderThan {
            current_batch,
            min_batches,
        } => r.age_in_batches(*current_batch) >= *min_batches,
        RelationshipPredicate::MinChangeCount(min) => r.lineage.change_count >= *min,
    }
}

fn sort_relationship_summaries(
    mut summaries: Vec<RelationshipSummary>,
    sort: &RelSort,
    limit: Option<usize>,
) -> Vec<RelationshipSummary> {
    match sort {
        RelSort::ActivityDesc => {
            summaries.sort_unstable_by(|a, b| b.activity.total_cmp(&a.activity))
        }
        RelSort::StrengthDesc => summaries
            .sort_unstable_by(|a, b| (b.activity + b.weight).total_cmp(&(a.activity + a.weight))),
        RelSort::WeightDesc => summaries.sort_unstable_by(|a, b| b.weight.total_cmp(&a.weight)),
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

fn sort_loci_summaries(
    world: &World,
    mut summaries: Vec<LocusSummary>,
    sort_by: &Option<LocusSort>,
) -> Vec<LocusSummary> {
    if let Some(sort) = sort_by {
        match sort {
            LocusSort::StateDesc(slot) => {
                summaries.sort_unstable_by(|a, b| {
                    let va = a.state.get(*slot).copied().unwrap_or(f32::NEG_INFINITY);
                    let vb = b.state.get(*slot).copied().unwrap_or(f32::NEG_INFINITY);
                    vb.total_cmp(&va)
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

fn sort_entity_ids(
    world: &World,
    mut ids: Vec<EntityId>,
    sort_by: &Option<EntitySort>,
) -> Vec<EntityId> {
    if let Some(sort) = sort_by {
        match sort {
            EntitySort::CoherenceDesc => {
                ids.sort_unstable_by(|a, b| {
                    let ca = world
                        .entities()
                        .get(*a)
                        .map(|entity| entity.current.coherence)
                        .unwrap_or(0.0);
                    let cb = world
                        .entities()
                        .get(*b)
                        .map(|entity| entity.current.coherence)
                        .unwrap_or(0.0);
                    cb.total_cmp(&ca)
                });
            }
            EntitySort::MemberCountDesc => {
                ids.sort_unstable_by_key(|id| {
                    std::cmp::Reverse(
                        world
                            .entities()
                            .get(*id)
                            .map(|entity| entity.current.members.len())
                            .unwrap_or(0),
                    )
                });
            }
        }
    }
    ids
}

fn limit_items<T>(mut items: Vec<T>, limit: Option<usize>) -> Vec<T> {
    if let Some(n) = limit {
        items.truncate(n);
    }
    items
}
