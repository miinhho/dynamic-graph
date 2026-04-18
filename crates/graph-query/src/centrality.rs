//! Analytical centrality functions: Burt's structural constraint, structural
//! balance (Cartwright & Harary), Newman–Girvan modularity, betweenness
//! centrality (Brandes), and harmonic closeness centrality.
//!
//! All functions take `&World` and are read-only.

use std::collections::VecDeque;

use graph_core::LocusId;
use graph_world::World;
use rustc_hash::{FxHashMap, FxHashSet};

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Build a map of `neighbor → total strength` for a given locus.
///
/// Sums `rel.strength()` over all relationships touching `locus` and groups
/// by the other endpoint. Multiple relationships between the same pair are
/// accumulated.
fn neighbor_strengths(world: &World, locus: LocusId) -> FxHashMap<LocusId, f32> {
    let mut map: FxHashMap<LocusId, f32> = FxHashMap::default();
    for rel in world.relationships_for_locus(locus) {
        let other = rel.endpoints.other_than(locus);
        *map.entry(other).or_insert(0.0) += rel.strength();
    }
    map
}

// ─── Burt's Structural Constraint ─────────────────────────────────────────────

/// Compute Burt's structural constraint for `locus`.
///
/// Low constraint indicates the locus bridges structurally disconnected groups
/// (a structural hole); high constraint means the locus's contacts are
/// redundantly connected to each other.
///
/// Formula (Burt 1992):
/// ```text
/// p_ij = w_ij / Σ_k w_ik
/// c_ij = (p_ij + Σ_{q≠i,j} p_iq * p_qj)²
/// C_i  = Σ_j c_ij
/// ```
///
/// Uses `rel.strength()` (= activity + weight) as edge weight. Multiple
/// relationships between the same pair are summed before computing proportions.
///
/// Returns `None` when `locus` has no relationships (total strength = 0).
pub fn structural_constraint(world: &World, locus: LocusId) -> Option<f32> {
    let w_i = neighbor_strengths(world, locus);
    let total_i: f32 = w_i.values().sum();
    if total_i == 0.0 || w_i.is_empty() {
        return None;
    }

    // p_ij for each neighbor j
    let p: FxHashMap<LocusId, f32> = w_i.iter().map(|(&j, &wij)| (j, wij / total_i)).collect();

    // Pre-compute each neighbor q's strength map so we can compute p_qj efficiently.
    // For each q in i's neighborhood, we need: p_qj = w_qj / Σ_k w_qk
    let q_strengths: FxHashMap<LocusId, (FxHashMap<LocusId, f32>, f32)> = p
        .keys()
        .map(|&q| {
            let wq = neighbor_strengths(world, q);
            let total_q: f32 = wq.values().sum();
            (q, (wq, total_q))
        })
        .collect();

    let mut constraint = 0.0f32;

    for (&j, &p_ij) in &p {
        // Indirect term: Σ_{q≠i,j} p_iq * p_qj
        // q iterates over i's neighbors (excluding j itself)
        // p_qj = w_qj / Σ_k w_qk  (q's proportion invested in j)
        let indirect: f32 = p
            .iter()
            .filter(|&(&q, _)| q != j)
            .map(|(&q, &p_iq)| {
                let p_qj = q_strengths
                    .get(&q)
                    .map(|(wq, total_q)| {
                        if *total_q > 0.0 {
                            wq.get(&j).copied().unwrap_or(0.0) / total_q
                        } else {
                            0.0
                        }
                    })
                    .unwrap_or(0.0);
                p_iq * p_qj
            })
            .sum();

        let c_ij = (p_ij + indirect).powi(2);
        constraint += c_ij;
    }

    Some(constraint)
}

/// Compute structural constraint for every locus in the world.
///
/// Returns `(LocusId, constraint)` pairs sorted in **ascending** order of
/// constraint — loci with the lowest constraint (greatest brokerage value /
/// structural hole position) appear first.
///
/// Loci with no relationships are omitted.
pub fn all_constraints(world: &World) -> Vec<(LocusId, f32)> {
    let mut result: Vec<(LocusId, f32)> = world
        .loci()
        .iter()
        .filter_map(|l| structural_constraint(world, l.id).map(|c| (l.id, c)))
        .collect();
    result.sort_by(|a, b| a.1.total_cmp(&b.1));
    result
}

/// Compute Burt's effective network size for `locus`.
///
/// Measures how many non-redundant contacts `locus` has. A clique of k
/// contacts scores near 1 (all redundant); k contacts in separate components
/// scores near k.
///
/// Formula (Burt 1992, eq. 2.4):
/// ```text
/// ENS_i = degree_i - Σ_j p_ij * Σ_{q≠j} p_jq
/// ```
///
/// Returns `0.0` when `locus` has no relationships.
pub fn effective_network_size(world: &World, locus: LocusId) -> f32 {
    let w_i = neighbor_strengths(world, locus);
    let degree = w_i.len() as f32;
    let total_i: f32 = w_i.values().sum();
    if total_i == 0.0 || w_i.is_empty() {
        return 0.0;
    }

    // Build the set of i's neighbors for quick lookup
    let i_neighbors: FxHashSet<LocusId> = w_i.keys().copied().collect();

    let redundancy: f32 = w_i
        .iter()
        .map(|(&j, &wij)| {
            let p_ij = wij / total_i;

            // Σ_{q≠j, q ∈ N_i} p_jq: j's proportional strength going to i's
            // other contacts (measures how redundant j is with i's network).
            // p_jq = w_jq / Σ_k w_jk  (from j's perspective)
            let w_j = neighbor_strengths(world, j);
            let total_j: f32 = w_j.values().sum();
            let sum_p_jq: f32 = if total_j > 0.0 {
                w_j.iter()
                    .filter(|&(&q, _)| q != j && q != locus && i_neighbors.contains(&q))
                    .map(|(_, &wjq)| wjq / total_j)
                    .sum()
            } else {
                0.0
            };

            p_ij * sum_p_jq
        })
        .sum();

    degree - redundancy
}

// ─── Structural Balance ───────────────────────────────────────────────────────

/// Structural balance classification of a triangle.
///
/// Based on Cartwright & Harary (1956): the sign of each edge is determined
/// by `rel.strength()` relative to a caller-supplied `threshold`.
/// A triangle is **balanced** if the product of its three signs is positive
/// (+++ or +−−), and **unstable** otherwise (+−− counts balanced, +++ balanced;
/// see [sign rule](https://en.wikipedia.org/wiki/Balance_theory)).
#[derive(Debug, Clone, PartialEq)]
pub enum TriangleBalance {
    /// Product of signs = +1. Triangle is structurally stable.
    Balanced,
    /// Product of signs = −1. Triangle is structurally unstable.
    Unstable,
}

