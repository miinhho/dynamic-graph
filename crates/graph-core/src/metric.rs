//! Behavioral metric primitives — quantitative analogue of bisimulation.
//!
//! See `docs/coalgebra-advanced.md` §1 for the full categorical framing.
//! Briefly: classical bisimulation answers a yes/no question ("are these
//! two states observationally equivalent?"). A **behavioral metric**
//! answers a real-valued one ("how far apart are these two states in
//! observable behavior?"). The metric is the canonical replacement for
//! ad-hoc thresholds (`min_bridge_activity`, `min_activity_threshold`,
//! the Davis/Karate calibration knobs) — instead of cutting a binary
//! relation at a hand-chosen value, you carry the distance through and
//! cut later (or not at all).
//!
//! ## Categorical anchor
//!
//! Following Desharnais–Edalat–Panangaden (TCS 2004) and Bonchi–König–
//! Petrişan (CONCUR 2018), a behavioral metric on an `F`-coalgebra
//! `(X, α)` is a function `d: X × X → [0, 1]` that is the unique
//! fixed point of a contraction map of the form
//!
//! ```text
//! d_{k+1}(x, y) = (1 − γ) · d_loc(x, y)
//!               + γ · lift_F(d_k)(α(x), α(y))
//! ```
//!
//! where `lift_F` is the **Kantorovich lifting** of `d` to `F(X)`
//! and `γ ∈ (0, 1)` is a discount factor. Banach's theorem guarantees
//! convergence regardless of seed.
//!
//! For our deterministic Mealy coalgebra, `lift_F` reduces to a
//! Hausdorff-style distance over the multiset of `(edge, neighbor,
//! direction)` triples around each locus. The runtime algorithm lives
//! in `graph-query::metric`; this module ships the trait surface plus
//! standard concrete metrics.
//!
//! ## Relationship to `coalgebra` module
//!
//! The encoders in `coalgebra` produce *colors* (discrete equivalence-
//! class tokens). Metrics produce *distances*. Both are perspective-
//! parameterized, but the metric is strictly more expressive — you can
//! recover the partition at any threshold by cutting `d`, but the
//! reverse is not possible.

use crate::locus::Locus;
use crate::relationship::Relationship;

/// User-supplied pointwise distance between two loci.
///
/// Required to satisfy the metric axioms — `d(x, x) = 0`, symmetry,
/// triangle inequality — so that the lifted bisimulation distance
/// inherits them.  Implementations should map distances into `[0, 1]`
/// so the contraction lifting in `graph-query::metric` stays bounded.
///
/// **Conventional choice for two loci of different `LocusKindId`:**
/// return `1.0`. Different kinds run different programs and are
/// considered maximally far apart by default. Override only if the
/// domain genuinely treats kinds as comparable (rare).
pub trait LocusMetric {
    fn locus_distance(&self, a: &Locus, b: &Locus) -> f64;
}

/// User-supplied pointwise distance between two relationships.
///
/// Same axioms, same `[0, 1]` convention. Edges of different kinds
/// default to distance `1.0` in the standard implementations.
pub trait EdgeMetric {
    fn edge_distance(&self, a: &Relationship, b: &Relationship) -> f64;
}

// ── Standard metrics ─────────────────────────────────────────────────────

/// Discrete metric on locus kind alone — `d(a, b) = 0` if same kind,
/// `1` otherwise. Pairs naturally with `coalgebra::KindOnlyEncoder`
/// (the metric is a relaxation of the binary equivalence).
#[derive(Debug, Clone, Copy, Default)]
pub struct KindOnlyMetric;

impl LocusMetric for KindOnlyMetric {
    fn locus_distance(&self, a: &Locus, b: &Locus) -> f64 {
        if a.kind == b.kind { 0.0 } else { 1.0 }
    }
}

/// Locus distance = `1.0` if kinds differ, else a normalized Euclidean
/// distance on the state vectors clamped to `[0, 1]`.
///
/// `state_scale` controls the normalization: distances are divided by
/// `state_scale` and clamped, so `state_scale` should be the largest
/// state-vector L2 distance you want to count as fully "different".
/// Loci of identical state under matching kind have distance 0; loci
/// whose state differs by `>= state_scale` cap at 1.0.
#[derive(Debug, Clone, Copy)]
pub struct KindAndStateMetric {
    pub state_scale: f64,
}

