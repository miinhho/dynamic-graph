//! Layer 2: relationship — the first emergent layer.
//!
//! A `Relationship` is *not* registered by the user. The engine
//! recognizes one whenever it observes cross-locus causal flow: a
//! change at locus B whose predecessor set contains a change at a
//! different locus A implies "A influences B" of the kind carried by
//! the new change. The first such observation creates a relationship;
//! subsequent observations update its state and lineage.
//!
//! Per O8 in `docs/redesign.md` §8, the relationship's kind is the same
//! identifier as the influence kind that created it (`RelationshipKindId
//! == InfluenceKindId`). They are the *same dimension*; refining
//! sub-kinds (e.g., "thermal radiation" vs "thermal conduction") is
//! deferred until a real use case asks for it.
//!
//! Relationships are entity-like: they have stable IDs, they evolve, and
//! they are valid `ChangeSubject`s — relationship-subject changes update
//! their state and feed higher emergent layers.

use crate::ids::{ChangeId, InfluenceKindId, LocusId, RelationshipKindId};
use crate::state::StateVector;

/// Identity of a relationship in the relationship store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RelationshipId(pub u64);

/// Which loci a relationship connects, and how.
///
/// `Directed` is the common case (A influences B). `Symmetric` exists
/// for kinds that are inherently undirected (e.g., shared resonance).
/// Both variants are binary — a relationship connects exactly two loci.
/// N-ary hyperedges are not modelled; multi-party interactions are
/// expressed as multiple pairwise relationships.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Endpoints {
    Directed { from: LocusId, to: LocusId },
    Symmetric { a: LocusId, b: LocusId },
}

impl Endpoints {
    /// Returns `true` if every endpoint in this relationship is contained
    /// in `set`. Used by `World::induced_subgraph`.
    pub fn all_endpoints_in(&self, set: &rustc_hash::FxHashSet<LocusId>) -> bool {
        match self {
            Endpoints::Directed { from, to } => set.contains(from) && set.contains(to),
            Endpoints::Symmetric { a, b } => set.contains(a) && set.contains(b),
        }
    }

    /// Returns `true` if `locus` appears in any endpoint position.
    pub fn involves(&self, locus: LocusId) -> bool {
        match self {
            Endpoints::Directed { from, to } => *from == locus || *to == locus,
            Endpoints::Symmetric { a, b } => *a == locus || *b == locus,
        }
    }

    /// The endpoint that is not `locus`, treating the relationship as
    /// undirected. For a self-loop (`from == to == locus`), returns `locus`.
    pub fn other_than(&self, locus: LocusId) -> LocusId {
        match self {
            Endpoints::Directed { from, to } => {
                if *from == locus { *to } else { *from }
            }
            Endpoints::Symmetric { a, b } => {
                if *a == locus { *b } else { *a }
            }
        }
    }

    /// Canonical lookup key — endpoints flattened into a stable shape so
    /// the relationship store can dedupe hits regardless of insertion
    /// order. For `Symmetric`, the two ids are sorted; for `Directed`,
    /// order is preserved (it carries meaning).
    pub fn key(&self) -> EndpointKey {
        match self {
            Endpoints::Directed { from, to } => EndpointKey::Directed(*from, *to),
            Endpoints::Symmetric { a, b } => {
                let (lo, hi) = if a.0 <= b.0 { (*a, *b) } else { (*b, *a) };
                EndpointKey::Symmetric(lo, hi)
            }
        }
    }
}

/// Hashable canonical form of `Endpoints`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EndpointKey {
    Directed(LocusId, LocusId),
    Symmetric(LocusId, LocusId),
}

/// Lineage metadata for a relationship — which changes brought it into
/// existence, which most recently touched it, and how active it has
/// been over the run. The framework consumes this both for the
/// relationship's own state evolution and as one of the inputs to the
/// (later) entity-emergence perspective.
///
/// Both `created_by` and `last_touched_by` are `None` for relationships
/// created via `StructuralProposal` or test helpers. `last_touched_by`
/// is filled in by the first engine change that touches the relationship
/// after its creation.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RelationshipLineage {
    /// The change that first auto-emerged this relationship; `None` if
    /// created structurally (no single originating change).
    pub created_by: Option<ChangeId>,
    /// The most recent engine change that touched this relationship.
    pub last_touched_by: Option<ChangeId>,
    pub change_count: u64,
    /// Influence kinds the engine has seen flow through this
    /// relationship. Often a single entry, but kept open in case the
    /// emergence policy chooses to collapse multiple kinds into one
    /// edge.
    pub kinds_observed: Vec<InfluenceKindId>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Relationship {
    pub id: RelationshipId,
    pub kind: RelationshipKindId,
    pub endpoints: Endpoints,
    /// Cumulative metrics for the relationship. Slot 0 is conventionally
    /// the "activity score" — incremented on each new change observed
    /// and decayed once per batch by the kind's `decay_per_batch`. The
    /// engine treats this slot specially; downstream perspectives may
    /// add their own slots.
    pub state: StateVector,
    pub lineage: RelationshipLineage,
    /// Batch number at which `state` slots were last explicitly decayed.
    /// The engine uses lazy decay: accumulated decay is applied when a
    /// relationship is touched (auto-emerge) or flushed before entity
    /// recognition. Use `decay^(current_batch - last_decayed_batch)`.
    pub last_decayed_batch: u64,
}

impl Relationship {
    /// Index of the activity slot inside `state`. Incremented on each
    /// new change observed, decayed once per batch.
    pub const ACTIVITY_SLOT: usize = 0;

    /// Index of the Hebbian weight slot inside `state`. Updated by the
    /// plasticity rule: `Δweight = η * pre_signal * post_signal`.
    /// Zero initially; grows with correlated pre/post activity.
    pub const WEIGHT_SLOT: usize = 1;

    /// Read the activity score (slot 0).
    pub fn activity(&self) -> f32 {
        self.state.as_slice().get(Self::ACTIVITY_SLOT).copied().unwrap_or(0.0)
    }

    /// Read the learned Hebbian weight (slot 1).
    pub fn weight(&self) -> f32 {
        self.state.as_slice().get(Self::WEIGHT_SLOT).copied().unwrap_or(0.0)
    }
}
