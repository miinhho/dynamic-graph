//! Predicate liftings and coinvariant classification.
//!
//! See `docs/coalgebra-advanced.md` §3 for the framing. Briefly:
//! given an `F`-coalgebra `(X, α)`, a predicate `P ⊆ X` lifts to a
//! predicate `F̄(P) ⊆ F(X)` along the functor. A **coinvariant** of
//! `α` is a predicate `P` such that `α(x) ∈ F̄(P)` whenever `x ∈ P` —
//! i.e. one step of dynamics never leaves `P`.
//!
//! In categorical terms (Pattinson, *Coalgebraic modal logic*, TCS
//! 2003; Hermida-Jacobs, *Structural induction and coinduction in a
//! fibrational setting*, IC 1998) the coinvariant condition is the
//! coalgebraic dual of "P is closed under constructors". For our
//! Mealy-style locus coalgebra it answers: *"if I assert P at the
//! start of a batch, is P automatically true at the end of the
//! batch?"* If yes, `P` is a one-step coinvariant and you don't have
//! to recheck it after every program dispatch — you only check at
//! crate / batch boundaries.
//!
//! ## Why this matters here
//!
//! The project's CLAUDE.md "Design invariants" section currently lists
//! six rules in prose. They are not all the same kind of object:
//!
//! - **One-step coinvariants** survive any single batch transition.
//!   `ChangeId density` and `Predecessor auto-derivation` are this
//!   kind: they are preserved by `commit_batch`'s contract, so any
//!   step of `Engine::tick` preserves them.
//!
//! - **Trace invariants** must be re-established at every operation.
//!   `ChangeLog append-only` is this kind: nothing in the
//!   substrate is allowed to ever delete a change, so every code path
//!   has the obligation.
//!
//! - **Boundary invariants** apply only at API edges.
//!   `Schema versioning` is this kind: the version check happens at
//!   `Storage::open`, never inside the engine loop.
//!
//! Classifying each rule via `InvariantKind` makes the burden
//! explicit: a one-step coinvariant only requires *local* preservation
//! (cheap to verify per-PR), while a trace invariant requires a
//! *global* audit (expensive). Misclassifying a trace invariant as a
//! one-step coinvariant is a common source of subtle bugs.
//!
//! ## What this module ships
//!
//! - The [`Coinvariant`] trait — a property carrier with a stability-
//!   under-step check.
//! - [`InvariantKind`] — the three-way classification.
//! - A small library of coinvariant predicates that map to entries of
//!   the existing CLAUDE.md list, so violations can be detected
//!   programmatically (debug builds) instead of only by code review.
//!
//! ## What this module does NOT ship
//!
//! - Full first-order predicate logic — we don't need it.
//! - Modal-logic formula evaluation — covered as deferred in
//!   `docs/coalgebra-advanced.md`.
//! - Proof assistant integration — out of scope.

use crate::change::Change;
use crate::ids::ChangeId;

/// The three classes of invariants found in the codebase. See module
/// doc comment for the rationale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvariantKind {
    /// Holds across any single F-step. The locus coalgebra preserves
    /// it automatically; new code inside the engine loop only needs
    /// to verify *local* preservation.
    OneStep,
    /// Required of every operation in the substrate, not just batch
    /// transitions. Adding new mutations requires explicit verification.
    Trace,
    /// Checked only at API boundaries (open, save, load, migrate).
    Boundary,
}

impl InvariantKind {
    /// Short prose label suitable for diagnostic output.
    pub fn label(self) -> &'static str {
        match self {
            Self::OneStep => "one-step coinvariant",
            Self::Trace => "trace invariant",
            Self::Boundary => "boundary invariant",
        }
    }
}

/// A property of the substrate that admits a check on a *witness*.
///
/// The witness is a snapshot of whatever data the property concerns
/// (a single `Change`, a slice of `ChangeId`s, …). Implementations are
/// free to choose any witness shape; the trait is parameterized by it.
///
/// Returning `Ok(())` from `check` certifies the property holds on
/// the witness. Returning `Err(reason)` documents the violation in
/// human-readable form.
pub trait Coinvariant {
    /// What the predicate is checked against. Often a borrowed
    /// reference to a fragment of world state.
    type Witness<'a>;

    /// Short stable name used in diagnostics (`"changeid_density"`,
    /// `"changelog_append_only"`, …). Stable across runs.
    fn name(&self) -> &'static str;

    /// Classification.
    fn kind(&self) -> InvariantKind;

