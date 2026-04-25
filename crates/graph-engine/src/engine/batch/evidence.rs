//! Phase 0 of the trigger-axis roadmap: name the emergence atom.
//!
//! `EmergenceEvidence` is the *raw observation* that suggests a relationship
//! might exist between two loci. The batch pipeline now has three explicit
//! seams instead of a single fused path:
//!
//! ```text
//!   detection      → Vec<EmergenceEvidence>          (this module)
//!   interpretation → Vec<CrossLocusPred>             (compute.rs)
//!   application    → store mutation                  (emergence_apply.rs)
//! ```
//!
//! The detection step records *what was observed*; the interpretation step
//! decides whether it implies an Update or Create on the relationship store
//! and computes the per-evidence schema/decay arithmetic. Splitting these
//! steps gives later phases a clean place to add new evidence variants
//! (joint flow, reverse inference, feedback) without rewriting the apply
//! path.
//!
//! Today there is exactly one variant — `CrossLocusFlow` — and its semantics
//! are bit-equivalent to the pre-Phase-0 `cross_locus_predecessors` output.

use graph_core::{BatchId, ChangeId, LocusId, LocusKindId};

/// One unit of detected evidence that a relationship between two loci
/// may exist or be reinforced. Produced during the parallel COMPUTE
/// phase and consumed by the interpreter in the same phase.
///
/// **Contract**: an evidence value is a *pure observation* — it carries
/// no decision about whether the relationship store should be mutated,
/// no schema-violation flag, and no resolution. Those are derived from
/// the evidence by `interpret_evidence`.
pub(crate) enum EmergenceEvidence {
    /// The canonical "A influenced B" observation: a change at `to_locus`
    /// (the receiver, recorded by the caller) lists a change at
    /// `from_locus` as a causal predecessor.
    ///
    /// `pre_signal` is preserved with sign — the abs() reduction that the
    /// activity-accumulation arithmetic requires happens during
    /// interpretation, not detection. Phase 1 of the roadmap reconsiders
    /// where signed semantics should be preserved (activity slot itself,
    /// or boundary-only); until then the abs is centralised through
    /// `activity_contribution_magnitude` so there is a single change site.
    CrossLocusFlow {
        from_locus: LocusId,
        from_kind: LocusKindId,
        pre_signal: f32,
        pred_batch: BatchId,
        pred_change_id: ChangeId,
    },
}

/// Compute the signed contribution that one evidence adds to a
/// relationship's activity slot.
///
/// **Phase 1 (2026-04-25)**: returns `activity_contribution × pre_signal`
/// without taking the absolute value. Inhibitory predecessors
/// (`pre_signal < 0`) now subtract from activity, and the label-propagation
/// step in `emergence/default/community.rs` interprets the resulting
/// negative weight as **repulsion** between the endpoints. This unlocks
/// the inhibitory-edge support that the propagation algorithm was already
/// written to handle (see comment in `default.rs:518-525`) but had never
/// observed because the Phase 0 abs reduction kept activity non-negative.
///
/// Consumer sites that compare activity against a "still alive" threshold
/// (boundary signal, decay floor, perspective filter, cold-relationship
/// demotion, active-query filter) take `.abs()` at the call site instead,
/// so existing single-sign benchmarks remain bit-equivalent.
///
/// Only the three activity-write sites route through this helper. The
/// Hebbian rule needs the signed `pre_signal` directly and bypasses it.
#[inline]
pub(crate) fn signed_activity_contribution(
    activity_contribution: f32,
    pre_signal: f32,
) -> f32 {
    activity_contribution * pre_signal
}
