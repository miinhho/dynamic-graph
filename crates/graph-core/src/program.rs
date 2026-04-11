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
    ///
    /// The return type is `Box<dyn Iterator>` because this is a `dyn`-safe
    /// trait. The boxing is a one-time allocation per call; iteration itself
    /// is zero-copy. If you need a `Vec`, collect the iterator: the cost of
    /// the collect dominates the box. For tight hot-path code, prefer
    /// keeping the iterator lazy.
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

    /// Iterate **all** relationships connecting `a` and `b`, across all
    /// kinds and directions. Complements `relationship_between` (which
    /// returns only the first match) for topologies where two loci can be
    /// connected by multiple relationship kinds simultaneously (e.g. both
    /// "trust" and "conflict" edges between the same pair of loci).
    fn relationships_between<'a>(
        &'a self,
        a: LocusId,
        b: LocusId,
    ) -> Box<dyn Iterator<Item = &'a Relationship> + 'a> {
        Box::new(self.relationships_for(a).filter(move |r| r.endpoints.involves(b)))
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
    ///
    /// **Default implementation returns `BatchId(0)` as a sentinel** — it
    /// does NOT reflect the real batch counter. The concrete `BatchContext`
    /// in graph-world overrides this with the actual current batch. If you
    /// are implementing a custom `LocusContext` outside the engine, you
    /// must override this method; otherwise temporal queries will silently
    /// read stale data.
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

    /// Iterate relationships that arrive **at** `locus`:
    /// - `Directed { from, to }` where `to == locus`.
    /// - `Symmetric` edges (always included on both sides — if you sum
    ///   incoming + outgoing, deduplicate symmetric edges).
    fn incoming_relationships<'a>(
        &'a self,
        locus: LocusId,
    ) -> Box<dyn Iterator<Item = &'a Relationship> + 'a> {
        Box::new(
            self.relationships_for(locus)
                .filter(move |r| match r.endpoints {
                    Endpoints::Directed { to, .. } => to == locus,
                    Endpoints::Symmetric { .. } => true,
                }),
        )
    }

    /// Iterate relationships that **originate** from `locus`:
    /// - `Directed { from, to }` where `from == locus`.
    /// - `Symmetric` edges (always included on both sides — if you sum
    ///   incoming + outgoing, deduplicate symmetric edges).
    fn outgoing_relationships<'a>(
        &'a self,
        locus: LocusId,
    ) -> Box<dyn Iterator<Item = &'a Relationship> + 'a> {
        Box::new(
            self.relationships_for(locus)
                .filter(move |r| match r.endpoints {
                    Endpoints::Directed { from, .. } => from == locus,
                    Endpoints::Symmetric { .. } => true,
                }),
        )
    }

    /// Incoming relationships filtered to a specific kind.
    ///
    /// Applies the direction and kind predicates in a single pass to avoid
    /// double dynamic dispatch overhead.
    fn incoming_relationships_of_kind<'a>(
        &'a self,
        locus: LocusId,
        kind: RelationshipKindId,
    ) -> Box<dyn Iterator<Item = &'a Relationship> + 'a> {
        Box::new(self.relationships_for(locus).filter(move |r| {
            r.kind == kind
                && match r.endpoints {
                    Endpoints::Directed { to, .. } => to == locus,
                    Endpoints::Symmetric { .. } => true,
                }
        }))
    }

    /// Outgoing relationships filtered to a specific kind.
    ///
    /// Applies the direction and kind predicates in a single pass to avoid
    /// double dynamic dispatch overhead.
    fn outgoing_relationships_of_kind<'a>(
        &'a self,
        locus: LocusId,
        kind: RelationshipKindId,
    ) -> Box<dyn Iterator<Item = &'a Relationship> + 'a> {
        Box::new(self.relationships_for(locus).filter(move |r| {
            r.kind == kind
                && match r.endpoints {
                    Endpoints::Directed { from, .. } => from == locus,
                    Endpoints::Symmetric { .. } => true,
                }
        }))
    }

    /// Return the domain-level properties for a locus, if any.
    ///
    /// Properties are set via `ingest()` or `world.properties_mut().insert()`.
    /// They hold human-readable, domain-specific data (e.g. `"name"`, `"type"`)
    /// that the engine never touches. The default returns `None`; the concrete
    /// `BatchContext` in graph-world wires this to `PropertyStore::get`.
    fn properties(&self, _id: LocusId) -> Option<&Properties> {
        None
    }

    /// Recent changes that targeted `rel_id`, newest first, limited to
    /// changes in batches no older than `since`. Programs use this for
    /// temporal reasoning on edges — e.g. detecting sudden activity spikes,
    /// measuring reliability drift, or computing rolling slot averages.
    ///
    /// Only explicit `ChangeSubject::Relationship` changes are returned;
    /// auto-emergence touches that do not emit a `Change` are not visible here.
    ///
    /// The default implementation returns an empty iterator; the concrete
    /// `BatchContext` in graph-world wires this to
    /// `ChangeLog::changes_to_relationship`.
    fn recent_changes_to_relationship<'a>(
        &'a self,
        _rel_id: RelationshipId,
        _since: BatchId,
    ) -> Box<dyn Iterator<Item = &'a Change> + 'a> {
        Box::new(std::iter::empty())
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

/// Filter `incoming` to changes of a specific influence kind.
///
/// Programs use this to scan only the signals they care about without
/// iterating the full inbox. Allocation is proportional to the match count,
/// not the full inbox size.
///
/// ```rust,ignore
/// let fires = changes_of_kind(incoming, FIRE_KIND);
/// if !fires.is_empty() {
///     // react to fire signals only
/// }
/// ```
pub fn changes_of_kind<'a>(incoming: &[&'a Change], kind: InfluenceKindId) -> Vec<&'a Change> {
    incoming.iter().copied().filter(|c| c.kind == kind).collect()
}

