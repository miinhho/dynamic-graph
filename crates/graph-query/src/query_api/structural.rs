use graph_core::RelationshipId;
use graph_world::World;

use super::{Query, QueryResult};

pub(super) fn execute_structural(world: &World, query: &Query) -> Option<QueryResult> {
    match query {
        Query::PathBetween { .. }
        | Query::PathBetweenOfKind { .. }
        | Query::DirectedPath { .. }
        | Query::PathBetweenActive { .. }
        | Query::StrongestPath { .. } => Some(execute_path_query(world, query)),
        Query::ReachableFrom { .. }
        | Query::DownstreamOf { .. }
        | Query::UpstreamOf { .. }
        | Query::ReachableFromActive { .. }
        | Query::DownstreamOfActive { .. }
        | Query::UpstreamOfActive { .. } => Some(execute_reachability_query(world, query)),
        Query::ConnectedComponents | Query::ConnectedComponentsOfKind(_) => {
            Some(execute_component_query(world, query))
        }
        Query::NeighborsOf(_)
        | Query::IsolatedLoci
        | Query::HubLoci(_)
        | Query::ReciprocOf(_)
        | Query::ReciprocPairs => Some(execute_topology_query(world, query)),
        Query::HasCycle => Some(QueryResult::Bool(crate::has_cycle(world))),
        _ => None,
    }
}

fn execute_path_query(world: &World, query: &Query) -> QueryResult {
    use crate::traversal::path_between_active;
    use crate::{directed_path, path_between, path_between_of_kind, strongest_path};

    match query {
        Query::PathBetween { from, to } => QueryResult::Path(path_between(world, *from, *to)),
        Query::PathBetweenOfKind { from, to, kind } => {
            QueryResult::Path(path_between_of_kind(world, *from, *to, *kind))
        }
        Query::DirectedPath { from, to } => QueryResult::Path(directed_path(world, *from, *to)),
        Query::PathBetweenActive {
            from,
            to,
            min_activity,
        } => QueryResult::Path(path_between_active(world, *from, *to, *min_activity)),
        Query::StrongestPath { from, to } => QueryResult::Path(strongest_path(world, *from, *to)),
        _ => unreachable!("path query dispatcher received non-path query"),
    }
}

fn execute_reachability_query(world: &World, query: &Query) -> QueryResult {
    use crate::traversal::{downstream_of_active, reachable_from_active, upstream_of_active};
    use crate::{downstream_of, reachable_from, upstream_of};

    let loci = match query {
        Query::ReachableFrom { start, depth } => reachable_from(world, *start, *depth),
        Query::DownstreamOf { start, depth } => downstream_of(world, *start, *depth),
        Query::UpstreamOf { start, depth } => upstream_of(world, *start, *depth),
        Query::ReachableFromActive {
            start,
            depth,
            min_activity,
        } => reachable_from_active(world, *start, *depth, *min_activity),
        Query::DownstreamOfActive {
            start,
            depth,
            min_activity,
        } => downstream_of_active(world, *start, *depth, *min_activity),
        Query::UpstreamOfActive {
            start,
            depth,
            min_activity,
        } => upstream_of_active(world, *start, *depth, *min_activity),
        _ => unreachable!("reachability dispatcher received non-reachability query"),
    };
    QueryResult::Loci(loci)
}

fn execute_component_query(world: &World, query: &Query) -> QueryResult {
    use crate::{connected_components, connected_components_of_kind};

    match query {
        Query::ConnectedComponents => QueryResult::Components(connected_components(world)),
        Query::ConnectedComponentsOfKind(kind) => {
            QueryResult::Components(connected_components_of_kind(world, *kind))
        }
        _ => unreachable!("component dispatcher received non-component query"),
    }
}

fn execute_topology_query(world: &World, query: &Query) -> QueryResult {
    use crate::{hub_loci, isolated_loci, neighbors_of, reciprocal_of, reciprocal_pairs};

    match query {
        Query::NeighborsOf(locus) => QueryResult::Loci(neighbors_of(world, *locus)),
        Query::IsolatedLoci => QueryResult::Loci(isolated_loci(world)),
        Query::HubLoci(n) => QueryResult::Loci(hub_loci(world, *n)),
        Query::ReciprocOf(rel_id) => {
            QueryResult::Relationships(reciprocal_of(world, *rel_id).into_iter().collect())
        }
        Query::ReciprocPairs => {
            QueryResult::Relationships(flatten_reciprocal_pairs(reciprocal_pairs(world)))
        }
        _ => unreachable!("topology dispatcher received non-topology query"),
    }
}

fn flatten_reciprocal_pairs(pairs: Vec<(RelationshipId, RelationshipId)>) -> Vec<RelationshipId> {
    pairs.into_iter().flat_map(|(a, b)| [a, b]).collect()
}
