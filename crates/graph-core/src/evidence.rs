//! Weighted-coalgebra primitives — a Kleisli-style evidence layer.
//!
//! See `docs/coalgebra-advanced.md` §5 for the categorical framing.
//! Briefly: a *weighted coalgebra* is a coalgebra `α: X → T(F(X))`
//! where `T` is a strong monad capturing accumulation (sum, max,
//! probability, …). Compared to a plain coalgebra, the same `F`-shape
//! is decorated with a weight whose semantics is dictated by `T`.
//!
//! In this codebase the next big consumer is **`EmergenceEvidence`**
//! (memory: `project_emergence_trigger_roadmap.md`, Phase 0–5). When
//! Phase 1 elevates emerge atom to a first-class evidence record, the
//! correct shape is `(observation, weight) ∈ T(Observation)` for some
//! `T`. The choice of `T` is a domain decision — sum-of-evidence,
//! max-of-evidence, and probabilistic-evidence give qualitatively
//! different aggregation behavior, and the lifting of bisimulation to
//! the weighted setting differs accordingly.
//!
//! This module ships the *infrastructure* — the monoid trait, several
//! standard monoids satisfying the laws, and a `WeightedObservation`
//! container — so when Phase 1 lands the evidence type is a one-liner
//! (`type EmergenceEvidence = WeightedObservation<EmergenceAtom, _>`).
//! No coupling to the unfinished feature is introduced here.
//!
//! ## Categorical anchor
//!
//! - Bonchi–Bonsangue–Rutten, *A Coalgebraic Framework for Linear
//!   Weighted Automata*, IC 2009.
//! - Hasuo–Jacobs–Sokolova, *Generic Trace Semantics via Coinduction*,
//!   LMCS 2007 — monadic trace semantics on Kleisli categories.
//! - Bonchi–Sokolova–Vignudelli, *The theory of traces for systems
//!   with nondeterminism, probability, and termination*, CONCUR 2019.
//!
//! ## What this module does NOT include
//!
//! - The evidence atom itself — that lives in the emergence module
//!   when Phase 1+ work begins.
//! - Specific aggregation policies (sum-evidence-with-decay etc.) —
//!   those are domain decisions made at the consumer site.
//! - Probabilistic / measure-theoretic monads — `ProbProductMonoid`
//!   below is a stand-in for "weights on a probability simplex" but
//!   does not enforce normalization.

/// A monoid `(M, ⊕, e)` is the algebraic structure that any evidence
/// weight must satisfy:
///
/// 1. **Associativity:** `(a ⊕ b) ⊕ c = a ⊕ (b ⊕ c)` for all `a, b, c`.
/// 2. **Identity:** `a ⊕ identity() = identity() ⊕ a = a`.
///
/// Implementations should be `Copy` and cheap to combine — these
/// operations run on every evidence accumulation.
///
/// The associated `T` monad in the categorical sense is
/// `T(X) = X × M`. `combine` is the multiplication of `T`; the
/// `WeightedObservation` newtype below is `T(Observation)` with the
/// monad operations exposed.
pub trait EvidenceMonoid: Copy + PartialEq {
    fn identity() -> Self;
    fn combine(self, other: Self) -> Self;
}

// ── Standard monoids ─────────────────────────────────────────────────────

/// Additive evidence on `f64`. The standard "count weighted by
/// strength" semantics — useful when evidence is independent and
/// reinforces.
///
/// Identity: `0.0`. Combine: addition.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct SumF64(pub f64);

