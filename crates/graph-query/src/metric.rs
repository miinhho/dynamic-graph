//! Behavioral distance over a `World` via Kantorovich-style contraction.
//!
//! See `docs/coalgebra-advanced.md` §1 for the framing and
//! `crates/graph-core/src/metric.rs` for the trait surface.
//!
//! ## Algorithm
//!
//! Given pointwise metrics `d_loc: Locus × Locus → [0,1]` and
//! `d_edge: Relationship × Relationship → [0,1]`, the behavioral
//! distance `d: LocusId × LocusId → [0,1]` is the unique fixed point
//! of
//!
//! ```text
//! d_{k+1}(x, y) = max( d_loc(x, y),
//!                      γ · matching(N(x), N(y), d_k, d_edge) )
//! ```
//!
//! This is the Desharnais–Edalat–Panangaden formulation: the pointwise
//! locus distance acts as a **hard floor** (different kinds are
//! maximally far apart, full stop), while the discounted topological
//! lift can only push the distance *up* from there. Monotone, bounded,
//! and convergent by Knaster–Tarski regardless of seed.
//!
//! `N(x)` is the multiset of `(edge, neighbor, direction)` for locus
//! `x`; `matching` is the Hausdorff lift over that multiset using a
//! fused per-neighbor distance
//!
//! ```text
//! d_pair((e_a, x_a, dir_a), (e_b, x_b, dir_b)) =
//!     1.0                                                if dir_a ≠ dir_b
//!     0.5 · d_edge(e_a, e_b) + 0.5 · d_k(x_a, x_b)        otherwise
//! ```
//!
//! and `γ ∈ (0, 1)` is the discount factor controlling how much weight
//! one batch of refinement carries vs. the seed. Banach's theorem
//! guarantees convergence regardless of starting `d_0`. We seed
//! `d_0(x, y) = d_loc(x, y)`.
//!
//! ## API
//!
//! - [`behavioral_distance`] — single point query, bounded depth.
//! - [`behavioral_distance_fixpoint`] — iterates until `max |Δd| < ε`
//!   or `max_rounds` reached. Always returns the latest estimate.
//! - [`MetricOptions`] — controls discount, depth, and metrics.
//!
//! Memoization keys are unordered locus pairs (`(min, max)`), so the
//! cost is `O(rounds × pairs_visited × Δ²)`. For full pairwise on a
//! large world prefer the partition primitive in `coalgebra` then
//! refine within each class.

use std::cmp::Ordering;

use graph_core::{
    EdgeDirection, EdgeMetric, KindAndStateMetric, KindOnlyEdgeMetric, LocusId, LocusMetric,
    Relationship, hausdorff_distance,
};
use graph_world::World;
use rustc_hash::FxHashMap;

/// Options for the contraction-fixpoint algorithm.
///
/// `discount ∈ (0, 1]` is the contraction factor on the recursive
/// (lifted) term. Smaller values pull distances toward the pointwise
/// floor; larger values let topology contribute more. The pointwise
/// locus metric always acts as a hard lower bound — `d ≥ d_loc(x, y)`
/// at every iteration — so two loci of different kinds always have
/// distance `1.0` regardless of structure.
///
/// `max_rounds` caps the iteration even if the fixpoint hasn't been
/// reached. `epsilon` is the early-termination threshold on the
/// largest absolute change between successive rounds.
#[derive(Debug, Clone)]
pub struct MetricOptions<L: LocusMetric, E: EdgeMetric> {
    pub discount: f64,
    pub max_rounds: u32,
    pub epsilon: f64,
    pub locus_metric: L,
    pub edge_metric: E,
}

impl Default for MetricOptions<KindAndStateMetric, KindOnlyEdgeMetric> {
    fn default() -> Self {
        Self {
            discount: 0.5,
            max_rounds: 16,
            epsilon: 1e-4,
            locus_metric: KindAndStateMetric::default(),
            edge_metric: KindOnlyEdgeMetric,
        }
    }
}