/// Filter `incoming` to changes whose subject is a `Relationship`.
///
/// Subscriber programs use this to separate relationship-state notifications
/// (delivered via subscription) from ordinary locus-to-locus signals in the
/// same inbox.
///
/// ```rust,ignore
/// let edge_updates = relationship_changes(incoming);
/// for c in edge_updates {
///     if let ChangeSubject::Relationship(rid) = c.subject { … }
/// }
/// ```
pub fn relationship_changes<'a>(incoming: &[&'a Change]) -> Vec<&'a Change> {
    incoming
        .iter()
        .copied()
        .filter(|c| matches!(c.subject, ChangeSubject::Relationship(_)))
        .collect()
}

/// Filter `incoming` to changes whose subject is a `Locus`.
///
/// Use alongside `relationship_changes` when a program handles both locus
/// signals and relationship-subscription notifications.
pub fn locus_changes<'a>(incoming: &[&'a Change]) -> Vec<&'a Change> {
    incoming
        .iter()
        .copied()
        .filter(|c| matches!(c.subject, ChangeSubject::Locus(_)))
        .collect()
}

/// Filter `incoming` to relationship-subject changes of a specific influence kind.
///
/// Combines `relationship_changes` + `changes_of_kind` in a single pass,
/// avoiding the double-allocation of calling them in sequence. The common
/// pattern in subscriber programs is to react to a specific kind of
/// relationship update:
///
/// ```rust,ignore
/// // Before: two allocations
/// changes_of_kind(&relationship_changes(incoming), SUPPLY_KIND)
///
/// // After: one allocation
/// relationship_changes_of_kind(incoming, SUPPLY_KIND)
/// ```
pub fn relationship_changes_of_kind<'a>(
    incoming: &[&'a Change],
    kind: InfluenceKindId,
) -> Vec<&'a Change> {
    incoming
        .iter()
        .copied()
        .filter(|c| c.kind == kind && matches!(c.subject, ChangeSubject::Relationship(_)))
        .collect()
}

