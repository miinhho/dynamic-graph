use graph_core::LocusId;
use graph_world::World;

use super::{brandes, traversal};

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
        .map(|(_, value)| value)
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
    let node_count = world.loci().len();
    if node_count < 2 {
        return None;
    }
    Some(traversal::bfs_harmonic_sum(world, locus) / (node_count as f32 - 1.0))
}

/// Harmonic closeness centrality for every locus, sorted **descending**.
///
/// Returns an empty `Vec` for worlds with fewer than 2 loci.
///
/// Uses a shared index-based BFS state (one `Vec<i32>` reset via a visited
/// list) to avoid per-locus HashMap allocations.
pub fn all_closeness(world: &World) -> Vec<(LocusId, f32)> {
    traversal::all_closeness(world)
}

/// Compute PageRank for every locus using relationship **activity** as edge
/// weight, sorted **descending** by score.
///
/// This is an activity-weighted variant of the standard PageRank algorithm.
/// For each directed hop `u → v`, the contribution is proportional to
/// `activity(u→v) / total_out_activity(u)`. `Symmetric` edges count in both
/// directions; `Directed` edges count only forward.
///
/// Dangling nodes (loci whose total out-activity is zero) redistribute their
/// mass uniformly to all nodes in each iteration, preventing rank sinks.
///
/// ## Parameters
/// - `damping` — typical value `0.85`. Fraction of rank passed through edges
///   (the remaining `1-damping` is teleported uniformly).
/// - `max_iter` — iteration cap (convergence is usually reached in < 50 steps).
/// - `tol` — convergence threshold: sum of absolute score changes per step.
///
/// Returns an empty `Vec` for worlds with no loci.
///
/// Complexity: O(max_iter × (V + E)).
pub fn pagerank(world: &World, damping: f32, max_iter: usize, tol: f32) -> Vec<(LocusId, f32)> {
    traversal::pagerank(world, damping, max_iter, tol)
}

/// PageRank for a single locus with default parameters
/// (`damping = 0.85`, `max_iter = 100`, `tol = 1e-6`).
///
/// Returns `0.0` if `locus` is not present in the world.
pub fn pagerank_centrality(world: &World, locus: LocusId) -> f32 {
    pagerank(world, 0.85, 100, 1e-6)
        .into_iter()
        .find(|(id, _)| *id == locus)
        .map(|(_, value)| value)
        .unwrap_or(0.0)
}

fn all_betweenness_inner(world: &World) -> Vec<(LocusId, f32)> {
    brandes::all_betweenness_inner(world)
}
