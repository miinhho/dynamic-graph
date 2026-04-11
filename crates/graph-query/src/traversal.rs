//! BFS-based graph traversal: shortest path, reachability, and connected
//! components. All operations treat the relationship graph as **undirected**
//! (any relationship connecting two loci counts as a hop regardless of its
//! direction or kind, unless a kind-filtered variant is used).

use std::collections::{BinaryHeap, VecDeque};

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

/// Dijkstra shortest path from `from` to `to`, preferring high-activity edges.
///
/// Edge cost is `1.0 / activity` (activity clamped to `1e-6` to avoid
/// infinite cost on dormant edges). The path returned minimises total cost —
/// equivalently, it traverses the edges with the highest combined activity.
///
/// This differs from `path_between` (which minimises hop count) by finding
/// the **live-wire path**: the route most actively carrying signal, which is
/// often the probable causal path in a dynamic graph.
///
/// Like `path_between`, traversal is **undirected** (both directions of a
/// `Directed` edge are considered as hops). Returns `None` if no path exists.
/// Returns `Some(vec![from])` if `from == to`.
///
/// Complexity: O((V + E) log V) over the visited subgraph.
pub fn strongest_path(world: &World, from: LocusId, to: LocusId) -> Option<Vec<LocusId>> {
    if from == to {
        return Some(vec![from]);
    }
    dijkstra_path(from, to, |locus, buf| {
        buf.extend(
            world
                .relationships_for_locus(locus)
                .map(|rel| (rel.endpoints.other_than(locus), 1.0 / rel.activity().max(1e-6))),
        );
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

/// All loci reachable from `start` within `depth` undirected relationship
/// hops that satisfy `pred`. Does not include `start` itself.
///
/// The BFS traverses through **all** loci (matching or not), so a
/// non-matching locus can act as a bridge to a matching one further away.
/// Only loci for which `pred(id)` returns `true` are included in the result.
///
/// Prefer this over `reachable_from` + post-filter: both have the same
/// complexity but this avoids allocating a separate result for the full
/// reachable set before filtering.
///
/// Returns an empty `Vec` when `depth == 0`.
/// Complexity: O(V + E) over the reachable subgraph.
pub fn reachable_matching(
    world: &World,
    start: LocusId,
    depth: usize,
    pred: impl Fn(LocusId) -> bool,
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
        buf.extend(neighbors(world, current, None));
        for &neighbor in &buf {
            if dist.contains_key(&neighbor) {
                continue;
            }
            dist.insert(neighbor, d + 1);
            if pred(neighbor) {
                result.push(neighbor);
            }
            queue.push_back(neighbor);
        }
    }
    result
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

// ─── Directed traversal ───────────────────────────────────────────────────────

/// All loci reachable from `start` by following relationship edges in the
/// **forward** direction within `depth` hops. Does not include `start` itself.
///
/// - `Directed { from, to }`: only traversed when `from == current` (outward).
/// - `Symmetric { a, b }`: traversed in both directions (same as undirected).
///
/// Returns an empty `Vec` when `depth == 0`.
/// Complexity: O(V + E) over the reachable subgraph.
pub fn downstream_of(world: &World, start: LocusId, depth: usize) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(successors(world, locus, None))
    })
}

/// All loci reachable from `start` by following relationship edges in the
/// **reverse** direction within `depth` hops. Does not include `start` itself.
///
/// - `Directed { from, to }`: only traversed when `to == current` (inward → `from`).
/// - `Symmetric { a, b }`: traversed in both directions (same as undirected).
///
/// Returns an empty `Vec` when `depth == 0`.
/// Complexity: O(V + E) over the reachable subgraph.
pub fn upstream_of(world: &World, start: LocusId, depth: usize) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(predecessors(world, locus, None))
    })
}

/// Directed variants of `downstream_of` and `upstream_of` restricted to
/// relationships of `kind`.
pub fn downstream_of_kind(world: &World, start: LocusId, depth: usize, kind: RelationshipKindId) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(successors(world, locus, Some(kind)))
    })
}

/// Upstream traversal restricted to relationships of `kind`.
pub fn upstream_of_kind(world: &World, start: LocusId, depth: usize, kind: RelationshipKindId) -> Vec<LocusId> {
    bfs_reachable(start, depth, |locus, buf| {
        buf.extend(predecessors(world, locus, Some(kind)))
    })
}

/// BFS shortest path from `from` to `to` following edges in the **forward**
/// direction only.
///
/// - `Directed { from, to }`: traversed outward only.
/// - `Symmetric`: traversed both ways.
///
/// Returns `None` if no directed path exists.
pub fn directed_path(world: &World, from: LocusId, to: LocusId) -> Option<Vec<LocusId>> {
    bfs_path(from, to, |locus, buf| {
        buf.extend(successors(world, locus, None))
    })
}

/// Directed path restricted to relationships of `kind`.
pub fn directed_path_of_kind(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: RelationshipKindId,
) -> Option<Vec<LocusId>> {
    bfs_path(from, to, |locus, buf| {
        buf.extend(successors(world, locus, Some(kind)))
    })
}

// ─── Internal neighbor iterators ─────────────────────────────────────────────

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

