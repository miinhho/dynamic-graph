//! Layer 1: the atomic change.
//!
//! Per `docs/redesign.md` §3.2 and §2.5, a `Change` is the only
//! first-class causal event in the substrate. There is no separate
//! "cause" or "effect" type — those are roles inside the change-flow.
//!
//! A change knows:
//! - which locus (or, later, which relationship) it modifies
//! - which influence kind it carries
//! - its causal predecessors (zero for an external stimulus, one or more
//!   for an internally produced change)
//! - the before and after state of the subject
//! - the batch in which it fired
//!
//! Both locus-subject and relationship-subject changes are supported.
//! The engine's batch loop dispatches locus programs only on locus-subject
//! changes; relationship-subject changes update the relationship's state
//! and are recorded in the log but do not trigger program dispatch.

use crate::ids::{BatchId, ChangeId, InfluenceKindId, LocusId};
use crate::property::Properties;
use crate::relationship::RelationshipId;
use crate::state::StateVector;

/// What a change is *about*.
///
/// - `Locus`: modifies a locus's state and may trigger its program.
/// - `Relationship`: modifies a relationship's state directly. Does not
///   trigger program dispatch. Enables locus programs to write feedback
///   directly onto a relationship (e.g. Hebbian-style weight updates).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChangeSubject {
    Locus(LocusId),
    Relationship(RelationshipId),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Change {
    pub id: ChangeId,
    pub subject: ChangeSubject,
    pub kind: InfluenceKindId,
    pub predecessors: Vec<ChangeId>,
    pub before: StateVector,
    pub after: StateVector,
    pub batch: BatchId,
    /// Wall-clock timestamp at the time the change was committed, in
    /// milliseconds since the Unix epoch. `None` when the engine is
    /// running in a context where wall time is unavailable or not needed
    /// (e.g. deterministic replay, unit tests).
    pub wall_time: Option<u64>,
    /// Arbitrary domain properties attached to this change. Use this for
    /// provenance metadata (source system, confidence score, event ID)
    /// that does not fit in the numeric `StateVector`. `None` = no
    /// metadata (zero overhead on the hot path).
    pub metadata: Option<Properties>,
}

impl Change {
    /// Convenience constructor for a stimulus — a change with no causal
    /// predecessors. Per O9 in `docs/redesign.md` §8, "stimulus" is just
    /// a `Change` with an empty predecessor set, not a separate type.
    pub fn stimulus(
        id: ChangeId,
        subject: ChangeSubject,
        kind: InfluenceKindId,
        before: StateVector,
        after: StateVector,
        batch: BatchId,
    ) -> Self {
        Self {
            id,
            subject,
            kind,
            predecessors: Vec::new(),
            before,
            after,
            batch,
            wall_time: None,
            metadata: None,
        }
    }

    /// True iff this change has no causal predecessors. Such a change is
    /// a root in the causal DAG — i.e., a stimulus from outside the
    /// engine.
    pub fn is_stimulus(&self) -> bool {
        self.predecessors.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stimulus_has_empty_predecessors() {
        let c = Change::stimulus(
            ChangeId(1),
            ChangeSubject::Locus(LocusId(7)),
            InfluenceKindId(2),
            StateVector::zeros(2),
            StateVector::from_slice(&[0.5, 0.0]),
            BatchId(0),
        );
        assert!(c.is_stimulus());
        assert_eq!(c.predecessors.len(), 0);
    }

    #[test]
    fn internal_change_is_not_a_stimulus() {
        let c = Change {
            id: ChangeId(2),
            subject: ChangeSubject::Locus(LocusId(7)),
            kind: InfluenceKindId(2),
            predecessors: vec![ChangeId(1)],
            before: StateVector::empty(),
            after: StateVector::empty(),
            batch: BatchId(1),
            wall_time: None,
            metadata: None,
        };
        assert!(!c.is_stimulus());
    }
}