/// Compute the behavioral distance between `a` and `b` to depth
/// `opts.max_rounds`, returning `None` if either locus is missing.
///
/// Uses a depth-limited recursion with a `(LocusId, LocusId) → f64`
/// memo so each pair is computed once per round. For repeated queries
/// over the same world prefer [`behavioral_distance_fixpoint`].
pub fn behavioral_distance<L: LocusMetric, E: EdgeMetric>(
    world: &World,
    a: LocusId,
    b: LocusId,
    opts: &MetricOptions<L, E>,
) -> Option<f64> {
    if world.locus(a).is_none() || world.locus(b).is_none() {
        return None;
    }
    let mut memo = FxHashMap::default();
    Some(distance_recursive(
        world,
        a,
        b,
        opts.max_rounds,
        opts,
        &mut memo,
    ))
}

/// Iterate the contraction map to fixpoint and return the resulting
/// distance between `a` and `b`. Stops when the maximum change across
/// the locus pair table drops below `opts.epsilon`, or after
/// `opts.max_rounds`. Returns `None` if either locus is missing.
///
/// The fixpoint is an estimate of `d(a, b)` for the full World.
/// Internally it materializes the full `O(V²)` distance table for
/// the candidate-equivalent pairs (loci of matching kind), so this is
/// not appropriate for V > a few thousand. For larger worlds first
/// partition with `coalgebra::behavioral_partition` and then call this
/// per class.
pub fn behavioral_distance_fixpoint<L: LocusMetric, E: EdgeMetric>(
    world: &World,
    a: LocusId,
    b: LocusId,
    opts: &MetricOptions<L, E>,
) -> Option<f64> {
    if world.locus(a).is_none() || world.locus(b).is_none() {
        return None;
    }

    let kinds: FxHashMap<LocusId, _> = world.loci().iter().map(|l| (l.id, l.kind)).collect();

    // Initialize d_0(x, y) = d_loc(x, y) for all matching-kind pairs.
    let mut current: FxHashMap<(LocusId, LocusId), f64> = FxHashMap::default();
    let ids: Vec<LocusId> = kinds.keys().copied().collect();
    for i in 0..ids.len() {
        for j in i..ids.len() {
            let x = ids[i];
            let y = ids[j];
            if kinds[&x] != kinds[&y] {
                continue;
            }
            let dx = world.locus(x).unwrap();
            let dy = world.locus(y).unwrap();
            current.insert(pair_key(x, y), opts.locus_metric.locus_distance(dx, dy));
        }
    }

    for _ in 0..opts.max_rounds {
        let mut next: FxHashMap<(LocusId, LocusId), f64> =
            FxHashMap::with_capacity_and_hasher(current.len(), Default::default());
        let mut max_delta = 0.0_f64;
        for (key, _) in current.iter() {
            let (x, y) = (key.0, key.1);
            let new_d = step_distance(world, x, y, opts, &current);
            let prev = current.get(key).copied().unwrap_or(0.0);
            max_delta = max_delta.max((new_d - prev).abs());
            next.insert(*key, new_d);
        }
        current = next;
        if max_delta < opts.epsilon {
            break;
        }
    }

    let key = pair_key(a, b);
    current.get(&key).copied().or_else(|| {
        // a and b have different kinds → they were never inserted.
        // Fall back to direct pointwise distance.
        Some(
            opts.locus_metric
                .locus_distance(world.locus(a)?, world.locus(b)?),
        )
    })
}

// ── Internal helpers ─────────────────────────────────────────────────────

fn pair_key(a: LocusId, b: LocusId) -> (LocusId, LocusId) {
    if a <= b { (a, b) } else { (b, a) }
}

fn distance_recursive<L: LocusMetric, E: EdgeMetric>(
    world: &World,
    a: LocusId,
    b: LocusId,
    rounds: u32,
    opts: &MetricOptions<L, E>,
    memo: &mut FxHashMap<(LocusId, LocusId, u32), f64>,
) -> f64 {
    if let (Some(la), Some(lb)) = (world.locus(a), world.locus(b)) {
        let pointwise = opts.locus_metric.locus_distance(la, lb);
        if rounds == 0 {
            return pointwise;
        }
        let key = (pair_key(a, b).0, pair_key(a, b).1, rounds);
        if let Some(&cached) = memo.get(&key) {
            return cached;
        }
        // Place a placeholder to break cycles — using the pointwise as
        // an under-approximation. A cycle returning here will read this
        // value; subsequent rounds tighten the bound.
        memo.insert(key, pointwise);

        let n_a = collect_neighborhood(world, a);
        let n_b = collect_neighborhood(world, b);
        let lifted = matching_distance(
            &n_a,
            &n_b,
            opts,
            &mut |x: LocusId, y: LocusId| -> f64 {
                distance_recursive(world, x, y, rounds - 1, opts, memo)
            },
        );
        let combined = pointwise.max(opts.discount * lifted).min(1.0);
        memo.insert(key, combined);
        combined
    } else {
        1.0
    }
}

