//! BFS-based graph traversal: shortest path, reachability, and connected
//! components. All operations treat the relationship graph as **undirected**
//! (any relationship connecting two loci counts as a hop regardless of its
//! direction or kind, unless a kind-filtered variant is used).

use std::collections::{BinaryHeap, VecDeque};

use graph_core::{EndpointKey, Endpoints, LocusId, RelationshipId, RelationshipKindId};
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

// ─── Activity-aware traversal ─────────────────────────────────────────────────
//
// This engine assigns every relationship a live `activity()` score that rises
// with causal flow and decays each batch.  These variants prune edges whose
// activity falls below `min_activity` *during* BFS traversal — meaning the
// BFS never crosses dormant edges, and loci reachable only through them are
// excluded from the result entirely.
//
// This is distinct from post-filtering `reachable_from` output: post-filtering
// would still cross the dormant edge and find nodes behind it; here we never
// step onto dormant edges at all.

/// All loci reachable from `start` within `depth` undirected hops, traversing
/// only edges whose `activity()` meets or exceeds `min_activity`.
///
/// Unlike `reachable_from`, dormant edges (activity below the threshold) are
/// pruned *during* BFS — the traversal never crosses them. Loci reachable
/// only through dormant edges are excluded from the result.
///
/// This is the preferred query for **live-signal subgraphs**: in a running
/// simulation, dormant edges carry no active causal flow.
///
/// Returns an empty `Vec` when `depth == 0`.
/// Complexity: O(V_live + E_live) where the subscript denotes the subgraph
/// induced by edges with `activity >= min_activity`.
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
                .filter(|r| r.activity() >= min_activity)
                .map(|r| r.endpoints.other_than(locus)),
        )
    })
}

/// All loci reachable by following directed edges **forward** from `start`,
/// restricted to edges with `activity() >= min_activity`.
///
/// Analogous to `downstream_of` but skips dormant edges during BFS.
/// Returns an empty `Vec` when `depth == 0`.
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
                .filter(|r| r.activity() >= min_activity)
                .filter_map(move |r| match r.endpoints {
                    Endpoints::Directed { from, to } if from == locus => Some(to),
                    Endpoints::Directed { .. } => None,
                    Endpoints::Symmetric { .. } => Some(r.endpoints.other_than(locus)),
                }),
        )
    })
}

/// All loci reachable by following directed edges **backward** from `start`,
/// restricted to edges with `activity() >= min_activity`.
///
/// Analogous to `upstream_of` but skips dormant edges during BFS.
/// Returns an empty `Vec` when `depth == 0`.
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
                .filter(|r| r.activity() >= min_activity)
                .filter_map(move |r| match r.endpoints {
                    Endpoints::Directed { from, to } if to == locus => Some(from),
                    Endpoints::Directed { .. } => None,
                    Endpoints::Symmetric { .. } => Some(r.endpoints.other_than(locus)),
                }),
        )
    })
}

/// BFS shortest path from `from` to `to`, traversing only edges with
/// `activity() >= min_activity`.
///
/// Returns `None` if no path exists through sufficiently active edges.
/// Returns `Some(vec![from])` if `from == to`.
///
/// Use this instead of `path_between` when you want a path through the
/// **live-signal subgraph** rather than the full structural graph.
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
                .filter(|r| r.activity() >= min_activity)
                .map(|r| r.endpoints.other_than(locus)),
        )
    })
}

// ─── Reciprocal / structural topology helpers ────────────────────────────────

/// Find the reciprocal of a directed relationship.
///
/// For a `Directed(A→B)` edge of kind K, returns the `RelationshipId` of the
/// `Directed(B→A)` edge of the same kind if it exists; otherwise `None`.
///
/// Returns `None` for `Symmetric` edges — they are inherently bidirectional
/// and have no separate "reverse" edge.
pub fn reciprocal_of(world: &World, rel_id: RelationshipId) -> Option<RelationshipId> {
    let rel = world.relationships().get(rel_id)?;
    match &rel.endpoints {
        Endpoints::Directed { from, to } => {
            let rev_key = EndpointKey::Directed(*to, *from);
            world.relationships().lookup(&rev_key, rel.kind)
        }
        Endpoints::Symmetric { .. } => None,
    }
}