impl Default for KindAndStateMetric {
    fn default() -> Self {
        Self { state_scale: 1.0 }
    }
}

impl LocusMetric for KindAndStateMetric {
    fn locus_distance(&self, a: &Locus, b: &Locus) -> f64 {
        if a.kind != b.kind {
            return 1.0;
        }
        let raw = a.state.euclidean_distance(&b.state) as f64;
        (raw / self.state_scale).clamp(0.0, 1.0)
    }
}

/// Discrete metric on relationship kind. Pairs with
/// `coalgebra::KindOnlyEdgeEncoder`.
#[derive(Debug, Clone, Copy, Default)]
pub struct KindOnlyEdgeMetric;

impl EdgeMetric for KindOnlyEdgeMetric {
    fn edge_distance(&self, a: &Relationship, b: &Relationship) -> f64 {
        if a.kind == b.kind { 0.0 } else { 1.0 }
    }
}

/// Edge distance = `1.0` if kinds differ; else weighted blend of
/// activity-difference and weight-difference.
///
/// `activity_scale`, `weight_scale` normalize each axis as in
/// [`KindAndStateMetric`]. The blend uses `activity_blend ∈ [0, 1]`
/// with the rest going to the weight axis.
#[derive(Debug, Clone, Copy)]
pub struct KindAndStrengthEdgeMetric {
    pub activity_scale: f64,
    pub weight_scale: f64,
    pub activity_blend: f64,
}

impl Default for KindAndStrengthEdgeMetric {
    fn default() -> Self {
        Self {
            activity_scale: 1.0,
            weight_scale: 1.0,
            activity_blend: 0.5,
        }
    }
}

impl EdgeMetric for KindAndStrengthEdgeMetric {
    fn edge_distance(&self, a: &Relationship, b: &Relationship) -> f64 {
        if a.kind != b.kind {
            return 1.0;
        }
        let slots_a = a.state.as_slice();
        let slots_b = b.state.as_slice();
        let act_a = slots_a.first().copied().unwrap_or(0.0) as f64;
        let act_b = slots_b.first().copied().unwrap_or(0.0) as f64;
        let w_a = slots_a.get(1).copied().unwrap_or(0.0) as f64;
        let w_b = slots_b.get(1).copied().unwrap_or(0.0) as f64;
        let d_act = ((act_a - act_b).abs() / self.activity_scale).clamp(0.0, 1.0);
        let d_w = ((w_a - w_b).abs() / self.weight_scale).clamp(0.0, 1.0);
        let blend = self.activity_blend.clamp(0.0, 1.0);
        blend * d_act + (1.0 - blend) * d_w
    }
}

// ── Hausdorff lift over a finite multiset ────────────────────────────────

