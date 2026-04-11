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
use crate::property::Properties;
use crate::relationship::{Endpoints, Relationship, RelationshipId, RelationshipSlotDef};
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

    /// Look up a relationship by id.
    ///
    /// Meta-loci and event-loci use this to read the current state of
    /// specific relationships they are monitoring — e.g. after receiving
    /// a subscription notification in their inbox. Returns `None` if the
    /// relationship does not exist or has been pruned.
    fn relationship(&self, _id: RelationshipId) -> Option<&Relationship> {
        None
    }

    /// IDs of all loci directly connected to `locus` (one hop away).
    ///
    /// One entry per relationship — if two relationships of different kinds
    /// connect to the same neighbor, that neighbor appears twice.
    fn neighbor_ids(&self, locus: LocusId) -> Vec<LocusId> {
        self.relationships_for(locus)
            .map(|r| r.endpoints.other_than(locus))
            .collect()
    }

    /// IDs of all loci connected to `locus` via a relationship of `kind`.
    ///
    /// Like `neighbor_ids` but restricts to a single influence kind.
    fn neighbor_ids_of_kind(&self, locus: LocusId, kind: RelationshipKindId) -> Vec<LocusId> {
        self.relationships_for(locus)
            .filter(|r| r.kind == kind)
            .map(|r| r.endpoints.other_than(locus))
            .collect()
    }

    /// Find a relationship between `a` and `b` of a specific kind.
    ///
    /// When a locus participates in multiple relationship kinds with the
    /// same neighbor (e.g. "trust" and "conflict"), this lets programs
    /// select the specific edge they care about without iterating all
    /// neighbors. Returns the first match found (there should normally
    /// be at most one per kind pair).
    fn relationship_between_kind(
        &self,
        a: LocusId,
        b: LocusId,
        kind: RelationshipKindId,
    ) -> Option<&Relationship> {
        self.relationships_for(a)
            .find(|r| r.kind == kind && r.endpoints.involves(b))
    }

    /// Iterate all relationships that include `locus` as an endpoint,
    /// filtered to a specific kind. Useful for programs that react only
    /// to one type of relationship.
    fn relationships_for_kind<'a>(
        &'a self,
        locus: LocusId,
        kind: RelationshipKindId,
    ) -> Box<dyn Iterator<Item = &'a Relationship> + 'a> {
        Box::new(self.relationships_for(locus).filter(move |r| r.kind == kind))
    }

    /// Return the slot definitions for relationships of the given kind.
    ///
    /// Programs use this together with `relationship_slot` to read extra
    /// slots by name without hard-coding slot indices. The default
    /// implementation returns an empty slice; `BatchContext` overrides
    /// this by threading the kind registry's slot definitions.
    fn extra_slots_for_kind(&self, _kind: RelationshipKindId) -> &[RelationshipSlotDef] {
        &[]
    }

    /// Read a named extra slot from a specific relationship.
    ///
    /// Looks up the slot index by name via `extra_slots_for_kind`, then
    /// reads that slot from the relationship's `StateVector`. Returns
    /// `None` if the relationship doesn't exist, the slot name is
    /// unknown, or the index is out of bounds.
    ///
    /// ```rust,ignore
    /// let hostility = ctx.relationship_slot(ab_rel_id, CONFLICT_KIND, "hostility");
    /// ```
    fn relationship_slot(
        &self,
        rel_id: RelationshipId,
        kind: RelationshipKindId,
        name: &str,
    ) -> Option<f32> {
        let rel = self.relationship(rel_id)?;
        let slot_defs = self.extra_slots_for_kind(kind);
        let slot_idx = slot_defs.iter().position(|s| s.name == name)?;
        // Extra slots begin at index 2 (after built-in activity + weight).
        rel.state.as_slice().get(2 + slot_idx).copied()
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
    /// Wall-clock timestamp to attach to the committed `Change`, in
    /// milliseconds since the Unix epoch. When `None` the engine leaves
    /// `Change::wall_time` as `None` too.
    pub wall_time: Option<u64>,
    /// Arbitrary domain properties to propagate into `Change::metadata`.
    /// Useful for attaching provenance (source, confidence, event ID)
    /// that doesn't fit in the numeric state.
    pub metadata: Option<Properties>,
}