/// Find all mutual (reciprocal) pairs in the world.
///
/// Returns each `(a, b)` pair once — `(a, b)` and `(b, a)` are not both
/// reported. Pair order within the tuple is unspecified. Only `Directed`
/// edges of the same kind are considered.
pub fn reciprocal_pairs(world: &World) -> Vec<(RelationshipId, RelationshipId)> {
    let mut seen: FxHashSet<RelationshipId> = FxHashSet::default();
    let mut pairs = Vec::new();
    for rel in world.relationships().iter() {
        if seen.contains(&rel.id) {
            continue;
        }
        if let Some(rec_id) = reciprocal_of(world, rel.id) {
            seen.insert(rel.id);
            seen.insert(rec_id);
            pairs.push((rel.id, rec_id));
        }
    }
    pairs
}

/// All loci whose relationship degree (number of edges in any direction) is
/// at least `min_degree`.
///
/// Complexity: O(k) where k is the number of distinct loci that have at
/// least one edge — the underlying `degree_iter` skips loci with no edges.
pub fn hub_loci(world: &World, min_degree: usize) -> Vec<LocusId> {
    world
        .relationships()
        .degree_iter()
        .filter(|(_, degree)| *degree >= min_degree)
        .map(|(locus, _)| locus)
        .collect()
}

/// Immediate undirected neighbors of `locus` — all loci connected by any
/// relationship, regardless of direction or kind.
///
/// This is equivalent to `reachable_from(world, locus, 1)` but avoids the
/// BFS allocation and is more semantically clear for single-hop lookups.
pub fn neighbors_of(world: &World, locus: LocusId) -> Vec<LocusId> {
    neighbors(world, locus, None).collect()
}

/// Immediate undirected neighbors of `locus` via relationships of `kind` only.
pub fn neighbors_of_kind(world: &World, locus: LocusId, kind: RelationshipKindId) -> Vec<LocusId> {
    neighbors(world, locus, Some(kind)).collect()
}

/// All loci that have zero relationships.
///
/// These are structurally "unconnected" — they have never been part of a
/// causal flow the engine observed. Useful for finding unreached loci after
/// a simulation run.
///
/// Complexity: O(V) where V is the number of loci.
pub fn isolated_loci(world: &World) -> Vec<LocusId> {
    world
        .loci()
        .iter()
        .filter(|l| world.relationships().degree(l.id) == 0)
        .map(|l| l.id)
        .collect()
}

/// Returns `true` if the **directed** relationship graph contains at least one
/// directed cycle.
///
/// Only `Directed` relationships are considered; `Symmetric` edges are ignored.
/// Uses iterative three-colour DFS to detect back edges without recursion, so
/// large graphs do not overflow the call stack.
///
/// Complexity: O(V + E_directed).
pub fn has_cycle(world: &World) -> bool {
    // 0 = WHITE (unvisited), 1 = GRAY (in current DFS stack), 2 = BLACK (done)
    let mut color: FxHashMap<LocusId, u8> = FxHashMap::default();

    for locus in world.loci().iter() {
        if color.get(&locus.id).copied().unwrap_or(0) != 0 {
            continue;
        }
        // Stack entries: (node, returning).
        // returning=false → entering for the first time.
        // returning=true  → all successors processed; mark BLACK.
        let mut stack: Vec<(LocusId, bool)> = vec![(locus.id, false)];
        let mut seen: FxHashSet<LocusId> = FxHashSet::default();
        while let Some((node, returning)) = stack.pop() {
            if returning {
                color.insert(node, 2); // BLACK
                continue;
            }
            let c = color.get(&node).copied().unwrap_or(0);
            if c == 2 {
                continue; // Already fully processed
            }
            if c == 1 {
                return true; // Back edge → cycle
            }
            // First visit: mark GRAY, schedule return, push successors.
            color.insert(node, 1);
            stack.push((node, true));
            seen.clear();
            for rel in world.relationships_for_locus(node) {
                if let graph_core::Endpoints::Directed { from, to } = rel.endpoints {
                    if from == node && seen.insert(to) {
                        let tc = color.get(&to).copied().unwrap_or(0);
                        if tc == 1 {
                            return true; // Immediate back edge
                        }
                        if tc == 0 {
                            stack.push((to, false));
                        }
                    }
                }
            }
        }
    }
    false
}

/// All loci that have **no incoming** directed edges but **at least one
/// outgoing** directed edge.
///
/// These are the "origins" of directed flows in the graph. Symmetric edges are
/// not counted as incoming or outgoing.
///
/// Complexity: O(V).
pub fn source_loci(world: &World) -> Vec<LocusId> {
    world
        .loci()
        .iter()
        .filter(|l| world.in_degree(l.id) == 0 && world.out_degree(l.id) > 0)
        .map(|l| l.id)
        .collect()
}