impl EvidenceMonoid for SumF64 {
    fn identity() -> Self {
        Self(0.0)
    }
    fn combine(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

/// Maximum evidence on `f64`. Useful when only the strongest single
/// signal matters (e.g. peak co-firing strength).
///
/// Identity: `f64::NEG_INFINITY`. Combine: `max`.
///
/// **NaN handling:** `combine` propagates NaN by treating it as the
/// identity. Don't feed NaN-bearing inputs unless you mean it.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct MaxF64(pub f64);

impl EvidenceMonoid for MaxF64 {
    fn identity() -> Self {
        Self(f64::NEG_INFINITY)
    }
    fn combine(self, other: Self) -> Self {
        if self.0.is_nan() {
            return other;
        }
        if other.0.is_nan() {
            return self;
        }
        Self(self.0.max(other.0))
    }
}

/// Minimum evidence on `f64`. Dual of `MaxF64`.
///
/// Identity: `f64::INFINITY`. Combine: `min`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct MinF64(pub f64);

impl EvidenceMonoid for MinF64 {
    fn identity() -> Self {
        Self(f64::INFINITY)
    }
    fn combine(self, other: Self) -> Self {
        if self.0.is_nan() {
            return other;
        }
        if other.0.is_nan() {
            return self;
        }
        Self(self.0.min(other.0))
    }
}

/// Bounded additive evidence on `f64` clamped to `[0, cap]`.
///
/// Use when evidence saturates — adding more past the cap shouldn't
/// matter. `cap > 0`. The identity is `0.0`. Combine: `(a + b).min(cap)`.
///
/// **Note on monoid law:** strict associativity holds because the
/// clamp commutes with addition for non-negative inputs; for inputs
/// with mixed signs the law can fail at the cap, so prefer using this
/// only on non-negative weights. The included tests cover the
/// non-negative case.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct BoundedSumF64 {
    pub value: f64,
    pub cap: f64,
}

impl BoundedSumF64 {
    pub fn new(value: f64, cap: f64) -> Self {
        debug_assert!(cap >= 0.0, "BoundedSumF64 cap must be non-negative");
        Self {
            value: value.clamp(0.0, cap),
            cap,
        }
    }
}

impl EvidenceMonoid for BoundedSumF64 {
    fn identity() -> Self {
        Self {
            value: 0.0,
            cap: f64::INFINITY,
        }
    }
    fn combine(self, other: Self) -> Self {
        // The cap follows whichever side carries a finite cap; if both,
        // take the smaller (more restrictive). This makes identity-with-
        // ∞-cap left-and-right neutral.
        let cap = self.cap.min(other.cap);
        Self {
            value: (self.value + other.value).clamp(0.0, cap),
            cap,
        }
    }
}

/// Probability-style evidence: combine = pointwise product. Identity
/// is `1.0`. Use for "independent observations multiply" semantics.
///
/// Does *not* enforce that values lie in `[0, 1]` — caller's job. The
/// included tests cover the `[0, 1]` case for monoid laws.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct ProbProductMonoid(pub f64);

impl EvidenceMonoid for ProbProductMonoid {
    fn identity() -> Self {
        Self(1.0)
    }
    fn combine(self, other: Self) -> Self {
        Self(self.0 * other.0)
    }
}

// ── WeightedObservation = the Kleisli-monad container ────────────────────

/// `T(Observation) = (Observation, Weight)`. The Kleisli-monad return
/// is `WeightedObservation::pure`; bind lifts a weighted observation
/// through a pure observation map.
///
/// Future consumers (e.g. `EmergenceEvidence`) instantiate this with
/// their concrete observation type and a chosen monoid:
///
/// ```rust
/// # use graph_core::evidence::{WeightedObservation, SumF64};
/// // Hypothetical use site once Phase 1+ ships:
/// // type EmergenceEvidence = WeightedObservation<EmergenceAtom, SumF64>;
/// let ev = WeightedObservation::pure((), SumF64(1.0));
/// let stronger = ev.combine_with(WeightedObservation::pure((), SumF64(2.5)), |_, b| b);
/// assert_eq!(stronger.weight, SumF64(3.5));
/// ```
///
/// `combine_with` is the natural binary operation: the user supplies
/// how to combine the *observations* (often projection or merge) and
/// the monoid combines the weights.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeightedObservation<T, W: EvidenceMonoid> {
    pub observation: T,
    pub weight: W,
}

impl<T, W: EvidenceMonoid> WeightedObservation<T, W> {
    /// Constructor — also the Kleisli monad's `return`.
    pub fn pure(observation: T, weight: W) -> Self {
        Self {
            observation,
            weight,
        }
    }

