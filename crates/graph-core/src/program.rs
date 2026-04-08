//! Locus program trait and the proposed-change shape it returns.
//!
//! A `LocusProgram` is the user's hook into the substrate. The engine
//! calls it whenever incoming changes touch a locus, and the program
//! decides which new changes (if any) to propose in response. Programs
//! are *pure functions* — same inputs always produce the same outputs —
//! to keep replay deterministic.
//!
//! `ProposedChange` is what a program returns. It is intentionally
//! "pre-commit": it has no `ChangeId`, no `BatchId`, and no resolved
//! predecessor list, because those are assigned by the engine when the
//! batch is committed. Per O1 in `docs/redesign.md` §8, predecessor
//! tracking is hybrid: the engine derives them automatically from the
//! reads the program performed via `ProcessingContext`, with
//! `extra_predecessors` available as an explicit override.
//!
//! The signature is intentionally minimal in this commit. The richer
//! `ProcessingContext` (typed inbox grouped by influence kind, read
//! tracking, kind registry access) lands when the batch loop does.

use crate::change::{Change, ChangeSubject};
use crate::ids::{ChangeId, InfluenceKindId};
use crate::locus::Locus;
use crate::relationship::{Endpoints, RelationshipId, RelationshipKindId};
use crate::state::StateVector;

/// A change a locus program wants to make. The engine assigns the final
/// `ChangeId`, `BatchId`, and `before` snapshot when committing.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposedChange {
    pub subject: ChangeSubject,
    pub kind: InfluenceKindId,
    pub after: StateVector,
    /// Predecessors the program wants to declare beyond whatever the
    /// engine auto-detects from read tracking. Usually empty.
    pub extra_predecessors: Vec<ChangeId>,
}

impl ProposedChange {
    pub fn new(subject: ChangeSubject, kind: InfluenceKindId, after: StateVector) -> Self {
        Self {
            subject,
            kind,
            after,
            extra_predecessors: Vec::new(),
        }
    }

    pub fn with_extra_predecessors(mut self, preds: Vec<ChangeId>) -> Self {
        self.extra_predecessors = preds;
        self
    }
}

/// A structural change to the relationship graph proposed by a program.
///
/// Structural proposals are applied at end-of-batch, after all state
/// changes have been committed and programs have run. They take effect
/// immediately (in the same batch's cleanup), so the new or removed
/// relationship is visible to the next batch.
///
/// Unlike `ProposedChange`, structural proposals are not recorded as
/// `Change`s in the log. Causal logging of structural changes is
/// deferred to the lineage query layer.
#[derive(Debug, Clone, PartialEq)]
pub enum StructuralProposal {
    /// Create a new relationship connecting `endpoints` of `kind`.
    ///
    /// If a relationship with the same `(endpoints.key(), kind)` already
    /// exists, this is treated as an activity touch rather than a
    /// duplicate — same semantics as `auto_emerge_relationship`.
    CreateRelationship {
        endpoints: Endpoints,
        kind: RelationshipKindId,
    },
    /// Remove a relationship from the world.
    ///
    /// The relationship's history in the change log is preserved (changes
    /// that targeted it still exist); only its live presence in the
    /// relationship store is removed. After deletion the relationship id
    /// becomes dangling — do not reuse it.
    DeleteRelationship { rel_id: RelationshipId },
}

/// User-supplied behavior for a single locus kind.
///
/// One impl is registered per `LocusKindId`; every locus carrying that
/// kind shares the impl. Implementations must be `Send + Sync` so the
/// batch loop can run them in parallel across causally-independent loci.
pub trait LocusProgram: Send + Sync {
    /// Run the program for one locus. `incoming` is the slice of
    /// committed changes that fired *into* this locus during the batch
    /// being processed.
    fn process(&self, locus: &Locus, incoming: &[Change]) -> Vec<ProposedChange>;

    /// Propose structural changes to the relationship topology.
    ///
    /// Called with the same `locus` and `incoming` as `process`, at the
    /// same point in the batch loop. The default implementation returns
    /// an empty vec — programs only override this when they need topology
    /// mutation. Backwards compatible: existing programs need not change.
    fn structural_proposals(&self, _locus: &Locus, _incoming: &[Change]) -> Vec<StructuralProposal> {
        Vec::new()
    }
}