/// Classify the triangle formed by loci `a`, `b`, `c`.
///
/// Returns `None` if any of the three edges `(a,b)`, `(b,c)`, `(a,c)` does
/// not exist. If multiple relationships exist between a pair, their strengths
/// are summed before sign determination.
///
/// `threshold`: edges with combined strength `> threshold` are positive;
/// strength `<= threshold` is negative.
pub fn triangle_balance(
    world: &World,
    a: LocusId,
    b: LocusId,
    c: LocusId,
    threshold: f32,
) -> Option<TriangleBalance> {
    let s_ab = edge_strength_between(world, a, b)?;
    let s_bc = edge_strength_between(world, b, c)?;
    let s_ac = edge_strength_between(world, a, c)?;

    let sign = |s: f32| if s > threshold { 1i32 } else { -1i32 };
    let product = sign(s_ab) * sign(s_bc) * sign(s_ac);

    if product > 0 {
        Some(TriangleBalance::Balanced)
    } else {
        Some(TriangleBalance::Unstable)
    }
}

/// Sum of `rel.strength()` across all relationships between `a` and `b`.
/// Returns `None` if there are no relationships between them.
fn edge_strength_between(world: &World, a: LocusId, b: LocusId) -> Option<f32> {
    let mut total = 0.0f32;
    let mut found = false;
    for rel in world.relationships_between(a, b) {
        total += rel.strength();
        found = true;
    }
    if found { Some(total) } else { None }
}

/// Enumerate all triangles in the world (deduplicated).
///
/// Each triangle appears exactly once as a sorted triple `(a, b, c)` with
/// `a < b < c`. Order is ascending by `(a, b, c)`.
///
/// Complexity: O(V · Δ²) where Δ is the maximum degree.
pub fn all_triangles(world: &World) -> Vec<(LocusId, LocusId, LocusId)> {
    // Build adjacency sets
    let adj = build_adj(world);
    let mut triangles: Vec<(LocusId, LocusId, LocusId)> = Vec::new();

    let mut loci: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();
    loci.sort();

    for &a in &loci {
        let neighbors_a = match adj.get(&a) {
            Some(s) => s,
            None => continue,
        };
        // Only look at neighbors b > a to avoid duplicate triples
        for &b in neighbors_a {
            if b <= a {
                continue;
            }
            let neighbors_b = match adj.get(&b) {
                Some(s) => s,
                None => continue,
            };
            // Common neighbors c > b
            for &c in neighbors_b {
                if c <= b {
                    continue;
                }
                if neighbors_a.contains(&c) {
                    triangles.push((a, b, c));
                }
            }
        }
    }

    triangles.sort();
    triangles
}

/// All triangles that are structurally unstable at the given `threshold`.
pub fn unstable_triangles(world: &World, threshold: f32) -> Vec<(LocusId, LocusId, LocusId)> {
    all_triangles(world)
        .into_iter()
        .filter(|&(a, b, c)| {
            triangle_balance(world, a, b, c, threshold) == Some(TriangleBalance::Unstable)
        })
        .collect()
}

/// Fraction of all triangles that are balanced.
///
/// Returns `0.0` when the world contains no triangles.
pub fn balance_index(world: &World, threshold: f32) -> f32 {
    let triangles = all_triangles(world);
    if triangles.is_empty() {
        return 0.0;
    }
    let balanced = triangles
        .iter()
        .filter(|&&(a, b, c)| {
            triangle_balance(world, a, b, c, threshold) == Some(TriangleBalance::Balanced)
        })
        .count();
    balanced as f32 / triangles.len() as f32
}

/// Build an undirected adjacency set for every locus.
fn build_adj(world: &World) -> FxHashMap<LocusId, FxHashSet<LocusId>> {
    let mut adj: FxHashMap<LocusId, FxHashSet<LocusId>> = FxHashMap::default();
    for rel in world.relationships().iter() {
        let (u, v) = match rel.endpoints {
            graph_core::Endpoints::Symmetric { a, b } => (a, b),
            graph_core::Endpoints::Directed { from, to } => (from, to),
        };
        adj.entry(u).or_default().insert(v);
        adj.entry(v).or_default().insert(u);
    }
    adj
}

// ─── Modularity Q ─────────────────────────────────────────────────────────────

/// Compute Newman–Girvan modularity Q for a given partition.
///
/// ```text
/// Q = (1/2m) * Σ_{ij} [ A_ij - k_i*k_j/(2m) ] * δ(c_i, c_j)
/// ```
///
/// - `A_ij` = sum of `rel.strength()` over all relationships between i and j.
/// - `k_i` = total strength of node i.
/// - `2m` = sum of all edge weights × 2 (= Σ_i k_i).
/// - `δ(c_i, c_j)` = 1 iff i and j are in the same group.
///
/// `partition` is a slice of groups; each `LocusId` should appear in at most
/// one group. Nodes not appearing in any group are ignored. Returns `0.0` if
/// there are no edges or the partition is empty.
pub fn modularity(world: &World, partition: &[Vec<LocusId>]) -> f32 {
    if partition.is_empty() {
        return 0.0;
    }

    // Build group membership map: LocusId → group index
    let mut group_of: FxHashMap<LocusId, usize> = FxHashMap::default();
    for (g, members) in partition.iter().enumerate() {
        for &id in members {
            group_of.insert(id, g);
        }
    }

    // k_i = total strength of each locus
    let mut k: FxHashMap<LocusId, f32> = FxHashMap::default();
    for rel in world.relationships().iter() {
        let (u, v) = match rel.endpoints {
            graph_core::Endpoints::Symmetric { a, b } => (a, b),
            graph_core::Endpoints::Directed { from, to } => (from, to),
        };
        let s = rel.strength();
        *k.entry(u).or_insert(0.0) += s;
        *k.entry(v).or_insert(0.0) += s;
    }

    // 2m = Σ_i k_i
    let two_m: f32 = k.values().sum();
    if two_m == 0.0 {
        return 0.0;
    }

    // Build A_ij: sum of strengths between each pair that are in the same group
    // We iterate over all relationships and accumulate the modularity sum.
    // For the A_ij term: each undirected edge (u,v) contributes A_ij + A_ji = 2*w_ij
    // For the k_i*k_j/(2m) term: sum over all same-group pairs
    //
    // Efficient: iterate edges for A term; iterate same-group pairs for null model term.

    // Sum A_ij * δ(c_i, c_j) over ordered pairs (= 2 * sum over unordered edges in same group)
    let mut a_sum = 0.0f32;
    for rel in world.relationships().iter() {
        let (u, v) = match rel.endpoints {
            graph_core::Endpoints::Symmetric { a, b } => (a, b),
            graph_core::Endpoints::Directed { from, to } => (from, to),
        };
        let s = rel.strength();
        if let (Some(&gu), Some(&gv)) = (group_of.get(&u), group_of.get(&v)) {
            if gu == gv {
                // counts u→v and v→u (both ordered pairs)
                a_sum += 2.0 * s;
            }
        }
    }

    // Sum k_i * k_j / (2m) * δ(c_i, c_j) over ordered pairs
    // = Σ_g (Σ_{i in g} k_i)² / (2m)
    let mut null_sum = 0.0f32;
    for members in partition {
        let sum_k: f32 = members
            .iter()
            .map(|id| k.get(id).copied().unwrap_or(0.0))
            .sum();
        null_sum += sum_k * sum_k;
    }
    null_sum /= two_m;

    (a_sum - null_sum) / two_m
}

