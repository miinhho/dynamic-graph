//! BFS-based graph traversal: shortest path, reachability, and connected
//! components. All operations treat the relationship graph as **undirected**
//! (any relationship connecting two loci counts as a hop regardless of its
//! direction or kind, unless a kind-filtered variant is used).

use std::collections::VecDeque;

use graph_core::{LocusId, RelationshipKindId};
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

// ─── Public API ───────────────────────────────────────────────────────────────

/// BFS shortest path from `from` to `to` over all relationships (undirected).
///
/// Returns `Some(path)` where `path[0] == from` and `path.last() == Some(&to)`,
/// or `None` if no path exists. Returns `Some(vec![from])` if `from == to`.
///
/// Complexity: O(V + E) over the visited subgraph.
pub fn path_between(world: &World, from: LocusId, to: LocusId) -> Option<Vec<LocusId>> {
    bfs_path(from, to, |locus, buf| {
        buf.extend(neighbors(world, locus, None))
    })
}

/// BFS shortest path restricted to relationships of `kind`.
///
/// Same semantics as `path_between` but only traverses edges of the given kind.
pub fn path_between_of_kind(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: RelationshipKindId,
) -> Option<Vec<LocusId>> {
    bfs_path(from, to, |locus, buf| {
        buf.extend(neighbors(world, locus, Some(kind)))
    })
}

/// All loci reachable from `start` within `depth` undirected relationship
/// hops. Does not include `start` itself.
///
/// Returns an empty `Vec` when `depth == 0`.
/// Complexity: O(V + E) over the reachable subgraph.
pub fn reachable_from(world: &World, start: LocusId, depth: usize) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(neighbors(world, locus, None))
    })
}

/// All loci reachable from `start` within `depth` hops, restricted to
/// relationships of `kind`. Does not include `start` itself.
pub fn reachable_from_of_kind(
    world: &World,
    start: LocusId,
    depth: usize,
    kind: RelationshipKindId,
) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(neighbors(world, locus, Some(kind)))
    })
}

/// Weakly connected components of the relationship graph.
///
/// Uses BFS over the undirected view of all relationships. Loci with no
/// relationships appear as singleton components.
///
/// Returns a `Vec` of components, each component a `Vec<LocusId>`.
/// Component order and member order within each component are unspecified.
/// Complexity: O(V + E).
pub fn connected_components(world: &World) -> Vec<Vec<LocusId>> {
    let all_loci: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();
    bfs_components(&all_loci, |locus, buf| {
        buf.extend(neighbors(world, locus, None))
    })
}

/// Weakly connected components restricted to relationships of `kind`.
///
/// Loci with no edges of that kind form singleton components.
pub fn connected_components_of_kind(
    world: &World,
    kind: RelationshipKindId,
) -> Vec<Vec<LocusId>> {
    let all_loci: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();
    bfs_components(&all_loci, |locus, buf| {
        buf.extend(neighbors(world, locus, Some(kind)))
    })
}

// ─── Internal neighbor iterator ───────────────────────────────────────────────

/// Undirected neighbors of `locus` via relationships of `kind` (or all kinds
/// when `kind` is `None`). Yields the other endpoint for each matching relationship.
fn neighbors(
    world: &World,
    locus: LocusId,
    kind: Option<RelationshipKindId>,
) -> impl Iterator<Item = LocusId> + '_ {
    world
        .relationships_for_locus(locus)
        .filter(move |r| kind.is_none_or(|k| r.kind == k))
        .map(move |r| r.endpoints.other_than(locus))
}

// ─── BFS primitives ───────────────────────────────────────────────────────────

/// BFS shortest path using a caller-supplied neighbor function.
///
/// `for_neighbors(locus, buf)` must extend `buf` with all undirected neighbors
/// of `locus`. `buf` is cleared before each call; the same allocation is
/// reused across all nodes to avoid per-node heap churn.
fn bfs_path(
    from: LocusId,
    to: LocusId,
    mut for_neighbors: impl FnMut(LocusId, &mut Vec<LocusId>),
) -> Option<Vec<LocusId>> {
    if from == to {
        return Some(vec![from]);
    }
    let mut prev: FxHashMap<LocusId, LocusId> = FxHashMap::default();
    let mut queue: VecDeque<LocusId> = VecDeque::new();
    let mut buf: Vec<LocusId> = Vec::new();
    prev.insert(from, from);
    queue.push_back(from);

    while let Some(current) = queue.pop_front() {
        buf.clear();
        for_neighbors(current, &mut buf);
        for &neighbor in &buf {
            if prev.contains_key(&neighbor) {
                continue;
            }
            prev.insert(neighbor, current);
            if neighbor == to {
                let mut path = vec![to];
                let mut node = to;
                while node != from {
                    node = prev[&node];
                    path.push(node);
                }
                path.reverse();
                return Some(path);
            }
            queue.push_back(neighbor);
        }
    }
    None
}

