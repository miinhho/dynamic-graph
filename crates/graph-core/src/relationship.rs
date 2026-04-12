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

use crate::ids::{BatchId, ChangeId, InfluenceKindId, LocusId, RelationshipKindId};
use crate::property::Properties;
use crate::state::StateVector;

/// The net effect when two influence kinds co-occur on the same pair of loci.
///
/// Used by `InfluenceKindRegistry::register_interaction` to declare how
/// cross-kind relationships should be interpreted at query time. The engine
/// never consults this during the batch loop — semantics are resolved
/// entirely in `graph-query`.
///
/// # Examples
///
/// ```ignore
/// // Excitatory + inhibitory cancel out → Antagonistic
/// registry.register_interaction(EXCITE, INHIBIT, InteractionEffect::Antagonistic { dampen: 0.5 });
///
/// // Two excitatory kinds reinforce → Synergistic
/// registry.register_interaction(FIRE, DOPAMINE, InteractionEffect::Synergistic { boost: 1.3 });
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum InteractionEffect {
    /// The two kinds amplify each other; multiply the combined activity by `boost`.
    /// Typical range: `boost > 1.0`. Values <= 0 are accepted but unusual.
    Synergistic { boost: f32 },
    /// The two kinds oppose each other; multiply the combined activity by `dampen`.
    /// Typical range: `0.0 < dampen < 1.0`.
    Antagonistic { dampen: f32 },
    /// No special interaction; activities are summed without adjustment.
    Neutral,
}

/// Definition of one user-defined extra slot in a relationship's `StateVector`.
///
/// By default, relationships carry exactly two built-in slots:
/// - slot 0: activity score (incremented on each touch, decayed per batch)
/// - slot 1: Hebbian weight (updated by plasticity rule)
///
/// `RelationshipSlotDef` lets users attach domain-specific metrics — e.g.
/// `hostility`, `trust`, or `supply_rate` for a real-world event model —
/// without wrapping the `Relationship` type.
///
/// Extra slots occupy indices **2, 3, 4, …** in the `StateVector`, in the
/// order they appear in `InfluenceKindConfig::extra_slots`.
#[derive(Debug, Clone)]
pub struct RelationshipSlotDef {
    /// Human-readable name for diagnostics and slot-by-name access.
    pub name: &'static str,
    /// Value assigned when the relationship is first created.
    pub default: f32,
    /// Per-batch multiplicative decay applied during lazy-decay flush.
    /// `None` = this slot is not decayed. `Some(1.0)` = explicit no-decay.
    pub decay: Option<f32>,
}

impl RelationshipSlotDef {
    pub fn new(name: &'static str, default: f32) -> Self {
        Self { name, default, decay: None }
    }