    /// Verify the predicate on `witness`. `Ok(())` = holds; `Err(msg)`
    /// = violation with a description for the programmer.
    fn check(&self, witness: Self::Witness<'_>) -> Result<(), String>;
}

// ── Concrete coinvariants over the change log ────────────────────────────

/// **Coinvariant CD-1 (ChangeId density):** the slice of `Change`s
/// passed in must have IDs forming a dense monotone sequence —
/// `ids[i] = first_id + i`. This is required by `ChangeLog::get`'s
/// `O(1)` lookup invariant in `docs/redesign.md` §8 (O6) and by the
/// `trim_before_batch` offset arithmetic.
///
/// Classification: **OneStep**. Every place that mints a `ChangeId`
/// is local; preservation is local.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChangeIdDensity;

impl Coinvariant for ChangeIdDensity {
    type Witness<'a> = &'a [Change];
    fn name(&self) -> &'static str {
        "changeid_density"
    }
    fn kind(&self) -> InvariantKind {
        InvariantKind::OneStep
    }
    fn check(&self, witness: Self::Witness<'_>) -> Result<(), String> {
        if witness.is_empty() {
            return Ok(());
        }
        let first = witness[0].id.0;
        for (offset, change) in witness.iter().enumerate() {
            let expected = first + offset as u64;
            if change.id.0 != expected {
                return Err(format!(
                    "ChangeId density violated at offset {offset}: expected {expected}, found {}",
                    change.id.0
                ));
            }
        }
        Ok(())
    }
}

/// **Coinvariant PA-1 (Predecessors are antecedent):** every
/// predecessor `ChangeId` of a `Change` `c` must be strictly less
/// than `c.id`. The change DAG is built atop a monotone `ChangeId`
/// sequence; cycles or forward references are not allowed.
///
/// Classification: **OneStep**. Per-batch local check.
#[derive(Debug, Clone, Copy, Default)]
pub struct PredecessorsAreAntecedent;

impl Coinvariant for PredecessorsAreAntecedent {
    type Witness<'a> = &'a Change;
    fn name(&self) -> &'static str {
        "predecessors_are_antecedent"
    }
    fn kind(&self) -> InvariantKind {
        InvariantKind::OneStep
    }
    fn check(&self, witness: Self::Witness<'_>) -> Result<(), String> {
        for pred in &witness.predecessors {
            if pred.0 >= witness.id.0 {
                return Err(format!(
                    "Change {} has predecessor {} which is not strictly antecedent",
                    witness.id.0, pred.0
                ));
            }
        }
        Ok(())
    }
}

/// **Coinvariant LA-1 (ChangeLog append-only):** the sequence of
/// `ChangeId`s observed across two snapshots — earlier (`before`) and
/// later (`after`) — must satisfy: every id in `before` is also in
/// `after`. No deletion / re-mint allowed.
///
/// Classification: **Trace**. Every mutation in the substrate has the
/// obligation to preserve the log; not just batch transitions.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChangeLogAppendOnly;