/// BFS reachability within `depth` hops using a caller-supplied neighbor
/// function. Returns all reachable loci (excluding `start`).
fn bfs_reachable(
    start: LocusId,
    depth: usize,
    mut for_neighbors: impl FnMut(LocusId, &mut Vec<LocusId>),
) -> Vec<LocusId> {
    if depth == 0 {
        return Vec::new();
    }
    let mut dist: FxHashMap<LocusId, usize> = FxHashMap::default();
    let mut queue: VecDeque<LocusId> = VecDeque::new();
    let mut buf: Vec<LocusId> = Vec::new();
    dist.insert(start, 0);
    queue.push_back(start);

    let mut result = Vec::new();
    while let Some(current) = queue.pop_front() {
        let d = dist[&current];
        if d >= depth {
            continue;
        }
        buf.clear();
        for_neighbors(current, &mut buf);
        for &neighbor in &buf {
            if dist.contains_key(&neighbor) {
                continue;
            }
            dist.insert(neighbor, d + 1);
            result.push(neighbor);
            queue.push_back(neighbor);
        }
    }
    result
}

/// BFS weakly connected components. Each isolated locus becomes its own
/// singleton component.
fn bfs_components(
    all_loci: &[LocusId],
    mut for_neighbors: impl FnMut(LocusId, &mut Vec<LocusId>),
) -> Vec<Vec<LocusId>> {
    let mut visited: FxHashSet<LocusId> = FxHashSet::default();
    let mut components: Vec<Vec<LocusId>> = Vec::new();
    let mut buf: Vec<LocusId> = Vec::new();

    for &seed in all_loci {
        if visited.contains(&seed) {
            continue;
        }
        let mut component = Vec::new();
        let mut queue: VecDeque<LocusId> = VecDeque::new();
        visited.insert(seed);
        queue.push_back(seed);
        while let Some(current) = queue.pop_front() {
            component.push(current);
            buf.clear();
            for_neighbors(current, &mut buf);
            for &neighbor in &buf {
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Endpoints, InfluenceKindId, Locus, LocusKindId, Relationship, RelationshipKindId,
        RelationshipLineage, StateVector,
    };

    fn chain_world(n: u64) -> World {
        let kind = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0..n {
            w.insert_locus(Locus::new(LocusId(i), kind, StateVector::zeros(1)));
        }
        for i in 0..(n - 1) {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed { from: LocusId(i), to: LocusId(i + 1) },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: vec![rk],
                },
                last_decayed_batch: 0,
            });
        }
        w
    }

    fn two_chain_world() -> World {
        let kind = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for id in [0u64, 1, 2, 10, 11] {
            w.insert_locus(Locus::new(LocusId(id), kind, StateVector::zeros(1)));
        }
        for (from, to) in [(0u64, 1), (1, 2), (10, 11)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed { from: LocusId(from), to: LocusId(to) },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: vec![rk],
                },
                last_decayed_batch: 0,
            });
        }
        w
    }

    #[test]
    fn path_between_same_locus_returns_singleton() {
        let w = chain_world(4);
        assert_eq!(path_between(&w, LocusId(2), LocusId(2)), Some(vec![LocusId(2)]));
    }

    #[test]
    fn path_between_finds_shortest_path() {
        let w = chain_world(5);
        let path = path_between(&w, LocusId(0), LocusId(4)).unwrap();
        assert_eq!(path, vec![LocusId(0), LocusId(1), LocusId(2), LocusId(3), LocusId(4)]);
    }

    #[test]
    fn path_between_returns_none_for_disconnected_loci() {
        let mut w = chain_world(3);
        w.insert_locus(Locus::new(LocusId(99), LocusKindId(1), StateVector::zeros(1)));
        assert!(path_between(&w, LocusId(0), LocusId(99)).is_none());
    }

    #[test]
    fn reachable_from_depth_1_returns_direct_neighbors() {
        let w = chain_world(5);
        let mut reached = reachable_from(&w, LocusId(2), 1);
        reached.sort();
        assert_eq!(reached, vec![LocusId(1), LocusId(3)]);
    }

    #[test]
    fn reachable_from_depth_0_is_empty() {
        let w = chain_world(4);
        assert!(reachable_from(&w, LocusId(0), 0).is_empty());
    }

    #[test]
    fn connected_components_counts_correctly() {
        let w = two_chain_world();
        let comps = connected_components(&w);
        assert_eq!(comps.len(), 2);
        let mut sizes: Vec<usize> = comps.iter().map(Vec::len).collect();
        sizes.sort();
        assert_eq!(sizes, vec![2, 3]);
    }

    #[test]
    fn connected_components_of_kind_filters_by_kind() {
        let kind_a: RelationshipKindId = InfluenceKindId(1);
        let kind_b: RelationshipKindId = InfluenceKindId(2);
        let lk = LocusKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        // 0 -kind_a-> 1, 2 -kind_b-> 3
        for (from, to, kind) in [(0u64, 1, kind_a), (2, 3, kind_b)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind,
                endpoints: Endpoints::Directed { from: LocusId(from), to: LocusId(to) },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: vec![kind],
                },
                last_decayed_batch: 0,
            });
        }
        // kind_a view: {0,1} connected, {2} and {3} isolated
        let comps_a = connected_components_of_kind(&w, kind_a);
        let mut sizes_a: Vec<usize> = comps_a.iter().map(Vec::len).collect();
        sizes_a.sort();
        assert_eq!(sizes_a, vec![1, 1, 2]);

        // kind_b view: {0} and {1} isolated, {2,3} connected
        let comps_b = connected_components_of_kind(&w, kind_b);
        let mut sizes_b: Vec<usize> = comps_b.iter().map(Vec::len).collect();
        sizes_b.sort();
        assert_eq!(sizes_b, vec![1, 1, 2]);
    }
}
