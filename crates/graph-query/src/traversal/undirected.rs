use graph_core::{LocusId, RelationshipKindId};
use graph_world::World;

use super::neighbors::neighbors;
use super::primitives::{bfs_components, bfs_path, bfs_reachable, dijkstra_path};

pub fn path_between(world: &World, from: LocusId, to: LocusId) -> Option<Vec<LocusId>> {
    bfs_path(from, to, |locus, buf| {
        buf.extend(neighbors(world, locus, None))
    })
}

pub fn strongest_path(world: &World, from: LocusId, to: LocusId) -> Option<Vec<LocusId>> {
    if from == to {
        return Some(vec![from]);
    }
    dijkstra_path(from, to, |locus, buf| {
        buf.extend(world.relationships_for_locus(locus).map(|relationship| {
            (
                relationship.endpoints.other_than(locus),
                1.0 / relationship.activity().max(1e-6),
            )
        }));
    })
}

pub fn path_between_of_kind(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: RelationshipKindId,
) -> Option<Vec<LocusId>> {
    bfs_path(from, to, |locus, buf| {
        buf.extend(neighbors(world, locus, Some(kind)));
    })
}

pub fn reachable_from(world: &World, start: LocusId, depth: usize) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(neighbors(world, locus, None));
    })
}

pub fn reachable_from_of_kind(
    world: &World,
    start: LocusId,
    depth: usize,
    kind: RelationshipKindId,
) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(neighbors(world, locus, Some(kind)));
    })
}

pub fn reachable_matching(
    world: &World,
    start: LocusId,
    depth: usize,
    pred: impl Fn(LocusId) -> bool,
) -> Vec<LocusId> {
    if depth == 0 {
        return Vec::new();
    }

    let mut dist = rustc_hash::FxHashMap::default();
    let mut queue = std::collections::VecDeque::new();
    let mut buf = Vec::new();
    dist.insert(start, 0usize);
    queue.push_back(start);

    let mut result = Vec::new();
    while let Some(current) = queue.pop_front() {
        let current_depth = dist[&current];
        if current_depth >= depth {
            continue;
        }
        buf.clear();
        buf.extend(neighbors(world, current, None));
        for &neighbor in &buf {
            if dist.contains_key(&neighbor) {
                continue;
            }
            dist.insert(neighbor, current_depth + 1);
            if pred(neighbor) {
                result.push(neighbor);
            }
            queue.push_back(neighbor);
        }
    }
    result
}

pub fn connected_components(world: &World) -> Vec<Vec<LocusId>> {
    let all_loci: Vec<LocusId> = world.loci().iter().map(|locus| locus.id).collect();
    bfs_components(&all_loci, |locus, buf| {
        buf.extend(neighbors(world, locus, None))
    })
}

pub fn connected_components_of_kind(world: &World, kind: RelationshipKindId) -> Vec<Vec<LocusId>> {
    let all_loci: Vec<LocusId> = world.loci().iter().map(|locus| locus.id).collect();
    bfs_components(&all_loci, |locus, buf| {
        buf.extend(neighbors(world, locus, Some(kind)));
    })
}