/// All loci that have **no outgoing** directed edges but **at least one
/// incoming** directed edge.
///
/// These are the "terminal sinks" of directed flows in the graph. Symmetric
/// edges are not counted as incoming or outgoing.
///
/// Complexity: O(V).
pub fn sink_loci(world: &World) -> Vec<LocusId> {
    world
        .loci()
        .iter()
        .filter(|l| world.out_degree(l.id) == 0 && world.in_degree(l.id) > 0)
        .map(|l| l.id)
        .collect()
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

// ─── Transitive inference ─────────────────────────────────────────────────────

/// Rule for composing relationship activities along a directed path.
///
/// Used by [`infer_transitive`] to combine edge activities when walking
/// from `from` to `to` through intermediate loci.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransitiveRule {
    /// Multiply activities along the path: `a₁ × a₂ × … × aₙ`.
    ///
    /// Strength weakens with each hop — models trust or reliability chains
    /// where every intermediate link is a bottleneck (0.9 × 0.9 = 0.81).
    Product,
    /// Take the minimum activity along the path.
    ///
    /// The weakest link dominates — conservative estimate of throughput.
    Min,
    /// Arithmetic mean of edge activities.
    ///
    /// All links are treated as equally important. Useful when the chain
    /// length varies and you want scale-invariant comparison.
    Mean,
}