    /// Combine two weighted observations.
    ///
    /// `merge_obs` is supplied by the caller — there is no canonical
    /// way to merge arbitrary `T`. Common choices: `|a, _| a` (keep
    /// left), `|_, b| b` (keep right), or a domain-specific union.
    ///
    /// The weight is combined via the monoid.
    pub fn combine_with<F>(self, other: Self, merge_obs: F) -> Self
    where
        F: FnOnce(T, T) -> T,
    {
        Self {
            observation: merge_obs(self.observation, other.observation),
            weight: self.weight.combine(other.weight),
        }
    }

    /// Lift a pure observation map through the weighted container.
    /// Functorial part of the Kleisli structure.
    pub fn map_observation<U, F>(self, f: F) -> WeightedObservation<U, W>
    where
        F: FnOnce(T) -> U,
    {
        WeightedObservation {
            observation: f(self.observation),
            weight: self.weight,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_associative<W: EvidenceMonoid + std::fmt::Debug>(a: W, b: W, c: W) {
        let lhs = a.combine(b).combine(c);
        let rhs = a.combine(b.combine(c));
        assert_eq!(
            lhs, rhs,
            "associativity must hold for {a:?} ⊕ {b:?} ⊕ {c:?}"
        );
    }

    fn assert_identity<W: EvidenceMonoid + std::fmt::Debug>(a: W) {
        let id = W::identity();
        assert_eq!(a.combine(id), a, "right-identity for {a:?}");
        assert_eq!(id.combine(a), a, "left-identity for {a:?}");
    }

    #[test]
    fn sum_f64_satisfies_monoid_laws() {
        let a = SumF64(1.5);
        let b = SumF64(2.5);
        let c = SumF64(-0.7);
        assert_associative(a, b, c);
        assert_identity(a);
        assert_identity(SumF64(0.0));
    }

    #[test]
    fn max_f64_satisfies_monoid_laws() {
        let a = MaxF64(0.3);
        let b = MaxF64(2.7);
        let c = MaxF64(-1.0);
        assert_associative(a, b, c);
        assert_identity(a);
    }

    #[test]
    fn min_f64_satisfies_monoid_laws() {
        let a = MinF64(0.3);
        let b = MinF64(2.7);
        let c = MinF64(-1.0);
        assert_associative(a, b, c);
        assert_identity(a);
    }

    #[test]
    fn bounded_sum_f64_associative_for_non_negative_inputs() {
        // The claim in the doc-comment: associative for non-negative weights.
        let a = BoundedSumF64::new(0.4, 1.0);
        let b = BoundedSumF64::new(0.5, 1.0);
        let c = BoundedSumF64::new(0.3, 1.0);
        assert_associative(a, b, c);
    }

    #[test]
    fn bounded_sum_clamps_at_cap() {
        let a = BoundedSumF64::new(0.7, 1.0);
        let b = BoundedSumF64::new(0.8, 1.0);
        let combined = a.combine(b);
        assert_eq!(combined.value, 1.0);
        assert_eq!(combined.cap, 1.0);
    }

    #[test]
    fn bounded_sum_identity_with_infinite_cap() {
        let a = BoundedSumF64::new(0.5, 1.0);
        // Identity has ∞ cap; combining should preserve the finite cap from `a`.
        let combined = a.combine(BoundedSumF64::identity());
        assert_eq!(combined.value, 0.5);
        assert_eq!(combined.cap, 1.0);
    }

    #[test]
    fn prob_product_satisfies_monoid_laws_in_unit_interval() {
        let a = ProbProductMonoid(0.5);
        let b = ProbProductMonoid(0.7);
        let c = ProbProductMonoid(0.3);
        assert_associative(a, b, c);
        assert_identity(a);
    }

    #[test]
    fn weighted_observation_pure_and_combine() {
        let a = WeightedObservation::<i32, SumF64>::pure(10, SumF64(1.0));
        let b = WeightedObservation::pure(20, SumF64(2.0));
        let merged = a.combine_with(b, |x, y| x + y);
        assert_eq!(merged.observation, 30);
        assert_eq!(merged.weight, SumF64(3.0));
    }

    #[test]
    fn weighted_observation_map() {
        let a = WeightedObservation::<i32, MaxF64>::pure(7, MaxF64(0.4));
        let b = a.map_observation(|x| x.to_string());
        assert_eq!(b.observation, "7");
        assert_eq!(b.weight, MaxF64(0.4));
    }
}
