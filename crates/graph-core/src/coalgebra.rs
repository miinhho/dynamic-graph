//! Coalgebraic framing of the substrate's dynamics.
//!
//! See `docs/coalgebra.md` for the full design memo. This module is the
//! thin in-code anchor for that framing — it provides the *vocabulary*
//! (observation type, color type, encoder traits) used by the
//! behavioral-equivalence primitives in `graph-query::coalgebra`.
//!
//! ## Quick orientation
//!
//! In categorical terms, an `F`-coalgebra is a pair `(X, α: X → F(X))`
//! where `F` is an endofunctor that captures *what is observable about
//! `X` in one step*. Several layers of the substrate already have this
//! shape:
//!
//! - **Locus dynamics** (`LocusProgram::process`) — a Mealy-machine-style
//!   coalgebra: given a state and an inbox, the program emits an output
//!   (proposed changes) and produces a successor state at commit time.
//!   `α: State × Inbox → Output × State`.
//!
//! - **ChangeLog** — a coalgebra whose carrier is `ChangeId` and whose
//!   step gives `(Change, predecessors)`. `causal_ancestors` is BFS over
//!   this coalgebra, i.e. the standard *unfolding* (anamorphism) into
//!   the predecessor tree.
//!
//! - **Relationship decay/Hebbian** — an autonomous (input-free)
//!   coalgebra: `α: State → State`, applied once per batch.
//!
//! The categorical payoff is a precise notion of *equivalence*:
//! **bisimulation**. Two states `x, y : X` are bisimilar under `F` when
//! their observations match and their successors are pairwise bisimilar.
//! For the locus coalgebra this collapses to "interchangeable up to
//! observation depth". The bounded version of that check (Weisfeiler-
//! Lehman color refinement) is implemented in `graph-query::coalgebra`
//! and is the new capability this framing buys us — the rest is mostly
//! making existing structure legible.
//!
//! ## What lives here vs. graph-query
//!
//! - **Here (graph-core):** the encoder traits and color type. Pure
//!   data, no `World` dependency. Intended to be implemented or chosen
//!   by callers who want to control the granularity of "same".
//!
//! - **graph-query::coalgebra:** the bisimulation algorithm itself,
//!   which needs `&World` to walk neighbors.

use crate::ids::{InfluenceKindId, LocusKindId};
use crate::locus::Locus;
use crate::relationship::{Endpoints, Relationship};

/// Hash-bucketed identity used by the behavioral-refinement procedure.
///
/// One round of refinement assigns each locus a new `Color` derived from
/// its previous color and the multiset of `(edge color, neighbor color,
/// edge direction)` around it. Two loci share a color after `k` rounds
/// iff they are `k`-bisimilar under the chosen encoders.
///
/// Colors carry no semantic meaning across runs — they are just stable
/// equivalence-class tokens within one call.
pub type BehaviorColor = u64;

/// User-supplied encoding from a locus to its initial behavioral color.
///
/// The encoder is the "perspective" knob: it decides what counts as
/// **observably the same locus at depth 0**. Common choices:
///
/// - [`KindOnlyEncoder`] — color = locus kind. Pure topological framing;
///   two loci of the same kind are depth-0 indistinguishable regardless
///   of their numeric state.
/// - [`KindAndQuantizedStateEncoder`] — color = (kind, quantized state).
///   Treats two loci of the same kind whose state vectors agree to a
///   coarse bucket as initially equivalent.
///
/// Implement this trait directly when neither default fits the domain.
pub trait LocusEncoder {
    fn encode_locus(&self, locus: &Locus) -> BehaviorColor;
}

/// User-supplied encoding from a relationship to its edge color.
///
/// The default ([`KindOnlyEdgeEncoder`]) ignores activity/weight and
/// uses the relationship kind only — appropriate for structural
/// equivalence. Replace with a custom encoder when the domain wants to
/// distinguish, e.g., "high-weight friendship" from "low-weight
/// friendship" as different edges from the bisimulation point of view.
pub trait EdgeEncoder {
    fn encode_edge(&self, rel: &Relationship) -> BehaviorColor;
}

/// Direction of a relationship as seen from one endpoint.
///
/// Folded into the per-neighbor signature so `(A → B)` and `(B → A)`
/// are not collapsed in the directed case while `Symmetric` edges
/// observe identically from both sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeDirection {
    /// `Endpoints::Directed { from, to }` viewed from `from`.
    Outgoing,
    /// `Endpoints::Directed { from, to }` viewed from `to`.
    Incoming,
    /// `Endpoints::Symmetric { .. }` viewed from either endpoint.
    Symmetric,
}