/// Infer the transitive influence of `kind` from `from` to `to` by composing
/// activities along the shortest directed path of that kind.
///
/// Returns `None` when:
/// - No directed path of `kind` exists from `from` to `to`.
/// - `from == to` (trivially connected; no edges to compose).
/// - Any edge on the path has no relationship in the world (should not
///   happen for a valid path, but guards against stale IDs).
///
/// # Example
///
/// ```ignore
/// // A→B TRUST(0.8), B→C TRUST(0.7)
/// let implied = infer_transitive(&world, a, c, TRUST, TransitiveRule::Product);
/// assert!((implied.unwrap() - 0.56).abs() < 1e-5);
/// ```
pub fn infer_transitive(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: graph_core::InfluenceKindId,
    rule: TransitiveRule,
) -> Option<f32> {
    if from == to {
        return None;
    }
    let path = directed_path_of_kind(world, from, to, kind)?;
    if path.len() < 2 {
        return None;
    }

    // Collect edge activities along consecutive locus pairs in the path.
    let activities: Vec<f32> = path
        .windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            world
                .relationships()
                .iter()
                .find(|r| {
                    r.kind == kind
                        && matches!(
                            r.endpoints,
                            graph_core::Endpoints::Directed { from: fa, to: tb }
                            if fa == a && tb == b
                        )
                })
                .map(|r| r.activity())
                .unwrap_or(0.0)
        })
        .collect();

    if activities.is_empty() {
        return None;
    }

    let result = match rule {
        TransitiveRule::Product => activities.iter().product(),
        TransitiveRule::Min => activities
            .iter()
            .cloned()
            .fold(f32::INFINITY, f32::min),
        TransitiveRule::Mean => activities.iter().sum::<f32>() / activities.len() as f32,
    };

    Some(result)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId, Relationship,
        RelationshipKindId, RelationshipLineage, StateVector,
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
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
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
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
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
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
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
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(kind)],
                },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
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
                    change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
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

    // ── reciprocal / hub / isolated ─────────────────────────────────────────

    /// Build: L0 →(k1)→ L1, L1 →(k1)→ L0 (mutual), L0 →(k1)→ L2 (one-way).
    fn reciprocal_world() -> World {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to) in [(0u64, 1u64), (1, 0), (0, 2)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed { from: LocusId(from), to: LocusId(to) },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None, last_touched_by: None,
                    change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
            });
        }
        w
    }

    #[test]
    fn reciprocal_of_finds_reverse_directed_edge() {
        let w = reciprocal_world();
        // L0→L1 should have L1→L0 as reciprocal.
        let rel_01 = w.relationships().relationships_from(LocusId(0))
            .find(|r| r.endpoints.target() == Some(LocusId(1)))
            .map(|r| r.id)
            .unwrap();
        let rec = reciprocal_of(&w, rel_01);
        assert!(rec.is_some());
        let rec_rel = w.relationships().get(rec.unwrap()).unwrap();
        assert_eq!(rec_rel.endpoints.source(), Some(LocusId(1)));
        assert_eq!(rec_rel.endpoints.target(), Some(LocusId(0)));
    }

    #[test]
    fn reciprocal_of_returns_none_for_one_way_edge() {
        let w = reciprocal_world();
        // L0→L2 has no reverse.
        let rel_02 = w.relationships().relationships_from(LocusId(0))
            .find(|r| r.endpoints.target() == Some(LocusId(2)))
            .map(|r| r.id)
            .unwrap();
        assert!(reciprocal_of(&w, rel_02).is_none());
    }

    #[test]
    fn reciprocal_of_returns_none_for_symmetric() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..2 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::Symmetric { a: LocusId(0), b: LocusId(1) },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None, last_touched_by: None,
                change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        assert!(reciprocal_of(&w, id).is_none());
    }

    #[test]
    fn reciprocal_pairs_finds_mutual_pair() {
        let w = reciprocal_world();
        let pairs = reciprocal_pairs(&w);
        assert_eq!(pairs.len(), 1);
        // Both members of the pair should be the L0↔L1 edges.
        let (a, b) = pairs[0];
        let rel_a = w.relationships().get(a).unwrap();
        let rel_b = w.relationships().get(b).unwrap();
        assert!(rel_a.endpoints.involves(LocusId(0)));
        assert!(rel_a.endpoints.involves(LocusId(1)));
        assert!(rel_b.endpoints.involves(LocusId(0)));
        assert!(rel_b.endpoints.involves(LocusId(1)));
    }

    #[test]
    fn hub_loci_filters_by_degree() {
        let w = reciprocal_world();
        // L0 has degree 3 (→L1, ←L1, →L2), L1 has degree 2, L2 has degree 1.
        let hubs = hub_loci(&w, 3);
        assert_eq!(hubs, vec![LocusId(0)]);

        let all_connected = hub_loci(&w, 1);
        assert_eq!(all_connected.len(), 3);

        let none = hub_loci(&w, 10);
        assert!(none.is_empty());
    }

    #[test]
    fn isolated_loci_returns_loci_with_no_edges() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        // Only L0→L1; L2 and L3 are isolated.
        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(1) },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None, last_touched_by: None,
                change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        let mut iso = isolated_loci(&w);
        iso.sort();
        assert_eq!(iso, vec![LocusId(2), LocusId(3)]);
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
                    change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
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

    // ── neighbors_of / neighbors_of_kind ────────────────────────────────────

    #[test]
    fn neighbors_of_returns_immediate_undirected_neighbors() {
        // chain: 0→1→2→3→4
        let w = chain_world(5);
        // Locus 2 connects to both 1 (predecessor) and 3 (successor) undirectedly
        let mut nbrs = neighbors_of(&w, LocusId(2));
        nbrs.sort();
        assert_eq!(nbrs, vec![LocusId(1), LocusId(3)]);
        // Locus 0 is an endpoint — only connects to 1
        let nbrs0 = neighbors_of(&w, LocusId(0));
        assert_eq!(nbrs0, vec![LocusId(1)]);
    }

    #[test]
    fn neighbors_of_kind_filters_to_specific_kind() {
        let lk = LocusKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        let rk1: RelationshipKindId = InfluenceKindId(1);
        let rk2: RelationshipKindId = InfluenceKindId(2);
        let id1 = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: id1, kind: rk1,
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(1) },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage { created_by: None, last_touched_by: None, change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk1)] },
            created_batch: graph_core::BatchId(0), last_decayed_batch: 0,
            metadata: None,
        });
        let id2 = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: id2, kind: rk2,
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(2) },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage { created_by: None, last_touched_by: None, change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk2)] },
            created_batch: graph_core::BatchId(0), last_decayed_batch: 0,
            metadata: None,
        });
        // Kind 1 neighbor of L0 is L1 only
        assert_eq!(neighbors_of_kind(&w, LocusId(0), rk1), vec![LocusId(1)]);
        // Kind 2 neighbor of L0 is L2 only
        assert_eq!(neighbors_of_kind(&w, LocusId(0), rk2), vec![LocusId(2)]);
        // No kind-3 neighbors
        let rk3: RelationshipKindId = InfluenceKindId(3);
        assert!(neighbors_of_kind(&w, LocusId(0), rk3).is_empty());
    }

    // ─── infer_transitive ────────────────────────────────────────────────────

    fn trust_chain_world() -> World {
        // A→B TRUST(0.8), B→C TRUST(0.7)
        use graph_core::{Endpoints, InfluenceKindId, LocusKindId, Relationship, RelationshipKindId, RelationshipLineage, StateVector};
        let lk = LocusKindId(1);
        let trust: RelationshipKindId = InfluenceKindId(10);
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        let id1 = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: id1, kind: trust,
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(1) },
            state: StateVector::from_slice(&[0.8, 0.0]),
            lineage: RelationshipLineage { created_by: None, last_touched_by: None, change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(trust)] },
            created_batch: graph_core::BatchId(0), last_decayed_batch: 0, metadata: None,
        });
        let id2 = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: id2, kind: trust,
            endpoints: Endpoints::Directed { from: LocusId(1), to: LocusId(2) },
            state: StateVector::from_slice(&[0.7, 0.0]),
            lineage: RelationshipLineage { created_by: None, last_touched_by: None, change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(trust)] },
            created_batch: graph_core::BatchId(0), last_decayed_batch: 0, metadata: None,
        });
        w
    }

    #[test]
    fn infer_transitive_product_weakens_with_hops() {
        use graph_core::InfluenceKindId;
        let w = trust_chain_world();
        let trust: graph_core::RelationshipKindId = InfluenceKindId(10);
        let result = infer_transitive(&w, LocusId(0), LocusId(2), trust, TransitiveRule::Product);
        assert!(result.is_some());
        assert!((result.unwrap() - 0.8 * 0.7).abs() < 1e-5);
    }

    #[test]
    fn infer_transitive_min_is_weakest_link() {
        use graph_core::InfluenceKindId;
        let w = trust_chain_world();
        let trust: graph_core::RelationshipKindId = InfluenceKindId(10);
        let result = infer_transitive(&w, LocusId(0), LocusId(2), trust, TransitiveRule::Min);
        assert!((result.unwrap() - 0.7).abs() < 1e-5);
    }

    #[test]
    fn infer_transitive_mean_averages_edges() {
        use graph_core::InfluenceKindId;
        let w = trust_chain_world();
        let trust: graph_core::RelationshipKindId = InfluenceKindId(10);
        let result = infer_transitive(&w, LocusId(0), LocusId(2), trust, TransitiveRule::Mean);
        assert!((result.unwrap() - 0.75).abs() < 1e-5);
    }

    #[test]
    fn infer_transitive_no_path_returns_none() {
        use graph_core::InfluenceKindId;
        let w = trust_chain_world();
        // Reverse direction: no C→A path
        let trust: graph_core::RelationshipKindId = InfluenceKindId(10);
        assert!(infer_transitive(&w, LocusId(2), LocusId(0), trust, TransitiveRule::Product).is_none());
    }

    #[test]
    fn infer_transitive_same_locus_returns_none() {
        use graph_core::InfluenceKindId;
        let w = trust_chain_world();
        let trust: graph_core::RelationshipKindId = InfluenceKindId(10);
        assert!(infer_transitive(&w, LocusId(0), LocusId(0), trust, TransitiveRule::Product).is_none());
    }

    // ── has_cycle ────────────────────────────────────────────────────────────

    #[test]
    fn has_cycle_returns_false_for_dag() {
        // Diamond DAG: 0→1, 0→2, 1→3, 2→3 — no cycle.
        let w = diamond_world();
        assert!(!has_cycle(&w));
    }

    #[test]
    fn has_cycle_returns_true_for_simple_cycle() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        // 0→1→2→0
        for (from, to) in [(0u64, 1), (1, 2), (2, 0)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed { from: LocusId(from), to: LocusId(to) },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None, last_touched_by: None,
                    change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        assert!(has_cycle(&w));
    }

    #[test]
    fn has_cycle_ignores_symmetric_edges() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..2 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        // Symmetric edge 0↔1: not a directed cycle.
        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::Symmetric { a: LocusId(0), b: LocusId(1) },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None, last_touched_by: None,
                change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        assert!(!has_cycle(&w));
    }

    #[test]
    fn has_cycle_empty_world_returns_false() {
        assert!(!has_cycle(&World::new()));
    }

    // ── source_loci / sink_loci ──────────────────────────────────────────────

    #[test]
    fn source_loci_in_chain() {
        // chain_world: 0→1→2→3 (directed)
        let w = chain_world(4);
        let mut sources = source_loci(&w);
        sources.sort();
        assert_eq!(sources, vec![LocusId(0)], "only locus 0 has no incoming edges");
    }

    #[test]
    fn sink_loci_in_chain() {
        let w = chain_world(4);
        let mut sinks = sink_loci(&w);
        sinks.sort();
        assert_eq!(sinks, vec![LocusId(3)], "only locus 3 has no outgoing edges");
    }

    #[test]
    fn source_and_sink_empty_for_cycle() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to) in [(0u64, 1), (1, 2), (2, 0)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed { from: LocusId(from), to: LocusId(to) },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None, last_touched_by: None,
                    change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        assert!(source_loci(&w).is_empty(), "cycle has no pure source");
        assert!(sink_loci(&w).is_empty(), "cycle has no pure sink");
    }

    // ── activity-aware traversal ─────────────────────────────────────────────

    /// Chain: L0 --(0.8)--> L1 --(0.1)--> L2 --(0.8)--> L3.
    /// The middle edge (L1→L2, activity=0.1) acts as a dormant barrier.
    fn activity_chain_world() -> World {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to, activity) in [(0u64, 1u64, 0.8f32), (1, 2, 0.1), (2, 3, 0.8)] {
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
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    #[test]
    fn reachable_from_active_skips_dormant_edges() {
        let w = activity_chain_world();
        // min_activity=0.5: L0→L1 ok (0.8), L1→L2 pruned (0.1).
        // Only L1 is reachable from L0; L2 and L3 are behind the dormant barrier.
        let mut reach = reachable_from_active(&w, LocusId(0), 10, 0.5);
        reach.sort();
        assert_eq!(reach, vec![LocusId(1)], "L2 and L3 should not be reachable");
    }

    #[test]
    fn reachable_from_active_depth_zero_is_empty() {
        let w = activity_chain_world();
        assert!(reachable_from_active(&w, LocusId(0), 0, 0.5).is_empty());
    }

    #[test]
    fn reachable_from_active_zero_threshold_equals_reachable_from() {
        let w = activity_chain_world();
        // min_activity=0.0 — all edges pass, same result as reachable_from.
        let mut active = reachable_from_active(&w, LocusId(0), 10, 0.0);
        let mut standard = reachable_from(&w, LocusId(0), 10);
        active.sort();
        standard.sort();
        assert_eq!(active, standard);
    }

    #[test]
    fn downstream_of_active_skips_dormant_forward_edges() {
        let w = activity_chain_world();
        // From L0 forward, min=0.5: L0→L1(0.8 ✓), L1→L2(0.1 ✗ pruned).
        let mut ds = downstream_of_active(&w, LocusId(0), 10, 0.5);
        ds.sort();
        assert_eq!(ds, vec![LocusId(1)]);
    }

    #[test]
    fn upstream_of_active_skips_dormant_backward_edges() {
        let w = activity_chain_world();
        // From L3 backward, min=0.5: L2→L3(0.8 ✓), L1→L2(0.1 ✗ pruned).
        let mut us = upstream_of_active(&w, LocusId(3), 10, 0.5);
        us.sort();
        assert_eq!(us, vec![LocusId(2)]);
    }

    #[test]
    fn path_between_active_blocked_by_dormant_edge() {
        let w = activity_chain_world();
        // No active path L0→L3 with min=0.5 (L1→L2 blocks it).
        assert!(path_between_active(&w, LocusId(0), LocusId(3), 0.5).is_none());
    }

    #[test]
    fn path_between_active_finds_path_at_zero_threshold() {
        let w = activity_chain_world();
        let path = path_between_active(&w, LocusId(0), LocusId(3), 0.0).unwrap();
        assert_eq!(path.first(), Some(&LocusId(0)));
        assert_eq!(path.last(), Some(&LocusId(3)));
    }

    #[test]
    fn path_between_active_same_locus_returns_singleton() {
        let w = activity_chain_world();
        assert_eq!(
            path_between_active(&w, LocusId(1), LocusId(1), 0.9),
            Some(vec![LocusId(1)])
        );
    }

    #[test]
    fn symmetric_edges_not_counted_as_directed_degree() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..2 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        // Only a symmetric edge: neither source nor sink.
        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::Symmetric { a: LocusId(0), b: LocusId(1) },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None, last_touched_by: None,
                change_count: 1, kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        assert!(source_loci(&w).is_empty());
        assert!(sink_loci(&w).is_empty());
    }
}
