use graph_core::{LocusId, RelationshipKindId};
use graph_world::World;

use super::neighbors::{predecessors, successors};
use super::primitives::{bfs_path, bfs_reachable};

pub fn downstream_of(world: &World, start: LocusId, depth: usize) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(successors(world, locus, None));
    })
}

pub fn upstream_of(world: &World, start: LocusId, depth: usize) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(predecessors(world, locus, None));
    })
}

pub fn downstream_of_kind(
    world: &World,
    start: LocusId,
    depth: usize,
    kind: RelationshipKindId,
) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(successors(world, locus, Some(kind)));
    })
}

pub fn upstream_of_kind(
    world: &World,
    start: LocusId,
    depth: usize,
    kind: RelationshipKindId,
) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(predecessors(world, locus, Some(kind)));
    })
}

pub fn directed_path(world: &World, from: LocusId, to: LocusId) -> Option<Vec<LocusId>> {
    bfs_path(from, to, |locus, buf| {
        buf.extend(successors(world, locus, None));
    })
}

pub fn directed_path_of_kind(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: RelationshipKindId,
) -> Option<Vec<LocusId>> {
    bfs_path(from, to, |locus, buf| {
        buf.extend(successors(world, locus, Some(kind)));
    })
}