impl ProposedChange {
    pub fn new(subject: ChangeSubject, kind: InfluenceKindId, after: StateVector) -> Self {
        Self {
            subject,
            kind,
            after,
            extra_predecessors: Vec::new(),
            wall_time: None,
            metadata: None,
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

    /// Shorthand for a single-slot stimulus. The common case — one
    /// activation value on a single-slot locus. Equivalent to calling
    /// `stimulus(locus, kind, &[value])`.
    ///
    /// ```rust,ignore
    /// ProposedChange::activation(locus_id, FIRE_KIND, 0.8)
    /// ```
    pub fn activation(locus: LocusId, kind: InfluenceKindId, value: f32) -> Self {
        Self::new(
            ChangeSubject::Locus(locus),
            kind,
            StateVector::from_slice(&[value]),
        )
    }

    pub fn with_extra_predecessors(mut self, preds: Vec<ChangeId>) -> Self {
        self.extra_predecessors = preds;
        self
    }

    /// Attach a wall-clock timestamp (ms since Unix epoch) to this change.
    pub fn with_wall_time(mut self, ms: u64) -> Self {
        self.wall_time = Some(ms);
        self
    }

    /// Attach arbitrary domain metadata to this change.
    pub fn with_metadata(mut self, props: Properties) -> Self {
        self.metadata = Some(props);
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

    /// Subscribe `subscriber` to state changes of `rel_id`.
    ///
    /// After registration, whenever `rel_id`'s state is updated via a
    /// `ChangeSubject::Relationship` change, the committed `Change` is
    /// delivered into `subscriber`'s inbox in the **same batch** — allowing
    /// meta-loci and event-loci to react to relationship dynamics without
    /// a second program dispatch path.
    ///
    /// Subscription is idempotent: subscribing twice is equivalent to once.
    SubscribeToRelationship {
        subscriber: LocusId,
        rel_id: RelationshipId,
    },

    /// Cancel a previously registered subscription. Idempotent.
    UnsubscribeFromRelationship {
        subscriber: LocusId,
        rel_id: RelationshipId,
    },
}

impl StructuralProposal {
    /// Create a directed relationship `from → to` of `kind`.
    ///
    /// Equivalent to `StructuralProposal::CreateRelationship { endpoints: Endpoints::directed(from, to), kind }`.
    /// Idempotent — if the relationship already exists, it receives an activity touch instead.
    pub fn create_directed(from: LocusId, to: LocusId, kind: RelationshipKindId) -> Self {
        StructuralProposal::CreateRelationship {
            endpoints: Endpoints::directed(from, to),
            kind,
        }
    }

    /// Create a symmetric (undirected) relationship between `a` and `b` of `kind`.
    pub fn create_symmetric(a: LocusId, b: LocusId, kind: RelationshipKindId) -> Self {
        StructuralProposal::CreateRelationship {
            endpoints: Endpoints::symmetric(a, b),
            kind,
        }
    }

    /// Delete a relationship by id.
    pub fn delete(rel_id: RelationshipId) -> Self {
        StructuralProposal::DeleteRelationship { rel_id }
    }

    /// Subscribe `subscriber` to state changes of `rel_id`.
    pub fn subscribe(subscriber: LocusId, rel_id: RelationshipId) -> Self {
        StructuralProposal::SubscribeToRelationship { subscriber, rel_id }
    }

    /// Cancel a subscription from `subscriber` to `rel_id`.
    pub fn unsubscribe(subscriber: LocusId, rel_id: RelationshipId) -> Self {
        StructuralProposal::UnsubscribeFromRelationship { subscriber, rel_id }
    }
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

    /// Declare which relationships this locus should subscribe to from the
    /// very first batch, before any changes have been committed.
    ///
    /// Called once by `Engine::bootstrap_subscriptions` during world setup.
    /// This solves the chicken-and-egg problem: `structural_proposals` only
    /// runs when the locus receives an incoming change, but subscriptions
    /// must be in place *before* the first relevant relationship change
    /// fires so the notification is not missed.
    ///
    /// Programs that monitor specific, pre-existing relationships (e.g. an
    /// analyst locus watching a relationship created at world-construction
    /// time) should declare them here. Programs that subscribe dynamically
    /// (e.g. `EventLocusProgram` subscribing after activation) continue to
    /// use `StructuralProposal::SubscribeToRelationship` in
    /// `structural_proposals`.
    ///
    /// The default implementation returns an empty vec — most programs do
    /// not need to override this.
    fn initial_subscriptions(&self, _locus: &Locus) -> Vec<RelationshipId> {
        Vec::new()
    }
}