impl Coinvariant for ChangeLogAppendOnly {
    type Witness<'a> = (&'a [ChangeId], &'a [ChangeId]);
    fn name(&self) -> &'static str {
        "changelog_append_only"
    }
    fn kind(&self) -> InvariantKind {
        InvariantKind::Trace
    }
    fn check(&self, witness: Self::Witness<'_>) -> Result<(), String> {
        let (before, after) = witness;
        // O(n+m) check assuming both sorted. Caller guarantees the
        // change log produces ids in monotone order, so this is fine.
        let mut after_iter = after.iter().peekable();
        for id in before {
            loop {
                match after_iter.peek() {
                    None => {
                        return Err(format!("ChangeLog dropped id {} (no more later ids)", id.0));
                    }
                    Some(next) if next.0 < id.0 => {
                        after_iter.next();
                    }
                    Some(next) if next.0 == id.0 => {
                        break;
                    }
                    Some(next) => {
                        return Err(format!(
                            "ChangeLog dropped id {} (next surviving id is {})",
                            id.0, next.0
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// **Coinvariant SV-1 (Schema versioning):** the on-disk `META_SCHEMA_VERSION`
/// must equal the runtime's `EXPECTED_SCHEMA_VERSION`, or callers must
/// run `Storage::open_and_migrate` before `open`.
///
/// Classification: **Boundary**. Checked at storage open; never
/// inside the engine loop. Implementation lives in graph-storage; the
/// stub here documents the classification so future invariant audits
/// know where to look.
#[derive(Debug, Clone, Copy, Default)]
pub struct SchemaVersionMatches;

impl Coinvariant for SchemaVersionMatches {
    type Witness<'a> = (u32, u32); // (on_disk, expected)
    fn name(&self) -> &'static str {
        "schema_version_matches"
    }
    fn kind(&self) -> InvariantKind {
        InvariantKind::Boundary
    }
    fn check(&self, witness: Self::Witness<'_>) -> Result<(), String> {
        let (on_disk, expected) = witness;
        if on_disk != expected {
            Err(format!(
                "Schema version mismatch: on-disk {on_disk}, expected {expected}. \
                 Use Storage::open_and_migrate."
            ))
        } else {
            Ok(())
        }
    }
}

/// Helper for collecting the kinds of every coinvariant in a list —
/// useful for a CI audit that prints "we have N OneStep invariants,
/// M Trace, K Boundary" so reviewers can sanity-check coverage.
pub fn classification_summary(kinds: &[InvariantKind]) -> (usize, usize, usize) {
    let mut one = 0;
    let mut trace = 0;
    let mut boundary = 0;
    for k in kinds {
        match k {
            InvariantKind::OneStep => one += 1,
            InvariantKind::Trace => trace += 1,
            InvariantKind::Boundary => boundary += 1,
        }
    }
    (one, trace, boundary)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::ChangeSubject;
    use crate::ids::{BatchId, InfluenceKindId, LocusId};
    use crate::state::StateVector;

    fn ch(id: u64, preds: &[u64]) -> Change {
        Change {
            id: ChangeId(id),
            subject: ChangeSubject::Locus(LocusId(0)),
            kind: InfluenceKindId(1),
            predecessors: preds.iter().map(|p| ChangeId(*p)).collect(),
            before: StateVector::empty(),
            after: StateVector::empty(),
            batch: BatchId(0),
            wall_time: None,
            metadata: None,
        }
    }

    #[test]
    fn changeid_density_passes_on_dense_slice() {
        let s = [ch(0, &[]), ch(1, &[0]), ch(2, &[1])];
        assert!(ChangeIdDensity.check(&s).is_ok());
    }

    #[test]
    fn changeid_density_detects_gap() {
        let s = [ch(0, &[]), ch(2, &[0])]; // gap: missing id 1
        let err = ChangeIdDensity.check(&s).unwrap_err();
        assert!(err.contains("expected 1"), "got: {err}");
    }

    #[test]
    fn predecessors_must_be_antecedent_passes() {
        assert!(PredecessorsAreAntecedent.check(&ch(5, &[1, 2, 3])).is_ok());
    }

    #[test]
    fn predecessors_must_be_antecedent_detects_forward_ref() {
        let bad = ch(2, &[5]); // 5 > 2
        assert!(PredecessorsAreAntecedent.check(&bad).is_err());
    }

    #[test]
    fn predecessors_must_be_antecedent_detects_self_ref() {
        let bad = ch(2, &[2]);
        assert!(PredecessorsAreAntecedent.check(&bad).is_err());
    }

    #[test]
    fn changelog_append_only_passes_on_extension() {
        let before = [ChangeId(0), ChangeId(1)];
        let after = [ChangeId(0), ChangeId(1), ChangeId(2)];
        assert!(ChangeLogAppendOnly.check((&before, &after)).is_ok());
    }

    #[test]
    fn changelog_append_only_detects_deletion() {
        let before = [ChangeId(0), ChangeId(1), ChangeId(2)];
        let after = [ChangeId(0), ChangeId(2)]; // 1 was deleted
        let err = ChangeLogAppendOnly.check((&before, &after)).unwrap_err();
        assert!(err.contains("dropped id 1"), "got: {err}");
    }

    #[test]
    fn schema_version_matches_passes_on_equal() {
        assert!(SchemaVersionMatches.check((2, 2)).is_ok());
    }

    #[test]
    fn schema_version_matches_fails_on_drift() {
        let err = SchemaVersionMatches.check((1, 2)).unwrap_err();
        assert!(err.contains("on-disk 1"));
        assert!(err.contains("open_and_migrate"));
    }

    #[test]
    fn classification_summary_counts_correctly() {
        let kinds = [
            InvariantKind::OneStep,
            InvariantKind::OneStep,
            InvariantKind::Trace,
            InvariantKind::Boundary,
        ];
        assert_eq!(classification_summary(&kinds), (2, 1, 1));
    }

    #[test]
    fn invariants_have_stable_names() {
        // Names are part of the diagnostic surface; pinning them in a
        // test catches accidental renames in PR review.
        assert_eq!(ChangeIdDensity.name(), "changeid_density");
        assert_eq!(
            PredecessorsAreAntecedent.name(),
            "predecessors_are_antecedent"
        );
        assert_eq!(ChangeLogAppendOnly.name(), "changelog_append_only");
        assert_eq!(SchemaVersionMatches.name(), "schema_version_matches");
    }
}