/// Hausdorff-style asymmetric distance between two finite sequences.
///
/// `d(seq_a, seq_b) = max(seq_a → seq_b, seq_b → seq_a)`,
/// where each direction is `sup_x inf_y d(x, y)`.
///
/// This is the standard finitary Kantorovich lifting for the
/// nondeterministic / multiset functor: it is the largest distance
/// such that every element of `seq_a` is within that distance of some
/// element of `seq_b`, and vice versa. Returns `0.0` when both
/// sequences are empty.
///
/// Public so callers can reuse it for arbitrary multisets — useful when
/// implementing a custom metric over relationship slots, sediment
/// layers, or other "bag of observations" structures.
pub fn hausdorff_distance<T, F>(a: &[T], b: &[T], pointwise: F) -> f64
where
    F: Fn(&T, &T) -> f64,
{
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    if a.is_empty() || b.is_empty() {
        return 1.0;
    }
    let one_way = |from: &[T], to: &[T]| -> f64 {
        from.iter()
            .map(|x| {
                to.iter()
                    .map(|y| pointwise(x, y))
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0.0_f64, f64::max)
    };
    one_way(a, b).max(one_way(b, a))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{InfluenceKindId, LocusId, LocusKindId};
    use crate::relationship::{Endpoints, KindObservation, RelationshipId, RelationshipLineage};
    use crate::state::StateVector;

    fn lc(id: u64, kind: u64, state: &[f32]) -> Locus {
        Locus::new(LocusId(id), LocusKindId(kind), StateVector::from_slice(state))
    }

    fn rel(kind: u64, state: &[f32]) -> Relationship {
        Relationship {
            id: RelationshipId(0),
            kind: InfluenceKindId(kind),
            endpoints: Endpoints::Symmetric {
                a: LocusId(0),
                b: LocusId(1),
            },
            state: StateVector::from_slice(state),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(InfluenceKindId(kind))],
            },
            created_batch: crate::ids::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        }
    }

    #[test]
    fn kind_only_is_zero_on_same_kind() {
        let m = KindOnlyMetric;
        assert_eq!(m.locus_distance(&lc(0, 1, &[]), &lc(1, 1, &[])), 0.0);
        assert_eq!(m.locus_distance(&lc(0, 1, &[]), &lc(1, 2, &[])), 1.0);
    }

    #[test]
    fn kind_and_state_is_clamped_to_unit_interval() {
        let m = KindAndStateMetric { state_scale: 1.0 };
        let d = m.locus_distance(&lc(0, 1, &[0.0]), &lc(1, 1, &[10.0]));
        assert!(d <= 1.0);
        assert!(d >= 0.0);
        // exactly equal → 0
        assert_eq!(m.locus_distance(&lc(0, 1, &[0.5]), &lc(1, 1, &[0.5])), 0.0);
    }

    #[test]
    fn metric_is_symmetric() {
        let m = KindAndStateMetric { state_scale: 2.0 };
        let a = lc(0, 1, &[0.3]);
        let b = lc(1, 1, &[0.7]);
        assert!((m.locus_distance(&a, &b) - m.locus_distance(&b, &a)).abs() < 1e-9);
    }

    #[test]
    fn metric_satisfies_triangle_for_state() {
        // For a Euclidean-derived metric on 1-D states, triangle
        // inequality should hold up to the clamp.
        let m = KindAndStateMetric { state_scale: 10.0 };
        let a = lc(0, 1, &[0.0]);
        let b = lc(1, 1, &[1.0]);
        let c = lc(2, 1, &[2.5]);
        let dab = m.locus_distance(&a, &b);
        let dbc = m.locus_distance(&b, &c);
        let dac = m.locus_distance(&a, &c);
        assert!(dac <= dab + dbc + 1e-9, "triangle: {dac} <= {dab} + {dbc}");
    }

    #[test]
    fn hausdorff_zero_on_identical_sequences() {
        let a = [1.0_f64, 2.0, 3.0];
        let b = [1.0_f64, 2.0, 3.0];
        let d = hausdorff_distance(&a, &b, |x, y| (x - y).abs());
        assert_eq!(d, 0.0);
    }

    #[test]
    fn hausdorff_picks_furthest_pair() {
        let a = [0.0_f64];
        let b = [0.0_f64, 1.0];
        // a→b: inf for 0 is 0. b→a: max(inf for 0, inf for 1) = max(0, 1) = 1.
        let d = hausdorff_distance(&a, &b, |x, y| (x - y).abs());
        assert_eq!(d, 1.0);
    }

    #[test]
    fn hausdorff_empty_handling() {
        let empty: [f64; 0] = [];
        let one = [0.5_f64];
        assert_eq!(
            hausdorff_distance(&empty, &empty, |x, y: &f64| (x - y).abs()),
            0.0
        );
        assert_eq!(hausdorff_distance(&empty, &one, |x, y| (x - y).abs()), 1.0);
        assert_eq!(hausdorff_distance(&one, &empty, |x, y| (x - y).abs()), 1.0);
    }

    #[test]
    fn edge_metric_distinguishes_kind() {
        let m = KindOnlyEdgeMetric;
        assert_eq!(m.edge_distance(&rel(1, &[1.0, 0.0]), &rel(1, &[0.0, 1.0])), 0.0);
        assert_eq!(m.edge_distance(&rel(1, &[1.0, 0.0]), &rel(2, &[1.0, 0.0])), 1.0);
    }

    #[test]
    fn edge_strength_blend_combines_axes() {
        let m = KindAndStrengthEdgeMetric {
            activity_scale: 1.0,
            weight_scale: 1.0,
            activity_blend: 0.5,
        };
        let d = m.edge_distance(&rel(1, &[0.0, 0.0]), &rel(1, &[1.0, 1.0]));
        assert!((d - 1.0).abs() < 1e-9);
        let d = m.edge_distance(&rel(1, &[0.0, 0.0]), &rel(1, &[1.0, 0.0]));
        assert!((d - 0.5).abs() < 1e-9);
    }
}