// ─── Betweenness Centrality (Brandes, undirected, normalized) ─────────────────

/// Compute betweenness centrality for a single locus using Brandes' algorithm.
///
/// Betweenness centrality measures how often a locus lies on a shortest path
/// between pairs of other loci. A high value signals a broker or bridge.
///
/// The graph is treated as **undirected**; both endpoints of `Directed` edges
/// count as neighbours. The score is normalised to \[0, 1\] by dividing the raw
/// Brandes accumulator by `(n-1)(n-2)`, where `n` is the number of loci.
/// Returns `0.0` for worlds with fewer than 3 loci.
///
/// Complexity: O(V · (V + E))
pub fn betweenness_centrality(world: &World, locus: LocusId) -> f32 {
    all_betweenness_inner(world)
        .into_iter()
        .find(|(id, _)| *id == locus)
        .map(|(_, v)| v)
        .unwrap_or(0.0)
}

/// Betweenness centrality for every locus, sorted **descending** by score.
///
/// Isolates and loci on no shortest path score `0.0` and appear last.
pub fn all_betweenness(world: &World) -> Vec<(LocusId, f32)> {
    let mut scores = all_betweenness_inner(world);
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    scores
}

fn all_betweenness_inner(world: &World) -> Vec<(LocusId, f32)> {
    let loci: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();
    let n = loci.len();
    if n < 3 {
        return loci.into_iter().map(|id| (id, 0.0)).collect();
    }
    let norm = (n - 1) as f32 * (n - 2) as f32;

    // Locus id → compact index.
    let idx: FxHashMap<LocusId, usize> = loci.iter().enumerate().map(|(i, &id)| (id, i)).collect();

    // Build undirected adjacency via dedup sets, then convert to sorted Vecs
    // for sequential (cache-friendly) iteration in the hot BFS loop.
    let mut adj_set: Vec<FxHashSet<usize>> = vec![FxHashSet::default(); n];
    for rel in world.relationships().iter() {
        let (u, v) = match rel.endpoints {
            graph_core::Endpoints::Symmetric { a, b } => (a, b),
            graph_core::Endpoints::Directed { from, to } => (from, to),
        };
        if let (Some(&ui), Some(&vi)) = (idx.get(&u), idx.get(&v)) {
            adj_set[ui].insert(vi);
            adj_set[vi].insert(ui);
        }
    }
    let adj: Vec<Vec<usize>> = adj_set
        .into_iter()
        .map(|s| {
            let mut v: Vec<usize> = s.into_iter().collect();
            v.sort_unstable();
            v
        })
        .collect();

    let mut cb = vec![0.0f32; n];

    // ── Reusable per-source buffers (reset via `visited` list, not full clear) ──
    let mut stack: Vec<usize> = Vec::with_capacity(n);
    let mut queue: VecDeque<usize> = VecDeque::with_capacity(n);
    let mut visited: Vec<usize> = Vec::with_capacity(n);
    let mut sigma: Vec<f32> = vec![0.0; n];
    let mut dist: Vec<i32> = vec![-1; n];
    let mut delta: Vec<f32> = vec![0.0; n];
    let mut pred: Vec<Vec<usize>> = vec![Vec::new(); n];

    for s in 0..n {
        // Reset only the nodes touched in the previous iteration.
        for &v in &visited {
            sigma[v] = 0.0;
            dist[v] = -1;
            delta[v] = 0.0;
            pred[v].clear();
        }
        visited.clear();
        stack.clear();
        queue.clear();

        sigma[s] = 1.0;
        dist[s] = 0;
        visited.push(s);
        queue.push_back(s);

        // Forward BFS: compute σ and predecessors on shortest paths.
        while let Some(v) = queue.pop_front() {
            stack.push(v);
            let dv = dist[v];
            for &w in &adj[v] {
                if dist[w] < 0 {
                    dist[w] = dv + 1;
                    visited.push(w);
                    queue.push_back(w);
                }
                if dist[w] == dv + 1 {
                    sigma[w] += sigma[v];
                    pred[w].push(v);
                }
            }
        }

        // Backward pass: accumulate pair dependencies.
        while let Some(w) = stack.pop() {
            for &v in &pred[w] {
                delta[v] += (sigma[v] / sigma[w]) * (1.0 + delta[w]);
            }
            if w != s {
                cb[w] += delta[w];
            }
        }
    }

    // For undirected graphs each unordered pair is counted from both endpoints,
    // so the raw Brandes sum equals 2× the directed betweenness.  Normalise
    // by (n-1)(n-2) to map to [0, 1].
    loci.into_iter()
        .enumerate()
        .map(|(i, id)| (id, cb[i] / norm))
        .collect()
}

// ─── Harmonic Closeness Centrality ───────────────────────────────────────────

/// Compute harmonic closeness centrality for `locus`.
///
/// Defined as:
/// ```text
/// H(v) = (1 / (n-1)) × Σ_{t≠v, t reachable} 1/d(v,t)
/// ```
///
/// Unlike classical closeness, the harmonic variant handles disconnected graphs
/// naturally — unreachable nodes contribute `0` to the sum.
///
/// Returns `None` for worlds with fewer than 2 loci. Returns `Some(0.0)` for
/// isolated loci (no reachable neighbours).
///
/// Complexity: O(V + E) per call.
pub fn closeness_centrality(world: &World, locus: LocusId) -> Option<f32> {
    let n = world.loci().len();
    if n < 2 {
        return None;
    }
    Some(bfs_harmonic_sum(world, locus) / (n as f32 - 1.0))
}