fn step_distance<L: LocusMetric, E: EdgeMetric>(
    world: &World,
    a: LocusId,
    b: LocusId,
    opts: &MetricOptions<L, E>,
    table: &FxHashMap<(LocusId, LocusId), f64>,
) -> f64 {
    let la = match world.locus(a) {
        Some(l) => l,
        None => return 1.0,
    };
    let lb = match world.locus(b) {
        Some(l) => l,
        None => return 1.0,
    };
    let pointwise = opts.locus_metric.locus_distance(la, lb);
    let n_a = collect_neighborhood(world, a);
    let n_b = collect_neighborhood(world, b);
    let lifted = matching_distance(&n_a, &n_b, opts, &mut |x, y| {
        let kx = world.locus(x).map(|l| l.kind);
        let ky = world.locus(y).map(|l| l.kind);
        if kx != ky {
            return 1.0;
        }
        let key = pair_key(x, y);
        table.get(&key).copied().unwrap_or(pointwise)
    });
    pointwise.max(opts.discount * lifted).min(1.0)
}

#[derive(Clone)]
struct NeighborTriple<'a> {
    edge: &'a Relationship,
    neighbor: LocusId,
    direction: EdgeDirection,
}

fn collect_neighborhood<'a>(world: &'a World, locus: LocusId) -> Vec<NeighborTriple<'a>> {
    let mut out = Vec::new();
    for rel in world.relationships_for_locus(locus) {
        let other = rel.endpoints.other_than(locus);
        if let Some(dir) = EdgeDirection::of(&rel.endpoints, locus) {
            out.push(NeighborTriple {
                edge: rel,
                neighbor: other,
                direction: dir,
            });
        }
    }
    out
}