/// Forward (downstream) neighbors of `locus`:
/// - `Directed { from, to }` where `from == locus` → yields `to`.
/// - `Symmetric { a, b }` → yields the other endpoint regardless of direction.
fn successors(
    world: &World,
    locus: LocusId,
    kind: Option<RelationshipKindId>,
) -> impl Iterator<Item = LocusId> + '_ {
    use graph_core::Endpoints;
    world
        .relationships_for_locus(locus)
        .filter(move |r| kind.is_none_or(|k| r.kind == k))
        .filter_map(move |r| match r.endpoints {
            Endpoints::Directed { from, to } if from == locus => Some(to),
            Endpoints::Directed { .. } => None,
            Endpoints::Symmetric { .. } => Some(r.endpoints.other_than(locus)),
        })
}

/// Backward (upstream) neighbors of `locus`:
/// - `Directed { from, to }` where `to == locus` → yields `from`.
/// - `Symmetric { a, b }` → yields the other endpoint regardless of direction.
fn predecessors(
    world: &World,
    locus: LocusId,
    kind: Option<RelationshipKindId>,
) -> impl Iterator<Item = LocusId> + '_ {
    use graph_core::Endpoints;
    world
        .relationships_for_locus(locus)
        .filter(move |r| kind.is_none_or(|k| r.kind == k))
        .filter_map(move |r| match r.endpoints {
            Endpoints::Directed { from, to } if to == locus => Some(from),
            Endpoints::Directed { .. } => None,
            Endpoints::Symmetric { .. } => Some(r.endpoints.other_than(locus)),
        })
}

// ─── Shared path reconstruction ──────────────────────────────────────────────

/// Reconstruct the path from `from` to `to` using the `prev` map built by
/// either BFS or Dijkstra.
///
/// # Invariant
/// `to` must be reachable from `from` via the `prev` map — every node in the
/// chain must have an entry until we reach `from`. Callers (`bfs_path`,
/// `dijkstra_path`) only call this after confirming `to` was reached.
fn reconstruct_path(
    from: LocusId,
    to: LocusId,
    prev: &FxHashMap<LocusId, LocusId>,
) -> Vec<LocusId> {
    let mut path = vec![to];
    let mut node = to;
    while node != from {
        debug_assert!(
            prev.contains_key(&node),
            "reconstruct_path: node {node:?} not in prev map — `to` must be reachable from `from`"
        );
        node = prev[&node];
        path.push(node);
    }
    path.reverse();
    path
}

// ─── Dijkstra primitive ───────────────────────────────────────────────────────