/// Harmonic closeness centrality for every locus, sorted **descending**.
///
/// Returns an empty `Vec` for worlds with fewer than 2 loci.
///
/// Uses a shared index-based BFS state (one `Vec<i32>` reset via a visited
/// list) to avoid per-locus HashMap allocations.
pub fn all_closeness(world: &World) -> Vec<(LocusId, f32)> {
    let loci: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();
    let n = loci.len();
    if n < 2 {
        return Vec::new();
    }

    let idx: FxHashMap<LocusId, usize> = loci.iter().enumerate().map(|(i, &id)| (id, i)).collect();

    // Index-based undirected adjacency (dedup via FxHashSet, stored as Vec for
    // cache-friendly iteration).
    let mut adj_set: Vec<FxHashSet<usize>> = vec![FxHashSet::default(); n];
    for rel in world.relationships().iter() {
        let (a, b) = match rel.endpoints {
            graph_core::Endpoints::Symmetric { a, b } => (a, b),
            graph_core::Endpoints::Directed { from, to } => (from, to),
        };
        if let (Some(&ai), Some(&bi)) = (idx.get(&a), idx.get(&b)) {
            adj_set[ai].insert(bi);
            adj_set[bi].insert(ai);
        }
    }
    let adj: Vec<Vec<usize>> = adj_set
        .into_iter()
        .map(|s| {
            let mut v: Vec<usize> = s.into_iter().collect();
            v.sort_unstable();
            v
        })
        .collect();

    // Shared BFS buffers — reset only touched nodes between iterations.
    let mut dist: Vec<i32> = vec![-1; n];
    let mut queue: VecDeque<usize> = VecDeque::with_capacity(n);
    let mut visited: Vec<usize> = Vec::with_capacity(n);

    let denom = n as f32 - 1.0;
    let mut result: Vec<(LocusId, f32)> = Vec::with_capacity(n);

    for s in 0..n {
        // Partial reset: only nodes visited in the previous BFS.
        for &v in &visited {
            dist[v] = -1;
        }
        visited.clear();
        queue.clear();

        dist[s] = 0;
        visited.push(s);
        queue.push_back(s);

        let mut harmonic = 0.0f32;
        while let Some(v) = queue.pop_front() {
            let d = dist[v];
            for &w in &adj[v] {
                if dist[w] < 0 {
                    let nd = d + 1;
                    dist[w] = nd;
                    visited.push(w);
                    harmonic += 1.0 / nd as f32;
                    queue.push_back(w);
                }
            }
        }
        result.push((loci[s], harmonic / denom));
    }

    result.sort_by(|a, b| b.1.total_cmp(&a.1));
    result
}

/// BFS from `start`; returns the sum of `1/d` over all reachable loci
/// (excluding `start` itself). Used by the single-locus `closeness_centrality`.
fn bfs_harmonic_sum(world: &World, start: LocusId) -> f32 {
    let mut dist: FxHashMap<LocusId, u32> = FxHashMap::default();
    let mut queue: VecDeque<LocusId> = VecDeque::new();
    dist.insert(start, 0);
    queue.push_back(start);
    let mut harmonic = 0.0f32;

    while let Some(current) = queue.pop_front() {
        let d = dist[&current];
        for rel in world.relationships_for_locus(current) {
            let neighbour = rel.endpoints.other_than(current);
            if !dist.contains_key(&neighbour) {
                let nd = d + 1;
                dist.insert(neighbour, nd);
                harmonic += 1.0 / nd as f32;
                queue.push_back(neighbour);
            }
        }
    }
    harmonic
}

// ─── PageRank ─────────────────────────────────────────────────────────────────

/// Compute PageRank for every locus using relationship **activity** as edge
/// weight, sorted **descending** by score.
///
/// This is an activity-weighted variant of the standard PageRank algorithm.
/// For each directed hop `u → v`, the contribution is proportional to
/// `activity(u→v) / total_out_activity(u)`.  `Symmetric` edges count in both
/// directions; `Directed` edges count only forward.
///
/// Dangling nodes (loci whose total out-activity is zero) redistribute their
/// mass uniformly to all nodes in each iteration, preventing rank sinks.
///
/// ## Parameters
/// - `damping` — typical value `0.85`.  Fraction of rank passed through edges
///   (the remaining `1-damping` is teleported uniformly).
/// - `max_iter` — iteration cap (convergence is usually reached in < 50 steps).
/// - `tol` — convergence threshold: sum of absolute score changes per step.
///
/// Returns an empty `Vec` for worlds with no loci.
///
/// Complexity: O(max_iter × (V + E)).
pub fn pagerank(world: &World, damping: f32, max_iter: usize, tol: f32) -> Vec<(LocusId, f32)> {
    let loci: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();
    let n = loci.len();
    if n == 0 {
        return Vec::new();
    }
    let idx: FxHashMap<LocusId, usize> = loci.iter().enumerate().map(|(i, &id)| (id, i)).collect();

    // Build weighted out-edges and total out-activity per node.
    // in_edges[v] = list of (u, normalised_weight) where u→v.
    let mut in_edges: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    let mut out_activity: Vec<f32> = vec![0.0; n];

    for rel in world.relationships().iter() {
        let activity = rel.activity().max(0.0);
        if activity == 0.0 {
            continue;
        }
        match rel.endpoints {
            graph_core::Endpoints::Directed { from, to } => {
                if let (Some(&ui), Some(&vi)) = (idx.get(&from), idx.get(&to)) {
                    in_edges[vi].push((ui, activity));
                    out_activity[ui] += activity;
                }
            }
            graph_core::Endpoints::Symmetric { a, b } => {
                if let (Some(&ai), Some(&bi)) = (idx.get(&a), idx.get(&b)) {
                    in_edges[bi].push((ai, activity));
                    in_edges[ai].push((bi, activity));
                    out_activity[ai] += activity;
                    out_activity[bi] += activity;
                }
            }
        }
    }

    // Normalise in-edge weights by the source's total out-activity.
    for v in 0..n {
        for (u, w) in &mut in_edges[v] {
            let total = out_activity[*u];
            if total > 0.0 {
                *w /= total;
            }
        }
    }

    // Identify dangling nodes (no outgoing activity).
    let dangling: Vec<usize> = (0..n).filter(|&i| out_activity[i] == 0.0).collect();

    let teleport = (1.0 - damping) / n as f32;
    let mut pr = vec![1.0 / n as f32; n];
    let mut pr_new = vec![0.0f32; n];

    for _ in 0..max_iter {
        // Dangling mass redistributed uniformly.
        let dangling_sum: f32 = dangling.iter().map(|&i| pr[i]).sum();
        let dangling_contrib = damping * dangling_sum / n as f32;

        for v in 0..n {
            let link_sum: f32 = in_edges[v].iter().map(|&(u, w)| pr[u] * w).sum();
            pr_new[v] = teleport + dangling_contrib + damping * link_sum;
        }

        // Convergence check.
        let delta: f32 = pr.iter().zip(&pr_new).map(|(a, b)| (a - b).abs()).sum();
        pr.copy_from_slice(&pr_new);
        if delta < tol {
            break;
        }
    }

    let mut result: Vec<(LocusId, f32)> = loci
        .into_iter()
        .enumerate()
        .map(|(i, id)| (id, pr[i]))
        .collect();
    result.sort_by(|a, b| b.1.total_cmp(&a.1));
    result
}