/// Hausdorff matching distance between two neighborhoods using the
/// fused per-pair metric `d_pair = 0.5 · d_edge + 0.5 · d_neighbor`,
/// with mismatched directions counted as distance 1.
fn matching_distance<L: LocusMetric, E: EdgeMetric>(
    a: &[NeighborTriple<'_>],
    b: &[NeighborTriple<'_>],
    opts: &MetricOptions<L, E>,
    neighbor_dist: &mut dyn FnMut(LocusId, LocusId) -> f64,
) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    if a.is_empty() || b.is_empty() {
        return 1.0;
    }
    // Inline Hausdorff so we can call the FnMut neighbor_dist closure
    // (the public hausdorff_distance takes Fn, not FnMut).
    let pair_d = |x: &NeighborTriple<'_>,
                  y: &NeighborTriple<'_>,
                  nd: &mut dyn FnMut(LocusId, LocusId) -> f64|
     -> f64 {
        if x.direction != y.direction {
            return 1.0;
        }
        let de = opts.edge_metric.edge_distance(x.edge, y.edge);
        let dn = nd(x.neighbor, y.neighbor);
        0.5 * de + 0.5 * dn
    };
    let one_way = |from: &[NeighborTriple<'_>],
                   to: &[NeighborTriple<'_>],
                   nd: &mut dyn FnMut(LocusId, LocusId) -> f64|
     -> f64 {
        from.iter()
            .map(|x| {
                to.iter()
                    .map(|y| pair_d(x, y, nd))
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0.0_f64, f64::max)
    };
    let ab = one_way(a, b, neighbor_dist);
    let ba = one_way(b, a, neighbor_dist);
    match ab.partial_cmp(&ba) {
        Some(Ordering::Greater) => ab,
        _ => ba,
    }
}

// `hausdorff_distance` is re-exported from graph-core; we only need it
// inside docs. Suppress unused-import warnings.
#[allow(dead_code)]
fn _hausdorff_anchor() -> f64 {
    hausdorff_distance::<f64, _>(&[], &[], |a, b| (a - b).abs())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId, Relationship,
        RelationshipKindId, RelationshipLineage, StateVector,
    };

    fn make_locus(world: &mut World, id: u64, kind: u64, state: &[f32]) {
        world.insert_locus(Locus::new(
            LocusId(id),
            LocusKindId(kind),
            StateVector::from_slice(state),
        ));
    }

    fn link(world: &mut World, from: u64, to: u64, kind: u64) {
        let rid = world.relationships_mut().mint_id();
        let kind_id: RelationshipKindId = InfluenceKindId(kind);
        world.relationships_mut().insert(Relationship {
            id: rid,
            kind: kind_id,
            endpoints: Endpoints::Directed {
                from: LocusId(from),
                to: LocusId(to),
            },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(kind_id)],
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
    }

    #[test]
    fn distance_to_self_is_zero() {
        let mut w = World::new();
        make_locus(&mut w, 1, 1, &[0.5]);
        let opts = MetricOptions::default();
        let d = behavioral_distance(&w, LocusId(1), LocusId(1), &opts).unwrap();
        assert!(d.abs() < 1e-9, "self-distance must be 0, got {d}");
    }

    #[test]
    fn distance_is_symmetric() {
        let mut w = World::new();
        make_locus(&mut w, 1, 1, &[0.0]);
        make_locus(&mut w, 2, 1, &[0.5]);
        let opts = MetricOptions::default();
        let d12 = behavioral_distance(&w, LocusId(1), LocusId(2), &opts).unwrap();
        let d21 = behavioral_distance(&w, LocusId(2), LocusId(1), &opts).unwrap();
        assert!((d12 - d21).abs() < 1e-9);
    }

    #[test]
    fn different_kinds_are_maximally_apart() {
        let mut w = World::new();
        make_locus(&mut w, 1, 1, &[0.5]);
        make_locus(&mut w, 2, 2, &[0.5]);
        let opts = MetricOptions::default();
        let d = behavioral_distance(&w, LocusId(1), LocusId(2), &opts).unwrap();
        assert_eq!(d, 1.0);
    }

    #[test]
    fn two_isomorphic_chains_collapse_to_zero_distance() {
        // Build two parallel chains of length 3 with identical state.
        // After fixpoint iteration, paired loci (heads/middles/tails)
        // should have distance 0.
        let mut w = World::new();
        for i in 1..=6 {
            make_locus(&mut w, i, 1, &[0.0]);
        }
        link(&mut w, 1, 2, 1);
        link(&mut w, 2, 3, 1);
        link(&mut w, 4, 5, 1);
        link(&mut w, 5, 6, 1);
        let opts = MetricOptions {
            discount: 0.5,
            max_rounds: 32,
            epsilon: 1e-6,
            ..Default::default()
        };
        // heads
        let d_heads = behavioral_distance_fixpoint(&w, LocusId(1), LocusId(4), &opts).unwrap();
        // tails
        let d_tails = behavioral_distance_fixpoint(&w, LocusId(3), LocusId(6), &opts).unwrap();
        assert!(d_heads < 1e-3, "isomorphic heads: got {d_heads}");
        assert!(d_tails < 1e-3, "isomorphic tails: got {d_tails}");
    }

    #[test]
    fn distinct_neighborhoods_separate_loci_quantitatively() {
        // 1 has an out-edge to a state=10 sink, 2 has none. Distance
        // should be strictly positive.
        let mut w = World::new();
        make_locus(&mut w, 1, 1, &[0.0]);
        make_locus(&mut w, 2, 1, &[0.0]);
        make_locus(&mut w, 3, 1, &[10.0]);
        link(&mut w, 1, 3, 1);
        let opts = MetricOptions {
            discount: 0.5,
            max_rounds: 8,
            epsilon: 1e-6,
            ..Default::default()
        };
        let d = behavioral_distance(&w, LocusId(1), LocusId(2), &opts).unwrap();
        assert!(d > 0.0, "neighborhood mismatch should yield d > 0, got {d}");
    }

    #[test]
    fn missing_locus_returns_none() {
        let w = World::new();
        let opts = MetricOptions::default();
        assert!(behavioral_distance(&w, LocusId(99), LocusId(100), &opts).is_none());
    }
}
