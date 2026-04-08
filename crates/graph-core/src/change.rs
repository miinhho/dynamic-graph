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
//! This commit only models locus-subject changes. Relationship-subject
//! changes are added when Layer 2 lands.

use crate::ids::{BatchId, ChangeId, InfluenceKindId, LocusId};
use crate::state::StateVector;

/// What a change is *about*. Currently always a locus; the
/// `Relationship` variant lands with Layer 2.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChangeSubject {
    Locus(LocusId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub id: ChangeId,
    pub subject: ChangeSubject,
    pub kind: InfluenceKindId,
    pub predecessors: Vec<ChangeId>,
    pub before: StateVector,
    pub after: StateVector,
    pub batch: BatchId,
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
        };
        assert!(!c.is_stimulus());
    }
}
