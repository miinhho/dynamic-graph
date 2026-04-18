use graph_core::{Endpoints, LocusId};
use graph_world::World;

use super::primitives::{bfs_path, bfs_reachable};

pub fn reachable_from_active(
    world: &World,
    start: LocusId,
    depth: usize,
    min_activity: f32,
) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(
            world
                .relationships_for_locus(locus)
                .filter(|relationship| relationship.activity() >= min_activity)
                .map(|relationship| relationship.endpoints.other_than(locus)),
        );
    })
}

pub fn downstream_of_active(
    world: &World,
    start: LocusId,
    depth: usize,
    min_activity: f32,
) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(
            world
                .relationships_for_locus(locus)
                .filter(|relationship| relationship.activity() >= min_activity)
                .filter_map(move |relationship| match relationship.endpoints {
                    Endpoints::Directed { from, to } if from == locus => Some(to),
                    Endpoints::Directed { .. } => None,
                    Endpoints::Symmetric { .. } => Some(relationship.endpoints.other_than(locus)),
                }),
        );
    })
}

pub fn upstream_of_active(
    world: &World,
    start: LocusId,
    depth: usize,
    min_activity: f32,
) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(
            world
                .relationships_for_locus(locus)
                .filter(|relationship| relationship.activity() >= min_activity)
                .filter_map(move |relationship| match relationship.endpoints {
                    Endpoints::Directed { from, to } if to == locus => Some(from),
                    Endpoints::Directed { .. } => None,
                    Endpoints::Symmetric { .. } => Some(relationship.endpoints.other_than(locus)),
                }),
        );
    })
}

pub fn path_between_active(
    world: &World,
    from: LocusId,
    to: LocusId,
    min_activity: f32,
) -> Option<Vec<LocusId>> {
    bfs_path(from, to, |locus, buf| {
        buf.extend(
            world
                .relationships_for_locus(locus)
                .filter(|relationship| relationship.activity() >= min_activity)
                .map(|relationship| relationship.endpoints.other_than(locus)),
        );
    })
}
