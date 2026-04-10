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
use crate::cohere::Cohere;
use crate::entity::{Entity, EntityId};
use crate::ids::{ChangeId, InfluenceKindId, LocusId, RelationshipKindId};
use crate::locus::Locus;
use crate::relationship::{Endpoints, Relationship, RelationshipId};
use crate::state::StateVector;
use crate::BatchId;

/// Read-only snapshot of the world state made available to
/// `LocusProgram::process` and `structural_proposals`. Programs use
/// this to read neighbor locus states or relationship weights without
/// holding a mutable borrow on the world.
///
/// The context reflects the world state *at the start of the current
/// batch* (after the previous batch's changes were committed). Programs
/// cannot see the outputs of other programs running in the same batch.
pub trait LocusContext {
    /// Current state of the locus with the given `id`. Returns `None` if
    /// the locus does not exist.
    fn locus(&self, id: LocusId) -> Option<&Locus>;

    /// Iterate all relationships that include `locus` as an endpoint.
    /// Direction is preserved — use `r.endpoints` to determine which end
    /// is the source and which is the target.
    fn relationships_for<'a>(
        &'a self,
        locus: LocusId,
    ) -> Box<dyn Iterator<Item = &'a Relationship> + 'a>;

    /// Find a specific relationship between two loci, if one exists.
    /// Checks both directions and symmetric edges.
    fn relationship_between(&self, a: LocusId, b: LocusId) -> Option<&Relationship> {
        self.relationships_for(a)
            .find(|r| r.endpoints.involves(b))
    }

    /// Recent changes that targeted `locus`, newest first, limited to
    /// changes in batches no older than `since`. Programs use this for
    /// temporal reasoning — e.g. spike-timing-dependent plasticity,
    /// burst detection, or rate estimation.
    ///
    /// The default implementation returns an empty iterator; the concrete
    /// `BatchContext` in graph-world wires this to `ChangeLog::changes_to_locus`.
    fn recent_changes<'a>(
        &'a self,
        _locus: LocusId,
        _since: BatchId,
    ) -> Box<dyn Iterator<Item = &'a Change> + 'a> {
        Box::new(std::iter::empty())
    }

    /// The current batch id. Programs can use this together with
    /// `recent_changes` to compute elapsed batches since an event.
    fn current_batch(&self) -> BatchId {
        BatchId(0)
    }

    /// Find the entity that contains `locus` as a member, if any.
    /// Enables **downward causation**: programs can adjust behavior
    /// based on entity-layer context (coherence, status, lifecycle).
    ///
    /// Default returns `None`; the concrete `BatchContext` wires this
    /// to the entity store.
    fn entity_of(&self, _locus: LocusId) -> Option<&Entity> {
        None
    }

    /// Get an entity by id.
    fn entity(&self, _id: EntityId) -> Option<&Entity> {
        None
    }

    /// Get cohere clusters for a perspective. Enables programs to react
    /// to higher-layer structural information.
    fn coheres(&self, _perspective: &str) -> Option<&[Cohere]> {
        None
    }
}

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

    /// Shorthand for creating a stimulus — a change targeting a locus with
    /// a single-element state vector. This is the most common shape for
    /// external stimuli injected into `Engine::tick`.
    pub fn stimulus(locus: LocusId, kind: InfluenceKindId, values: &[f32]) -> Self {
        Self::new(
            ChangeSubject::Locus(locus),
            kind,
            StateVector::from_slice(values),
        )
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
    /// Run the program for one locus.
    ///
    /// - `locus`: the locus this program is running for.
    /// - `incoming`: committed changes that fired into this locus during
    ///   the current batch.
    /// - `ctx`: read-only view of the world state at the start of this
    ///   batch. Use this to query neighbor states and relationship weights.
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange>;

    /// Propose structural changes to the relationship topology.
    ///
    /// Called with the same `locus`, `incoming`, and `ctx` as `process`,
    /// at the same point in the batch loop. Use `ctx` to query neighbor
    /// states when deciding whether to create or remove relationships.
    /// The default implementation returns an empty vec — programs only
    /// override this when they need topology mutation.
    fn structural_proposals(&self, _locus: &Locus, _incoming: &[&Change], _ctx: &dyn LocusContext) -> Vec<StructuralProposal> {
        Vec::new()
    }
}