    pub fn with_decay(mut self, decay: f32) -> Self {
        self.decay = Some(decay);
        self
    }
}

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
    /// Shorthand for `Endpoints::Directed { from, to }`.
    #[inline]
    pub fn directed(from: LocusId, to: LocusId) -> Self {
        Endpoints::Directed { from, to }
    }

    /// Shorthand for `Endpoints::Symmetric { a, b }`.
    #[inline]
    pub fn symmetric(a: LocusId, b: LocusId) -> Self {
        Endpoints::Symmetric { a, b }
    }

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

    /// Returns `true` when this is a `Directed` edge (`from → to`).
    #[inline]
    pub fn is_directed(&self) -> bool {
        matches!(self, Endpoints::Directed { .. })
    }

    /// Returns `true` when this is a `Symmetric` edge (`a ↔ b`).
    #[inline]
    pub fn is_symmetric(&self) -> bool {
        matches!(self, Endpoints::Symmetric { .. })
    }

    /// For a `Directed` edge, return the source endpoint (`from`).
    /// Returns `None` for `Symmetric` edges.
    pub fn source(&self) -> Option<LocusId> {
        match self {
            Endpoints::Directed { from, .. } => Some(*from),
            Endpoints::Symmetric { .. } => None,
        }
    }

    /// For a `Directed` edge, return the target endpoint (`to`).
    /// Returns `None` for `Symmetric` edges.
    pub fn target(&self) -> Option<LocusId> {
        match self {
            Endpoints::Directed { to, .. } => Some(*to),
            Endpoints::Symmetric { .. } => None,
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
    /// Batch in which this relationship was first created.
    ///
    /// Set by the engine (auto-emerge or structural proposal) to the batch
    /// that was active at creation time. Relationships created via
    /// `World::add_relationship` use the world's current batch. This field
    /// survives change-log trimming, unlike `lineage.created_by`.
    pub created_batch: BatchId,
    /// Batch number at which `state` slots were last explicitly decayed.
    /// The engine uses lazy decay: accumulated decay is applied when a
    /// relationship is touched (auto-emerge) or flushed before entity
    /// recognition. Use `decay^(current_batch - last_decayed_batch)`.
    pub last_decayed_batch: u64,
    /// Optional domain-specific properties attached to this relationship.
    ///
    /// Analogous to `Change::metadata` — zero overhead on the common path
    /// (stored as `None`), but lets users annotate relationships with
    /// string or numeric tags without wrapping the struct.
    ///
    /// Example uses: semantic type labels (`"type" → "trust"`), confidence
    /// scores (`"confidence" → 0.9`), provenance (`"source" → "pipeline-v2"`).
    pub metadata: Option<Properties>,
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

    /// How many batches old this relationship is: `current_batch - created_batch`.
    #[inline]
    pub fn age_in_batches(&self, current_batch: BatchId) -> u64 {
        current_batch.0.saturating_sub(self.created_batch.0)
    }

    /// Combined signal strength: `activity + weight`.
    ///
    /// Useful for ranking relationships by overall "importance" — high
    /// activity alone means a recently active edge, high weight alone means
    /// a well-learned structural edge, and high strength means both.
    #[inline]
    pub fn strength(&self) -> f32 {
        self.activity() + self.weight()
    }

    /// Returns `true` if `locus` appears in any endpoint position.
    #[inline]
    pub fn involves(&self, locus: LocusId) -> bool {
        self.endpoints.involves(locus)
    }

    /// The endpoint that is not `locus`, treating the edge as undirected.
    /// For a self-loop, returns `locus`.
    #[inline]
    pub fn other_endpoint(&self, locus: LocusId) -> LocusId {
        self.endpoints.other_than(locus)
    }

    /// For a `Directed` edge, return the source endpoint (`from`).
    /// Returns `None` for `Symmetric` edges.
    pub fn from(&self) -> Option<LocusId> {
        self.endpoints.source()
    }

    /// For a `Directed` edge, return the target endpoint (`to`).
    /// Returns `None` for `Symmetric` edges.
    pub fn to(&self) -> Option<LocusId> {
        self.endpoints.target()
    }

    /// Read a string property from this relationship's metadata.
    ///
    /// Returns `None` when `metadata` is absent or the key is not a string.
    pub fn get_str_property(&self, key: &str) -> Option<&str> {
        self.metadata.as_ref()?.get_str(key)
    }

    /// Read a numeric (f64) property from this relationship's metadata.
    ///
    /// Returns `None` when `metadata` is absent or the key is not numeric.
    pub fn get_f64_property(&self, key: &str) -> Option<f64> {
        self.metadata.as_ref()?.get_f64(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directed(from: u64, to: u64) -> Endpoints {
        Endpoints::directed(LocusId(from), LocusId(to))
    }

    fn symmetric(a: u64, b: u64) -> Endpoints {
        Endpoints::symmetric(LocusId(a), LocusId(b))
    }

    fn make_rel(endpoints: Endpoints) -> Relationship {
        Relationship {
            id: RelationshipId(0),
            kind: crate::ids::InfluenceKindId(1),
            endpoints,
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: vec![],
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        }
    }

    #[test]
    fn endpoints_is_directed_and_is_symmetric() {
        assert!(directed(1, 2).is_directed());
        assert!(!directed(1, 2).is_symmetric());
        assert!(symmetric(1, 2).is_symmetric());
        assert!(!symmetric(1, 2).is_directed());
    }

    #[test]
    fn relationship_involves_delegates_to_endpoints() {
        let rel = make_rel(directed(1, 2));
        assert!(rel.involves(LocusId(1)));
        assert!(rel.involves(LocusId(2)));
        assert!(!rel.involves(LocusId(3)));
    }

    #[test]
    fn relationship_other_endpoint_directed() {
        let rel = make_rel(directed(1, 2));
        assert_eq!(rel.other_endpoint(LocusId(1)), LocusId(2));
        assert_eq!(rel.other_endpoint(LocusId(2)), LocusId(1));
    }

    #[test]
    fn relationship_from_to_directed() {
        let rel = make_rel(directed(3, 7));
        assert_eq!(rel.from(), Some(LocusId(3)));
        assert_eq!(rel.to(), Some(LocusId(7)));
    }

    #[test]
    fn relationship_from_to_symmetric_is_none() {
        let rel = make_rel(symmetric(3, 7));
        assert_eq!(rel.from(), None);
        assert_eq!(rel.to(), None);
    }
}