impl EdgeDirection {
    /// Determine the direction of `endpoints` as observed from `viewer`.
    /// Returns `None` when `viewer` is not one of the endpoints.
    pub fn of(endpoints: &Endpoints, viewer: crate::ids::LocusId) -> Option<Self> {
        match endpoints {
            Endpoints::Directed { from, to } => {
                if *from == viewer {
                    Some(Self::Outgoing)
                } else if *to == viewer {
                    Some(Self::Incoming)
                } else {
                    None
                }
            }
            Endpoints::Symmetric { a, b } => {
                if *a == viewer || *b == viewer {
                    Some(Self::Symmetric)
                } else {
                    None
                }
            }
        }
    }
}

// ── Default encoders ──────────────────────────────────────────────────────

/// Color a locus by its `LocusKindId` only. Topological / kind-aware
/// equivalence: state values do not enter the coloring.
#[derive(Debug, Clone, Copy, Default)]
pub struct KindOnlyEncoder;

impl LocusEncoder for KindOnlyEncoder {
    fn encode_locus(&self, locus: &Locus) -> BehaviorColor {
        locus.kind.0
    }
}

/// Color a locus by `(kind, bucketed state)` where each state slot is
/// quantized to `step`-wide buckets via floor-division on the bit
/// pattern. Loci with state agreeing to the same bucket on every slot
/// receive the same depth-0 color.
///
/// `step` must be positive; smaller `step` = finer equivalence.
/// `0.0`-valued slots always bucket to `0` regardless of `step`.
#[derive(Debug, Clone, Copy)]
pub struct KindAndQuantizedStateEncoder {
    pub step: f32,
}

impl KindAndQuantizedStateEncoder {
    pub fn new(step: f32) -> Self {
        debug_assert!(step > 0.0, "quantization step must be positive");
        Self { step }
    }
}

impl LocusEncoder for KindAndQuantizedStateEncoder {
    fn encode_locus(&self, locus: &Locus) -> BehaviorColor {
        let mut h = fnv_seed();
        fnv_mix(&mut h, locus.kind.0);
        for (i, v) in locus.state.as_slice().iter().enumerate() {
            let bucket = if *v == 0.0 {
                0i64
            } else {
                (*v / self.step).floor() as i64
            };
            fnv_mix(&mut h, i as u64);
            fnv_mix(&mut h, bucket as u64);
        }
        h
    }
}

/// Color a relationship by its `RelationshipKindId` only.
///
/// `RelationshipKindId == InfluenceKindId` per O8 in `docs/redesign.md`.
#[derive(Debug, Clone, Copy, Default)]
pub struct KindOnlyEdgeEncoder;

impl EdgeEncoder for KindOnlyEdgeEncoder {
    fn encode_edge(&self, rel: &Relationship) -> BehaviorColor {
        let kind: InfluenceKindId = rel.kind;
        kind.0
    }
}

/// Color a relationship by `(kind, activity-bucket, weight-bucket)`.
/// Use when the bisimulation should distinguish edges by strength.
#[derive(Debug, Clone, Copy)]
pub struct KindAndStrengthEdgeEncoder {
    pub activity_step: f32,
    pub weight_step: f32,
}

impl Default for KindAndStrengthEdgeEncoder {
    fn default() -> Self {
        Self {
            activity_step: 0.1,
            weight_step: 0.1,
        }
    }
}

impl EdgeEncoder for KindAndStrengthEdgeEncoder {
    fn encode_edge(&self, rel: &Relationship) -> BehaviorColor {
        let kind: InfluenceKindId = rel.kind;
        let slots = rel.state.as_slice();
        let activity = slots.first().copied().unwrap_or(0.0);
        let weight = slots.get(1).copied().unwrap_or(0.0);
        let a_bucket = if activity == 0.0 {
            0i64
        } else {
            (activity / self.activity_step).floor() as i64
        };
        let w_bucket = if weight == 0.0 {
            0i64
        } else {
            (weight / self.weight_step).floor() as i64
        };
        let mut h = fnv_seed();
        fnv_mix(&mut h, kind.0);
        fnv_mix(&mut h, a_bucket as u64);
        fnv_mix(&mut h, w_bucket as u64);
        h
    }
}