/// Dijkstra shortest path with a caller-supplied weighted-neighbor function.
///
/// `for_neighbors(locus, buf)` must clear `buf` and extend it with
/// `(neighbor, cost)` pairs. `buf` is reused across calls to avoid per-node
/// heap allocation. Lower cost = more preferred.
///
/// `from != to` must hold; callers handle the trivial case before calling.
fn dijkstra_path(
    from: LocusId,
    to: LocusId,
    mut for_neighbors: impl FnMut(LocusId, &mut Vec<(LocusId, f32)>),
) -> Option<Vec<LocusId>> {
    #[derive(PartialEq)]
    struct Entry(f32, LocusId);
    impl Eq for Entry {}
    impl PartialOrd for Entry {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Entry {
        // Reverse for min-heap (BinaryHeap is max-heap by default).
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            other.0.total_cmp(&self.0)
        }
    }

    let mut dist: FxHashMap<LocusId, f32> = FxHashMap::default();
    let mut prev: FxHashMap<LocusId, LocusId> = FxHashMap::default();
    let mut heap: BinaryHeap<Entry> = BinaryHeap::new();
    let mut buf: Vec<(LocusId, f32)> = Vec::new();

    dist.insert(from, 0.0);
    heap.push(Entry(0.0, from));

    while let Some(Entry(cost, current)) = heap.pop() {
        if current == to {
            return Some(reconstruct_path(from, to, &prev));
        }
        if dist.get(&current).is_some_and(|&d| cost > d) {
            continue;
        }
        buf.clear();
        for_neighbors(current, &mut buf);
        for &(neighbor, edge_cost) in &buf {
            let new_cost = cost + edge_cost;
            if dist.get(&neighbor).is_none_or(|&d| new_cost < d) {
                dist.insert(neighbor, new_cost);
                prev.insert(neighbor, current);
                heap.push(Entry(new_cost, neighbor));
            }
        }
    }
    None
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
                return Some(reconstruct_path(from, to, &prev));
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

    // ── strongest_path ───────────────────────────────────────────────────────

    /// Build a world with two paths from locus 0 to locus 3:
    /// - Short path:  0 --(activity=0.1)--> 1 --(activity=0.1)--> 3   (low activity)
    /// - Strong path: 0 --(activity=5.0)--> 2 --(activity=5.0)--> 3   (high activity)
    fn two_path_world() -> World {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to, activity) in [(0u64, 1u64, 0.1f32), (1, 3, 0.1), (0, 2, 5.0), (2, 3, 5.0)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed { from: LocusId(from), to: LocusId(to) },
                state: StateVector::from_slice(&[activity, 0.0]),
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
    fn strongest_path_same_locus_returns_singleton() {
        let w = chain_world(4);
        assert_eq!(strongest_path(&w, LocusId(1), LocusId(1)), Some(vec![LocusId(1)]));
    }

    #[test]
    fn strongest_path_returns_none_for_disconnected() {
        let mut w = chain_world(3);
        w.insert_locus(Locus::new(LocusId(99), LocusKindId(1), StateVector::zeros(1)));
        assert!(strongest_path(&w, LocusId(0), LocusId(99)).is_none());
    }

    #[test]
    fn strongest_path_prefers_high_activity_over_short_hops() {
        let w = two_path_world();
        // path_between would return [0, 1, 3] (2 hops).
        // strongest_path should return [0, 2, 3] (through the high-activity edges).
        let path = strongest_path(&w, LocusId(0), LocusId(3)).unwrap();
        assert_eq!(
            path,
            vec![LocusId(0), LocusId(2), LocusId(3)],
            "strongest_path should prefer high-activity edges"
        );
    }

    #[test]
    fn path_between_chooses_short_path_over_strong_path() {
        // Verify the contrast: path_between returns the short path.
        let w = two_path_world();
        let path = path_between(&w, LocusId(0), LocusId(3)).unwrap();
        assert_eq!(
            path,
            vec![LocusId(0), LocusId(1), LocusId(3)],
            "path_between should return the shortest hop path"
        );
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

    // ── Directed traversal ──────────────────────────────────────────────────

    /// Build a diamond graph: 0 → 1, 0 → 2, 1 → 3, 2 → 3 (all directed).
    fn diamond_world() -> World {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to) in [(0u64, 1), (0, 2), (1, 3), (2, 3)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed { from: LocusId(from), to: LocusId(to) },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None, last_touched_by: None,
                    change_count: 1, kinds_observed: vec![rk],
                },
                last_decayed_batch: 0,
            });
        }
        w
    }

    #[test]
    fn downstream_of_follows_directed_edges_forward() {
        let w = diamond_world();
        // From 0: can reach 1, 2, 3 downstream.
        let mut reach = downstream_of(&w, LocusId(0), 3);
        reach.sort();
        assert_eq!(reach, vec![LocusId(1), LocusId(2), LocusId(3)]);
    }

    #[test]
    fn downstream_of_does_not_traverse_reverse_directed_edges() {
        let w = diamond_world();
        // From 3: no outgoing directed edges → nothing downstream.
        assert!(downstream_of(&w, LocusId(3), 3).is_empty());
    }

    #[test]
    fn upstream_of_follows_directed_edges_backward() {
        let w = diamond_world();
        // From 3: upstream are 1, 2, 0.
        let mut reach = upstream_of(&w, LocusId(3), 3);
        reach.sort();
        assert_eq!(reach, vec![LocusId(0), LocusId(1), LocusId(2)]);
    }

    #[test]
    fn upstream_of_does_not_traverse_forward_directed_edges() {
        let w = diamond_world();
        // From 0: no incoming directed edges → nothing upstream.
        assert!(upstream_of(&w, LocusId(0), 3).is_empty());
    }

    #[test]
    fn directed_path_follows_direction() {
        let w = diamond_world();
        // 0 → 3 exists via 1 or 2 (two hops).
        let path = directed_path(&w, LocusId(0), LocusId(3)).unwrap();
        assert_eq!(path.first(), Some(&LocusId(0)));
        assert_eq!(path.last(), Some(&LocusId(3)));
        assert_eq!(path.len(), 3, "two-hop path through the diamond");
    }

    #[test]
    fn directed_path_returns_none_against_direction() {
        let w = diamond_world();
        // 3 → 0 is not reachable via directed edges.
        assert!(directed_path(&w, LocusId(3), LocusId(0)).is_none());
    }

    #[test]
    fn symmetric_edges_count_in_directed_traversal_both_ways() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        // 0 ↔ 1 (symmetric), 1 → 2 (directed)
        for (a, b, sym) in [(0u64, 1u64, true), (1, 2, false)] {
            let id = w.relationships_mut().mint_id();
            let endpoints = if sym {
                Endpoints::Symmetric { a: LocusId(a), b: LocusId(b) }
            } else {
                Endpoints::Directed { from: LocusId(a), to: LocusId(b) }
            };
            w.relationships_mut().insert(Relationship {
                id, kind: rk, endpoints,
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None, last_touched_by: None,
                    change_count: 1, kinds_observed: vec![rk],
                },
                last_decayed_batch: 0,
            });
        }
        // downstream from 0: crosses symmetric 0↔1, then directed 1→2 → reaches {1, 2}
        let mut ds = downstream_of(&w, LocusId(0), 3);
        ds.sort();
        assert_eq!(ds, vec![LocusId(1), LocusId(2)]);
        // upstream from 2: crosses directed 1→2 backward, then symmetric 0↔1 → reaches {1, 0}
        let mut us = upstream_of(&w, LocusId(2), 3);
        us.sort();
        assert_eq!(us, vec![LocusId(0), LocusId(1)]);
    }
}