/// PageRank for a single locus with default parameters
/// (`damping = 0.85`, `max_iter = 100`, `tol = 1e-6`).
///
/// Returns `0.0` if `locus` is not present in the world.
pub fn pagerank_centrality(world: &World, locus: LocusId) -> f32 {
    pagerank(world, 0.85, 100, 1e-6)
        .into_iter()
        .find(|(id, _)| *id == locus)
        .map(|(_, v)| v)
        .unwrap_or(0.0)
}

// ─── Louvain Community Detection ─────────────────────────────────────────────

/// Detect communities using the Louvain modularity-optimisation algorithm.
///
/// Equivalent to [`louvain_with_resolution`] with `gamma = 1.0`.
///
/// Returns a list of communities (each community is a `Vec<LocusId>`), sorted
/// by size descending. Members within each community are sorted by `LocusId`.
/// Loci with no edges each appear as a singleton community.
pub fn louvain(world: &World) -> Vec<Vec<LocusId>> {
    louvain_with_resolution(world, 1.0)
}

/// Detect communities using the Louvain modularity-optimisation algorithm with
/// a tunable resolution parameter `gamma`.
///
/// The algorithm greedily maximises the Newman–Girvan modularity:
/// ```text
/// Q = (1/2m) Σ_{ij} [A_ij − γ · k_i · k_j / 2m] · δ(c_i, c_j)
/// ```
/// Edge weight is `rel.activity()`.  Both `Directed` and `Symmetric` edges are
/// treated as undirected (activity symmetrised).
///
/// **`gamma` (resolution parameter)**
/// - `gamma < 1.0` → fewer, larger communities.
/// - `gamma = 1.0` → standard modularity (default).
/// - `gamma > 1.0` → more, smaller communities.
///
/// The phase-1 greedy pass iterates over all nodes, moving each to the
/// neighbouring community that yields the greatest modularity gain.  Passes
/// repeat until a full sweep produces no improvement.
///
/// Complexity: O(passes × (V + E)) — typically converges in a handful of
/// passes for real-world graphs.
pub fn louvain_with_resolution(world: &World, gamma: f32) -> Vec<Vec<LocusId>> {
    let loci: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();
    let n = loci.len();
    if n == 0 {
        return Vec::new();
    }

    let idx: FxHashMap<LocusId, usize> = loci.iter().enumerate().map(|(i, &id)| (id, i)).collect();

    // Build undirected weighted adjacency (parallel edges summed).
    let mut adj_map: Vec<FxHashMap<usize, f32>> = vec![FxHashMap::default(); n];
    for rel in world.relationships().iter() {
        let w = rel.activity().max(0.0);
        if w == 0.0 {
            continue;
        }
        let (a, b) = match rel.endpoints {
            graph_core::Endpoints::Symmetric { a, b } => (a, b),
            graph_core::Endpoints::Directed { from, to } => (from, to),
        };
        if let (Some(&ai), Some(&bi)) = (idx.get(&a), idx.get(&b)) {
            *adj_map[ai].entry(bi).or_insert(0.0) += w;
            *adj_map[bi].entry(ai).or_insert(0.0) += w;
        }
    }
    let adj: Vec<Vec<(usize, f32)>> = adj_map
        .into_iter()
        .map(|m| m.into_iter().collect())
        .collect();

    // k[v] = total incident weight; m2 = 2m = Σ k[v].
    let k: Vec<f32> = adj.iter().map(|e| e.iter().map(|(_, w)| w).sum()).collect();
    let m2: f32 = k.iter().sum();

    if m2 == 0.0 {
        // No edges — every node is its own community.
        return loci.into_iter().map(|id| vec![id]).collect();
    }

    // community[v] = community index of v.
    // sigma_tot[c] = Σ k[v] for v in community c.
    let mut community: Vec<usize> = (0..n).collect();
    let mut sigma_tot: Vec<f32> = k.clone();

    // Reusable buffer for aggregating neighbour-community weights.
    // Avoids a per-node FxHashMap allocation in the hot loop.
    // Layout: (community_index, accumulated_weight); cleared each iteration.
    let mut nc_buf: Vec<(usize, f32)> = Vec::new();

    let mut improved = true;
    while improved {
        improved = false;
        for v in 0..n {
            let cv = community[v];

            // k_{v, in_cv} — weight from v to the rest of its current community.
            let k_v_in_cv: f32 = adj[v]
                .iter()
                .filter(|&&(u, _)| community[u] == cv)
                .map(|(_, w)| w)
                .sum();

            // Score of v staying in cv (after removing v from cv).
            let score_stay = k_v_in_cv - gamma * k[v] * (sigma_tot[cv] - k[v]) / m2;

            // Collect (community, weight) for all neighbours not in cv.
            nc_buf.clear();
            for &(u, w) in &adj[v] {
                let cu = community[u];
                if cu != cv {
                    // Linear scan is fast for typical low-degree nodes; most
                    // graphs have avg_degree << 100.
                    if let Some(entry) = nc_buf.iter_mut().find(|(c, _)| *c == cu) {
                        entry.1 += w;
                    } else {
                        nc_buf.push((cu, w));
                    }
                }
            }

            // Find the best target community.
            let mut best_c = cv;
            let mut best_score = score_stay;
            for &(c, k_v_in_c) in &nc_buf {
                let score = k_v_in_c - gamma * k[v] * sigma_tot[c] / m2;
                if score > best_score {
                    best_score = score;
                    best_c = c;
                }
            }

            if best_c != cv {
                sigma_tot[cv] -= k[v];
                sigma_tot[best_c] += k[v];
                community[v] = best_c;
                improved = true;
            }
        }
    }

    // Collect communities.
    let mut groups: FxHashMap<usize, Vec<LocusId>> = FxHashMap::default();
    for (i, &id) in loci.iter().enumerate() {
        groups.entry(community[i]).or_default().push(id);
    }

    let mut result: Vec<Vec<LocusId>> = groups.into_values().collect();
    for g in &mut result {
        g.sort();
    }
    result.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    result
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, KindObservation, Locus, LocusId, LocusKindId,
        Relationship, RelationshipLineage, StateVector,
    };
    use graph_world::World;
    use smallvec::smallvec;

    const LK: LocusKindId = LocusKindId(1);
    const RK: InfluenceKindId = InfluenceKindId(1);

    fn make_locus(id: u64) -> Locus {
        Locus::new(LocusId(id), LK, StateVector::zeros(1))
    }

    /// Insert a symmetric (undirected) relationship with given activity+weight.
    fn add_sym_rel(world: &mut World, a: u64, b: u64, activity: f32, weight: f32) {
        let id = world.relationships_mut().mint_id();
        world.relationships_mut().insert(Relationship {
            id,
            kind: RK,
            endpoints: Endpoints::Symmetric {
                a: LocusId(a),
                b: LocusId(b),
            },
            state: StateVector::from_slice(&[activity, weight]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec![KindObservation::synthetic(RK)],
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
    }

    // ── Star graph ────────────────────────────────────────────────────────────

    /// Star: hub=0, spokes=1,2,3.
    /// Hub bridges all spokes → low constraint.
    /// Spokes only connect to hub → high constraint.
    fn star_world() -> World {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        // All edges uniform strength 1.0 (activity=0.5, weight=0.5)
        for spoke in 1u64..4 {
            add_sym_rel(&mut w, 0, spoke, 0.5, 0.5);
        }
        w
    }

    #[test]
    fn star_hub_has_lower_constraint_than_spokes() {
        let w = star_world();
        let c_hub = structural_constraint(&w, LocusId(0)).expect("hub has rels");
        let c_spoke = structural_constraint(&w, LocusId(1)).expect("spoke has rels");
        assert!(
            c_hub < c_spoke,
            "hub constraint {c_hub} should be < spoke constraint {c_spoke}"
        );
    }

    #[test]
    fn star_spoke_no_rels_returns_none() {
        let mut w = World::new();
        w.insert_locus(make_locus(99));
        assert!(structural_constraint(&w, LocusId(99)).is_none());
    }

    #[test]
    fn all_constraints_sorted_ascending() {
        let w = star_world();
        let cs = all_constraints(&w);
        assert!(!cs.is_empty());
        for pair in cs.windows(2) {
            assert!(pair[0].1 <= pair[1].1, "not sorted: {:?}", cs);
        }
        // Hub should appear first (lowest constraint)
        assert_eq!(cs[0].0, LocusId(0));
    }

    #[test]
    fn star_effective_network_size_hub_near_three() {
        // Hub has 3 non-redundant contacts (spokes don't interconnect)
        let w = star_world();
        let ens = effective_network_size(&w, LocusId(0));
        // Should be close to 3.0 (each spoke is independent of the others)
        assert!(ens > 2.5 && ens <= 3.0, "expected ~3.0, got {ens}");
    }

    // ── Clique ────────────────────────────────────────────────────────────────

    /// Fully connected K4 clique: nodes 0,1,2,3 all connected.
    fn clique_world() -> World {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        for a in 0u64..4 {
            for b in (a + 1)..4 {
                add_sym_rel(&mut w, a, b, 0.5, 0.5);
            }
        }
        w
    }

    #[test]
    fn clique_all_nodes_high_constraint() {
        let w = clique_world();
        // In a clique all contacts are connected to each other → high constraint
        // Theoretical max for 3 contacts is 1.0; for 4-node clique it's ~1.125
        for i in 0u64..4 {
            let c = structural_constraint(&w, LocusId(i)).expect("clique node has rels");
            assert!(
                c > 0.5,
                "clique node {i} should have high constraint, got {c}"
            );
        }
    }

    #[test]
    fn clique_hub_has_higher_constraint_than_star_hub() {
        let star = star_world();
        let clique = clique_world();
        let c_star = structural_constraint(&star, LocusId(0)).unwrap();
        let c_clique = structural_constraint(&clique, LocusId(0)).unwrap();
        assert!(
            c_clique > c_star,
            "clique hub {c_clique} should be more constrained than star hub {c_star}"
        );
    }

    // ── Triangle balance ──────────────────────────────────────────────────────

    fn triangle_world(strength_ab: f32, strength_bc: f32, strength_ac: f32) -> World {
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(make_locus(i));
        }
        add_sym_rel(&mut w, 0, 1, strength_ab / 2.0, strength_ab / 2.0);
        add_sym_rel(&mut w, 1, 2, strength_bc / 2.0, strength_bc / 2.0);
        add_sym_rel(&mut w, 0, 2, strength_ac / 2.0, strength_ac / 2.0);
        w
    }

    #[test]
    fn triangle_all_positive_is_balanced() {
        // +++ → product = +1 → balanced
        let w = triangle_world(1.0, 1.0, 1.0);
        let bal = triangle_balance(&w, LocusId(0), LocusId(1), LocusId(2), 0.0);
        assert_eq!(bal, Some(TriangleBalance::Balanced));
    }

    #[test]
    fn triangle_two_neg_one_pos_is_balanced() {
        // +-- → product = +1 → balanced
        let w = triangle_world(1.0, -0.5, -0.5);
        let bal = triangle_balance(&w, LocusId(0), LocusId(1), LocusId(2), 0.0);
        assert_eq!(bal, Some(TriangleBalance::Balanced));
    }

    #[test]
    fn triangle_one_neg_two_pos_is_unstable() {
        // ++- → product = -1 → unstable
        let w = triangle_world(1.0, 1.0, -0.5);
        let bal = triangle_balance(&w, LocusId(0), LocusId(1), LocusId(2), 0.0);
        assert_eq!(bal, Some(TriangleBalance::Unstable));
    }

    #[test]
    fn triangle_all_negative_is_unstable() {
        // --- → product = -1 → unstable
        let w = triangle_world(-1.0, -1.0, -1.0);
        let bal = triangle_balance(&w, LocusId(0), LocusId(1), LocusId(2), 0.0);
        assert_eq!(bal, Some(TriangleBalance::Unstable));
    }

    #[test]
    fn triangle_missing_edge_returns_none() {
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(make_locus(i));
        }
        add_sym_rel(&mut w, 0, 1, 0.5, 0.5);
        add_sym_rel(&mut w, 1, 2, 0.5, 0.5);
        // No edge 0-2
        let bal = triangle_balance(&w, LocusId(0), LocusId(1), LocusId(2), 0.0);
        assert_eq!(bal, None);
    }

    #[test]
    fn all_triangles_clique_returns_four_triangles() {
        let w = clique_world();
        // K4 has C(4,3)=4 triangles
        let ts = all_triangles(&w);
        assert_eq!(ts.len(), 4, "K4 has 4 triangles, got: {:?}", ts);
    }

    #[test]
    fn all_triangles_dedup_sorted() {
        let w = clique_world();
        let ts = all_triangles(&w);
        for t in &ts {
            assert!(t.0 < t.1 && t.1 < t.2, "triangle not sorted: {:?}", t);
        }
        let mut sorted = ts.clone();
        sorted.sort();
        assert_eq!(ts, sorted, "triangles not in sorted order");
    }

    #[test]
    fn balance_index_all_positive_clique() {
        // All edges positive (strength 1.0 > threshold 0.0) → +++ for every triangle → 1.0
        let w = clique_world();
        let bi = balance_index(&w, 0.0);
        assert!((bi - 1.0).abs() < 1e-5, "expected 1.0, got {bi}");
    }

    #[test]
    fn balance_index_no_triangles_returns_zero() {
        let w = star_world(); // star has no triangles
        let bi = balance_index(&w, 0.0);
        assert_eq!(bi, 0.0);
    }

    #[test]
    fn unstable_triangles_detects_unstable() {
        // Build a triangle where one edge is negative (++-)
        let w = triangle_world(1.0, 1.0, -0.5);
        let us = unstable_triangles(&w, 0.0);
        assert_eq!(us.len(), 1);
    }

    // ── Modularity ────────────────────────────────────────────────────────────

    /// Two disconnected components: {0,1} and {2,3}, each fully connected.
    fn two_component_world() -> World {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        add_sym_rel(&mut w, 0, 1, 0.5, 0.5); // component A
        add_sym_rel(&mut w, 2, 3, 0.5, 0.5); // component B
        w
    }

    #[test]
    fn modularity_perfect_partition_near_one() {
        let w = two_component_world();
        let partition = vec![vec![LocusId(0), LocusId(1)], vec![LocusId(2), LocusId(3)]];
        let q = modularity(&w, &partition);
        // For two disconnected equal components, Q = 0.5
        assert!(q > 0.4, "expected Q near 0.5, got {q}");
    }

    #[test]
    fn modularity_single_community_near_zero() {
        let w = two_component_world();
        // Put all nodes in one group
        let partition = vec![vec![LocusId(0), LocusId(1), LocusId(2), LocusId(3)]];
        let q = modularity(&w, &partition);
        // Q ≈ 0 for a single-community partition
        assert!(q.abs() < 0.1, "expected Q near 0, got {q}");
    }

    #[test]
    fn modularity_empty_partition_returns_zero() {
        let w = two_component_world();
        let q = modularity(&w, &[]);
        assert_eq!(q, 0.0);
    }

    #[test]
    fn modularity_no_edges_returns_zero() {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        let partition = vec![vec![LocusId(0), LocusId(1)], vec![LocusId(2), LocusId(3)]];
        let q = modularity(&w, &partition);
        assert_eq!(q, 0.0);
    }

    #[test]
    fn modularity_wrong_partition_lower_than_correct() {
        let w = two_component_world();
        let correct = vec![vec![LocusId(0), LocusId(1)], vec![LocusId(2), LocusId(3)]];
        let wrong = vec![vec![LocusId(0), LocusId(2)], vec![LocusId(1), LocusId(3)]];
        let q_correct = modularity(&w, &correct);
        let q_wrong = modularity(&w, &wrong);
        assert!(
            q_correct > q_wrong,
            "correct partition Q={q_correct} should be > wrong Q={q_wrong}"
        );
    }

    // ── Betweenness centrality ─────────────────────────────────────────────────

    /// Path graph 0–1–2–3: locus 1 and 2 are on all shortest paths between the
    /// two halves → higher betweenness than the endpoints 0 and 3.
    fn path4_world() -> World {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        // Undirected chain: 0-1-2-3
        add_sym_rel(&mut w, 0, 1, 0.5, 0.5);
        add_sym_rel(&mut w, 1, 2, 0.5, 0.5);
        add_sym_rel(&mut w, 2, 3, 0.5, 0.5);
        w
    }

    #[test]
    fn betweenness_endpoints_are_zero() {
        let w = path4_world();
        let b0 = betweenness_centrality(&w, LocusId(0));
        let b3 = betweenness_centrality(&w, LocusId(3));
        assert_eq!(b0, 0.0, "endpoint 0 should have 0 betweenness");
        assert_eq!(b3, 0.0, "endpoint 3 should have 0 betweenness");
    }

    #[test]
    fn betweenness_inner_nodes_higher_than_endpoints() {
        let w = path4_world();
        let b1 = betweenness_centrality(&w, LocusId(1));
        let b2 = betweenness_centrality(&w, LocusId(2));
        let b0 = betweenness_centrality(&w, LocusId(0));
        assert!(b1 > b0, "node 1: {b1} should beat endpoint 0: {b0}");
        assert!(b2 > b0, "node 2: {b2} should beat endpoint 0: {b0}");
    }

    #[test]
    fn all_betweenness_sorted_descending() {
        let w = path4_world();
        let scores = all_betweenness(&w);
        assert_eq!(scores.len(), 4);
        for pair in scores.windows(2) {
            assert!(pair[0].1 >= pair[1].1, "not sorted desc: {:?}", scores);
        }
    }

    #[test]
    fn betweenness_star_hub_highest() {
        // In a star, the hub lies on every shortest path between spokes.
        let w = star_world();
        let b_hub = betweenness_centrality(&w, LocusId(0));
        for spoke in 1u64..4 {
            let b_spoke = betweenness_centrality(&w, LocusId(spoke));
            assert!(b_hub > b_spoke, "hub {b_hub} should beat spoke {b_spoke}");
        }
    }

    #[test]
    fn betweenness_small_world_returns_zero_for_missing_locus() {
        let mut w = World::new();
        w.insert_locus(make_locus(0));
        // < 3 loci → everyone is 0.0
        assert_eq!(betweenness_centrality(&w, LocusId(0)), 0.0);
    }

    // ── Closeness centrality ──────────────────────────────────────────────────

    #[test]
    fn closeness_single_locus_returns_none() {
        let mut w = World::new();
        w.insert_locus(make_locus(0));
        assert!(closeness_centrality(&w, LocusId(0)).is_none());
    }

    #[test]
    fn closeness_isolated_locus_is_zero() {
        let mut w = World::new();
        w.insert_locus(make_locus(0));
        w.insert_locus(make_locus(1));
        // No edges → harmonic sum = 0
        let c = closeness_centrality(&w, LocusId(0)).unwrap();
        assert_eq!(c, 0.0);
    }

    #[test]
    fn closeness_hub_higher_than_spoke_in_star() {
        let w = star_world();
        let c_hub = closeness_centrality(&w, LocusId(0)).unwrap();
        let c_spoke = closeness_centrality(&w, LocusId(1)).unwrap();
        assert!(c_hub > c_spoke, "hub {c_hub} should beat spoke {c_spoke}");
    }

    #[test]
    fn all_closeness_sorted_descending() {
        let w = star_world();
        let scores = all_closeness(&w);
        assert_eq!(scores.len(), 4);
        for pair in scores.windows(2) {
            assert!(pair[0].1 >= pair[1].1, "not sorted desc: {:?}", scores);
        }
        // Hub should appear first
        assert_eq!(scores[0].0, LocusId(0));
    }

    #[test]
    fn all_closeness_empty_for_one_locus() {
        let mut w = World::new();
        w.insert_locus(make_locus(0));
        assert!(all_closeness(&w).is_empty());
    }

    // ── PageRank ──────────────────────────────────────────────────────────────

    fn add_dir_rel(world: &mut World, from: u64, to: u64, activity: f32) {
        let id = world.relationships_mut().mint_id();
        world.relationships_mut().insert(Relationship {
            id,
            kind: RK,
            endpoints: graph_core::Endpoints::Directed {
                from: LocusId(from),
                to: LocusId(to),
            },
            state: StateVector::from_slice(&[activity, 0.5]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec![KindObservation::synthetic(RK)],
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
    }

    #[test]
    fn pagerank_empty_world_returns_empty() {
        assert!(pagerank(&World::new(), 0.85, 100, 1e-6).is_empty());
    }

    #[test]
    fn pagerank_scores_sum_to_one() {
        let w = star_world(); // undirected star → symmetric edges
        let scores = pagerank(&w, 0.85, 100, 1e-6);
        let total: f32 = scores.iter().map(|(_, v)| v).sum();
        assert!(
            (total - 1.0).abs() < 1e-4,
            "scores should sum to 1, got {total}"
        );
    }

    #[test]
    fn pagerank_hub_ranks_first_in_star() {
        let w = star_world();
        let scores = pagerank(&w, 0.85, 100, 1e-6);
        // Hub (locus 0) should have the highest PageRank in a star graph.
        assert_eq!(scores[0].0, LocusId(0), "hub should rank first: {scores:?}");
    }

    #[test]
    fn pagerank_sorted_descending() {
        let w = star_world();
        let scores = pagerank(&w, 0.85, 100, 1e-6);
        for pair in scores.windows(2) {
            assert!(pair[0].1 >= pair[1].1, "not sorted desc: {:?}", scores);
        }
    }

    #[test]
    fn pagerank_sink_accumulates_more_rank() {
        // Chain 0→1→2→3: locus 3 receives flow from all upstream nodes.
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        for i in 0u64..3 {
            add_dir_rel(&mut w, i, i + 1, 1.0);
        }
        let scores = pagerank(&w, 0.85, 100, 1e-6);
        // Sink node 3 should rank higher than source node 0.
        let pr0 = scores.iter().find(|(id, _)| *id == LocusId(0)).unwrap().1;
        let pr3 = scores.iter().find(|(id, _)| *id == LocusId(3)).unwrap().1;
        assert!(pr3 > pr0, "sink {pr3} should beat source {pr0}");
    }

    #[test]
    fn pagerank_centrality_single_locus_default_params() {
        let w = star_world();
        let pr_hub = pagerank_centrality(&w, LocusId(0));
        let pr_spoke = pagerank_centrality(&w, LocusId(1));
        assert!(pr_hub > pr_spoke, "hub {pr_hub} > spoke {pr_spoke}");
    }

    #[test]
    fn pagerank_centrality_missing_locus_returns_zero() {
        let w = star_world();
        assert_eq!(pagerank_centrality(&w, LocusId(99)), 0.0);
    }

    // ── Louvain community detection ───────────────────────────────────────────

    /// Two disconnected cliques: {0,1,2} and {3,4,5}.
    /// Louvain should recover both communities.
    fn two_clique_world() -> World {
        let mut w = World::new();
        for i in 0u64..6 {
            w.insert_locus(make_locus(i));
        }
        // Clique A: 0-1, 1-2, 0-2
        for (a, b) in [(0u64, 1), (1, 2), (0, 2)] {
            add_sym_rel(&mut w, a, b, 0.5, 0.5);
        }
        // Clique B: 3-4, 4-5, 3-5
        for (a, b) in [(3u64, 4), (4, 5), (3, 5)] {
            add_sym_rel(&mut w, a, b, 0.5, 0.5);
        }
        w
    }

    #[test]
    fn louvain_empty_world_returns_empty() {
        assert!(louvain(&World::new()).is_empty());
    }

    #[test]
    fn louvain_no_edges_each_node_is_own_community() {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        let comms = louvain(&w);
        assert_eq!(comms.len(), 4, "4 isolated nodes → 4 singleton communities");
        for c in &comms {
            assert_eq!(c.len(), 1);
        }
    }

    #[test]
    fn louvain_two_cliques_recovers_both_groups() {
        let w = two_clique_world();
        let comms = louvain(&w);
        // Should find exactly 2 communities of size 3 each.
        assert_eq!(comms.len(), 2, "expected 2 communities, got {comms:?}");
        let mut sizes: Vec<usize> = comms.iter().map(Vec::len).collect();
        sizes.sort();
        assert_eq!(sizes, vec![3, 3]);

        // Nodes within each clique should be grouped together.
        let clique_a: Vec<LocusId> = vec![LocusId(0), LocusId(1), LocusId(2)];
        let clique_b: Vec<LocusId> = vec![LocusId(3), LocusId(4), LocusId(5)];
        assert!(
            comms.iter().any(|c| c == &clique_a),
            "clique A not found in {comms:?}"
        );
        assert!(
            comms.iter().any(|c| c == &clique_b),
            "clique B not found in {comms:?}"
        );
    }

    #[test]
    fn louvain_all_nodes_covered() {
        let w = two_clique_world();
        let comms = louvain(&w);
        let mut all_nodes: Vec<LocusId> = comms.into_iter().flatten().collect();
        all_nodes.sort();
        let expected: Vec<LocusId> = (0u64..6).map(LocusId).collect();
        assert_eq!(all_nodes, expected, "every node must appear exactly once");
    }

    #[test]
    fn louvain_modularity_partition_beats_single_community() {
        let w = two_clique_world();
        let comms = louvain(&w);
        let q_louvain = modularity(&w, &comms);
        let all_together = vec![(0u64..6).map(LocusId).collect::<Vec<_>>()];
        let q_single = modularity(&w, &all_together);
        assert!(
            q_louvain > q_single,
            "Louvain Q={q_louvain} should beat single-community Q={q_single}"
        );
    }

    #[test]
    fn louvain_high_resolution_splits_more() {
        // Bridge graph: two cliques connected by a single weak bridge.
        let mut w = World::new();
        for i in 0u64..6 {
            w.insert_locus(make_locus(i));
        }
        for (a, b) in [(0u64, 1), (1, 2), (0, 2)] {
            add_sym_rel(&mut w, a, b, 1.0, 0.0);
        }
        for (a, b) in [(3u64, 4), (4, 5), (3, 5)] {
            add_sym_rel(&mut w, a, b, 1.0, 0.0);
        }
        // Weak bridge between the two cliques
        add_sym_rel(&mut w, 2, 3, 0.1, 0.0);

        let comms_default = louvain(&w);
        let comms_high_res = louvain_with_resolution(&w, 3.0);
        // Higher resolution should find at least as many communities.
        assert!(
            comms_high_res.len() >= comms_default.len(),
            "high resolution should not merge more: default={} high_res={}",
            comms_default.len(),
            comms_high_res.len(),
        );
    }
}