// ── FNV-1a 64-bit, deterministic and cheap ────────────────────────────────
//
// Used internally to fold heterogeneous parts of an observation into a
// single `BehaviorColor`. Not exposed because its only role is to keep
// the encoders deterministic and free of HashMap reseeding effects.

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv_seed() -> u64 {
    FNV_OFFSET
}

fn fnv_mix(state: &mut u64, x: u64) {
    *state ^= x;
    *state = state.wrapping_mul(FNV_PRIME);
}

/// Combine a previous color, the locus's own color, and a *sorted*
/// neighborhood signature into the next round's color. Public so callers
/// who roll their own refinement loop can stay bit-identical to
/// `graph-query::coalgebra::behavioral_partition`.
pub fn fold_color(
    previous: BehaviorColor,
    own: BehaviorColor,
    neighborhood_sorted: &[(BehaviorColor, BehaviorColor, EdgeDirection)],
) -> BehaviorColor {
    let mut h = fnv_seed();
    fnv_mix(&mut h, previous);
    fnv_mix(&mut h, own);
    for (edge, neighbor, dir) in neighborhood_sorted {
        fnv_mix(&mut h, *edge);
        fnv_mix(&mut h, *neighbor);
        fnv_mix(&mut h, *dir as u64);
    }
    h
}

/// Tag for the locus kind alone, exposed for callers that want to derive
/// their own encoder by composing on top of a kind tag.
pub fn kind_color(kind: LocusKindId) -> BehaviorColor {
    kind.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::LocusId;
    use crate::state::StateVector;

    #[test]
    fn kind_only_encoder_groups_by_kind() {
        let a = Locus::new(LocusId(0), LocusKindId(7), StateVector::from_slice(&[1.0]));
        let b = Locus::new(LocusId(1), LocusKindId(7), StateVector::from_slice(&[5.0]));
        let c = Locus::new(LocusId(2), LocusKindId(8), StateVector::from_slice(&[1.0]));
        let enc = KindOnlyEncoder;
        assert_eq!(enc.encode_locus(&a), enc.encode_locus(&b));
        assert_ne!(enc.encode_locus(&a), enc.encode_locus(&c));
    }

    #[test]
    fn quantized_state_encoder_distinguishes_buckets() {
        let a = Locus::new(LocusId(0), LocusKindId(1), StateVector::from_slice(&[0.05]));
        let b = Locus::new(LocusId(1), LocusKindId(1), StateVector::from_slice(&[0.06]));
        let c = Locus::new(LocusId(2), LocusKindId(1), StateVector::from_slice(&[0.20]));
        let enc = KindAndQuantizedStateEncoder::new(0.1);
        // a and b both fall in [0.0, 0.1)
        assert_eq!(enc.encode_locus(&a), enc.encode_locus(&b));
        // c falls in [0.2, 0.3)
        assert_ne!(enc.encode_locus(&a), enc.encode_locus(&c));
    }

    #[test]
    fn fold_color_is_order_sensitive() {
        // The caller is expected to sort before folding; if they don't,
        // the result depends on order — this test pins that behavior.
        let n1 = vec![(1u64, 2u64, EdgeDirection::Outgoing)];
        let n2 = vec![(2u64, 1u64, EdgeDirection::Outgoing)];
        let a = fold_color(0, 0, &n1);
        let b = fold_color(0, 0, &n2);
        assert_ne!(a, b);
    }

    #[test]
    fn edge_direction_recognizes_endpoints() {
        let viewer = LocusId(5);
        let directed_out = Endpoints::Directed {
            from: viewer,
            to: LocusId(9),
        };
        let directed_in = Endpoints::Directed {
            from: LocusId(9),
            to: viewer,
        };
        let sym = Endpoints::Symmetric {
            a: viewer,
            b: LocusId(9),
        };
        let unrelated = Endpoints::Directed {
            from: LocusId(1),
            to: LocusId(2),
        };
        assert_eq!(
            EdgeDirection::of(&directed_out, viewer),
            Some(EdgeDirection::Outgoing)
        );
        assert_eq!(
            EdgeDirection::of(&directed_in, viewer),
            Some(EdgeDirection::Incoming)
        );
        assert_eq!(
            EdgeDirection::of(&sym, viewer),
            Some(EdgeDirection::Symmetric)
        );
        assert_eq!(EdgeDirection::of(&unrelated, viewer), None);
    }
}