/// A change a locus program wants to make. The engine assigns the final
/// `ChangeId`, `BatchId`, and `before` snapshot when committing.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposedChange {
    pub subject: ChangeSubject,
    pub kind: InfluenceKindId,
    /// Desired full state after the change. Ignored when `slot_patches`
    /// is `Some` — use `StateVector::empty()` as a placeholder in that case.
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
    /// Domain-level property updates to apply to the subject locus when
    /// this change is committed. Keys in the patch are merged into the
    /// locus's `PropertyStore` entry (existing keys are overwritten).
    ///
    /// Use this to update human-readable metadata (e.g. `"label"`, `"score"`)
    /// atomically with a state change, without a separate `properties_mut` call.
    /// Has no effect on `ChangeSubject::Relationship` changes.
    pub property_patch: Option<Properties>,
    /// **Relationship-only** additive slot updates. When `Some`, the engine
    /// applies each `(slot_index, delta)` to the **current live state** of
    /// the relationship at commit time, rather than replacing the full vector.
    ///
    /// This avoids the Hebbian/program overwrite conflict: a program can
    /// increment `reliability` (slot 2) without reading or touching the
    /// weight slot (slot 1) that Hebbian just updated at end-of-batch.
    ///
    /// Ordering: patches are applied in slice order; duplicate indices are
    /// summed. Ignored for `ChangeSubject::Locus` subjects.
    ///
    /// Use `ProposedChange::relationship_patch` to construct.
    pub slot_patches: Option<Vec<(usize, f32)>>,
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
            property_patch: None,
            slot_patches: None,
        }
    }

    /// Propose additive slot deltas for a relationship, without replacing its
    /// full state vector.
    ///
    /// Each `(slot_index, delta)` in `patches` is **added** to the relationship's
    /// current slot value at commit time. Slots not mentioned in `patches` — including
    /// the Hebbian weight (slot 1) — are preserved exactly as-is.
    ///
    /// This is the recommended way to update domain-specific extra slots (e.g.
    /// `"reliability"`) on relationships when Hebbian plasticity is also active,
    /// since a full-state replacement via `ProposedChange::new` would overwrite
    /// Hebbian's weight updates.
    ///
    /// ```rust,ignore
    /// // Increment reliability slot by 0.1 without touching activity or weight.
    /// proposals.push(ProposedChange::relationship_patch(
    ///     rel.id, SUPPLY_KIND, &[(RELIABILITY_SLOT, 0.1)],
    /// ));
    /// ```
    pub fn relationship_patch(
        rel_id: RelationshipId,
        kind: InfluenceKindId,
        patches: &[(usize, f32)],
    ) -> Self {
        Self {
            subject: ChangeSubject::Relationship(rel_id),
            kind,
            after: StateVector::empty(), // resolved at commit time from patches
            extra_predecessors: Vec::new(),
            wall_time: None,
            metadata: None,
            property_patch: None,
            slot_patches: Some(patches.to_vec()),
        }
    }

    /// Apply an additive delta to a single slot of a relationship.
    ///
    /// Convenience wrapper for the single-slot case of `relationship_patch`.
    /// Use `relationship_patch` when updating multiple slots in one proposal.
    ///
    /// ```rust,ignore
    /// // Increment reliability slot by 0.1.
    /// proposals.push(ProposedChange::relationship_slot_patch(
    ///     rel.id, SUPPLY_KIND, RELIABILITY_SLOT, 0.1,
    /// ));
    /// ```
    pub fn relationship_slot_patch(
        rel_id: RelationshipId,
        kind: InfluenceKindId,
        slot_idx: usize,
        delta: f32,
    ) -> Self {
        Self::relationship_patch(rel_id, kind, &[(slot_idx, delta)])
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

    /// Patch the subject locus's domain properties when this change is committed.
    ///
    /// The given `props` are merged into the locus's `PropertyStore` entry —
    /// existing keys are overwritten, unmentioned keys are preserved.
    /// Has no effect on `ChangeSubject::Relationship` changes.
    pub fn with_property_patch(mut self, props: Properties) -> Self {
        self.property_patch = Some(props);
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
    ///
    /// **Initial state priority** (highest wins):
    /// 1. `initial_state: Some(v)` — the entire `StateVector` is used as-is.
    ///    Must have at least 2 slots (activity + weight); extra slots must match
    ///    the kind's configured slot count or behaviour is unspecified.
    /// 2. `initial_activity: Some(a)` — kind default state with slot 0 overridden.
    /// 3. Neither set — kind's configured default state is used (typically
    ///    `[1.0, 0.0, …]`).
    ///
    /// Both fields are ignored when the relationship already exists (touch
    /// semantics: activity bumped by 1.0, no other slots changed).
    CreateRelationship {
        endpoints: Endpoints,
        kind: RelationshipKindId,
        /// Override the initial activity (slot 0) only. Ignored when
        /// `initial_state` is `Some`. `None` uses the kind's configured default.
        initial_activity: Option<f32>,
        /// Override the **entire** initial state vector for the new relationship.
        /// Takes precedence over `initial_activity` when `Some`.
        /// Has no effect if the relationship already exists.
        initial_state: Option<StateVector>,
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

    /// Remove a locus and all its associated data from the world.
    ///
    /// Applied at end-of-batch. The engine removes all relationships
    /// touching `locus_id` first (cleaning up their subscriptions), then
    /// removes the locus's subscriptions, domain properties, name-index
    /// entry, and finally the locus itself.
    ///
    /// Changes that previously targeted this locus remain in the log —
    /// the `ChangeSubject` becomes a dangling reference after deletion,
    /// but the causal history is intact. Any pending changes targeting
    /// this locus in the same or subsequent batches are silently dropped
    /// by the engine's non-existent-locus guard.
    DeleteLocus { locus_id: LocusId },
}

impl StructuralProposal {
    /// Create a directed relationship `from → to` of `kind`.
    ///
    /// Idempotent — if the relationship already exists, it receives an activity touch instead.
    pub fn create_directed(from: LocusId, to: LocusId, kind: RelationshipKindId) -> Self {
        StructuralProposal::CreateRelationship {
            endpoints: Endpoints::directed(from, to),
            kind,
            initial_activity: None,
            initial_state: None,
        }
    }

    /// Create a symmetric (undirected) relationship between `a` and `b` of `kind`.
    pub fn create_symmetric(a: LocusId, b: LocusId, kind: RelationshipKindId) -> Self {
        StructuralProposal::CreateRelationship {
            endpoints: Endpoints::symmetric(a, b),
            kind,
            initial_activity: None,
            initial_state: None,
        }
    }

    /// Override the initial activity (slot 0) for a `CreateRelationship` proposal.
    /// Ignored when `with_initial_state` is also set.
    ///
    /// Panics in debug builds if called on a non-`CreateRelationship` variant.
    ///
    /// ```rust
    /// # use graph_core::{LocusId, InfluenceKindId, StructuralProposal};
    /// let proposal = StructuralProposal::create_directed(LocusId(0), LocusId(1), InfluenceKindId(1))
    ///     .with_initial_activity(0.3);
    /// ```
    pub fn with_initial_activity(self, activity: f32) -> Self {
        match self {
            StructuralProposal::CreateRelationship { endpoints, kind, initial_state, .. } => {
                StructuralProposal::CreateRelationship {
                    endpoints,
                    kind,
                    initial_activity: Some(activity),
                    initial_state,
                }
            }
            other => {
                debug_assert!(false, "with_initial_activity called on non-CreateRelationship variant");
                other
            }
        }
    }

    /// Override the **entire** initial state vector for a `CreateRelationship` proposal.
    /// Takes precedence over `with_initial_activity` when set.
    /// Has no effect if the relationship already exists.
    ///
    /// Panics in debug builds if called on a non-`CreateRelationship` variant.
    ///
    /// ```rust
    /// # use graph_core::{LocusId, InfluenceKindId, StateVector, StructuralProposal};
    /// let proposal = StructuralProposal::create_directed(LocusId(0), LocusId(1), InfluenceKindId(1))
    ///     .with_initial_state(StateVector::from_slice(&[2.0, 0.0, 0.5]));
    /// ```
    pub fn with_initial_state(self, state: StateVector) -> Self {
        match self {
            StructuralProposal::CreateRelationship { endpoints, kind, initial_activity, .. } => {
                StructuralProposal::CreateRelationship {
                    endpoints,
                    kind,
                    initial_activity,
                    initial_state: Some(state),
                }
            }
            other => {
                debug_assert!(false, "with_initial_state called on non-CreateRelationship variant");
                other
            }
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

    /// Remove a locus and all its associated data.
    pub fn delete_locus(locus_id: LocusId) -> Self {
        StructuralProposal::DeleteLocus { locus_id }
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
