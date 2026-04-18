use graph_core::RelationshipId;
use graph_world::World;

use super::{Query, QueryResult};

pub(super) fn execute_structural(world: &World, query: &Query) -> Option<QueryResult> {
    use crate::traversal::{
        downstream_of_active, path_between_active, reachable_from_active, upstream_of_active,
    };
    use crate::*;

    match query {
        Query::PathBetween { from, to } => Some(QueryResult::Path(path_between(world, *from, *to))),
        Query::PathBetweenOfKind { from, to, kind } => Some(QueryResult::Path(
            path_between_of_kind(world, *from, *to, *kind),
        )),
        Query::DirectedPath { from, to } => {
            Some(QueryResult::Path(directed_path(world, *from, *to)))
        }
        Query::ReachableFrom { start, depth } => {
            Some(QueryResult::Loci(reachable_from(world, *start, *depth)))
        }
        Query::DownstreamOf { start, depth } => {
            Some(QueryResult::Loci(downstream_of(world, *start, *depth)))
        }
        Query::UpstreamOf { start, depth } => {
            Some(QueryResult::Loci(upstream_of(world, *start, *depth)))
        }
        Query::ReachableFromActive {
            start,
            depth,
            min_activity,
        } => Some(QueryResult::Loci(reachable_from_active(
            world,
            *start,
            *depth,
            *min_activity,
        ))),
        Query::DownstreamOfActive {
            start,
            depth,
            min_activity,
        } => Some(QueryResult::Loci(downstream_of_active(
            world,
            *start,
            *depth,
            *min_activity,
        ))),
        Query::UpstreamOfActive {
            start,
            depth,
            min_activity,
        } => Some(QueryResult::Loci(upstream_of_active(
            world,
            *start,
            *depth,
            *min_activity,
        ))),
        Query::PathBetweenActive {
            from,
            to,
            min_activity,
        } => Some(QueryResult::Path(path_between_active(
            world,
            *from,
            *to,
            *min_activity,
        ))),
        Query::ConnectedComponents => Some(QueryResult::Components(connected_components(world))),
        Query::ConnectedComponentsOfKind(kind) => Some(QueryResult::Components(
            connected_components_of_kind(world, *kind),
        )),
        Query::NeighborsOf(locus) => Some(QueryResult::Loci(neighbors_of(world, *locus))),
        Query::IsolatedLoci => Some(QueryResult::Loci(isolated_loci(world))),
        Query::HubLoci(n) => Some(QueryResult::Loci(hub_loci(world, *n))),
        Query::ReciprocOf(rel_id) => {
            let result = reciprocal_of(world, *rel_id);
            Some(QueryResult::Relationships(
                result.map(|id| vec![id]).unwrap_or_default(),
            ))
        }
        Query::ReciprocPairs => {
            let pairs = reciprocal_pairs(world);
            let flat: Vec<RelationshipId> = pairs.into_iter().flat_map(|(a, b)| [a, b]).collect();
            Some(QueryResult::Relationships(flat))
        }
        Query::HasCycle => Some(QueryResult::Bool(has_cycle(world))),
        Query::StrongestPath { from, to } => {
            Some(QueryResult::Path(strongest_path(world, *from, *to)))
        }
        _ => None,
    }
}
