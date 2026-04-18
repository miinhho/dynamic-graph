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

use smallvec::SmallVec;

use crate::ids::{BatchId, ChangeId, InfluenceKindId, LocusId, RelationshipKindId};
use crate::property::Properties;
use crate::state::StateVector;

mod endpoints;
mod lineage;

/// Per-kind observation record in a `RelationshipLineage`.
///
/// Tracks how many times a specific influence kind has flowed through the
/// relationship and which batch last produced a touch of that kind.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct KindObservation {
    pub kind: InfluenceKindId,
    /// Number of times this kind has touched the relationship.
    pub touch_count: u64,
    /// Batch in which this kind last touched the relationship.
    pub last_batch: BatchId,
}

impl KindObservation {
    /// Create a first-touch observation at the given batch.
    pub fn once(kind: InfluenceKindId, batch: BatchId) -> Self {
        Self {
            kind,
            touch_count: 1,
            last_batch: batch,
        }
    }

    /// Create a placeholder observation for synthetic relationships that have
    /// no batch context (e.g. test fixtures, query-internal temporaries).
    pub fn synthetic(kind: InfluenceKindId) -> Self {
        Self {
            kind,
            touch_count: 1,
            last_batch: BatchId(0),
        }
    }
}

/// The net effect when two influence kinds co-occur on the same pair of loci.
///
/// Used by `InfluenceKindRegistry::register_interaction` to declare how
/// cross-kind relationships should be interpreted at query time. The engine
/// also applies these rules during the batch loop (activity multiplier after
/// Hebbian plasticity) whenever 2+ kinds co-occur on the same edge in one tick.
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
        Self {
            name,
            default,
            decay: None,
        }
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
        endpoints::directed(from, to)
    }

    /// Shorthand for `Endpoints::Symmetric { a, b }`.
    #[inline]
    pub fn symmetric(a: LocusId, b: LocusId) -> Self {
        endpoints::symmetric(a, b)
    }

    /// Returns `true` if every endpoint in this relationship is contained
    /// in `set`. Used by `World::induced_subgraph`.
    pub fn all_endpoints_in(&self, set: &rustc_hash::FxHashSet<LocusId>) -> bool {
        endpoints::all_endpoints_in(self, set)
    }

    /// Returns `true` if `locus` appears in any endpoint position.
    pub fn involves(&self, locus: LocusId) -> bool {
        endpoints::involves(self, locus)
    }

    /// The endpoint that is not `locus`, treating the relationship as
    /// undirected. For a self-loop (`from == to == locus`), returns `locus`.
    pub fn other_than(&self, locus: LocusId) -> LocusId {
        endpoints::other_than(self, locus)
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
        endpoints::source(self)
    }

    /// For a `Directed` edge, return the target endpoint (`to`).
    /// Returns `None` for `Symmetric` edges.
    pub fn target(&self) -> Option<LocusId> {
        endpoints::target(self)
    }

    /// Canonical lookup key — endpoints flattened into a stable shape so
    /// the relationship store can dedupe hits regardless of insertion
    /// order. For `Symmetric`, the two ids are sorted; for `Directed`,
    /// order is preserved (it carries meaning).
    pub fn key(&self) -> EndpointKey {
        endpoints::key(self)
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
    /// relationship. Usually a single entry; kept as `SmallVec<[_; 2]>`
    /// so the 1-kind and 2-kind cases avoid heap allocation entirely.
    pub kinds_observed: SmallVec<[KindObservation; 2]>,
}

impl RelationshipLineage {
    /// Create a lineage for a newly auto-emerged relationship (single kind, change-attributed).
    #[inline]
    pub fn new_emerged(change_id: ChangeId, kind: InfluenceKindId, batch: BatchId) -> Self {
        lineage::new_emerged(change_id, kind, batch)
    }

    /// Create a lineage for a synthetically created relationship (structural proposals,
    /// test helpers, queries) — no originating change, single observed kind.
    #[inline]
    pub fn new_synthetic(kind: InfluenceKindId) -> Self {
        lineage::new_synthetic(kind)
    }

    /// Create an empty lineage — no observed kinds, no attributed change.
    #[inline]
    pub fn empty() -> Self {
        lineage::empty()
    }

    /// Record or update the observation for `kind` at `batch`.
    ///
    /// If `kind` already appears in `kinds_observed`, increments
    /// `touch_count` and updates `last_batch`. Otherwise appends a new
    /// `KindObservation`. This is the canonical way for the engine to
    /// update lineage on each relationship touch.
    pub fn observe_kind(&mut self, kind: InfluenceKindId, batch: BatchId) {
        lineage::observe_kind(&mut self.kinds_observed, kind, batch);
    }

    /// Return the influence kind that has flowed through this relationship
    /// the most times. Ties are broken by kind id (lower wins). Returns
    /// `None` when `kinds_observed` is empty.
    pub fn dominant_flow_kind(&self) -> Option<InfluenceKindId> {
        lineage::dominant_flow_kind(&self.kinds_observed)
    }

    /// Number of times `kind` has touched this relationship. Returns `0`
    /// when the kind has never been observed.
    pub fn touch_count_for(&self, kind: InfluenceKindId) -> u64 {
        lineage::touch_count_for(&self.kinds_observed, kind)
    }

    /// Returns `true` when `kind` has been observed on this relationship.
    pub fn has_seen_kind(&self, kind: InfluenceKindId) -> bool {
        lineage::has_seen_kind(&self.kinds_observed, kind)
    }

    /// Iterate the ids of all influence kinds that have been observed.
    pub fn observed_kind_ids(&self) -> impl Iterator<Item = InfluenceKindId> + '_ {
        self.kinds_observed.iter().map(|o| o.kind)
    }
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
        self.state
            .as_slice()
            .get(Self::ACTIVITY_SLOT)
            .copied()
            .unwrap_or(0.0)
    }

    /// Read the learned Hebbian weight (slot 1).
    pub fn weight(&self) -> f32 {
        self.state
            .as_slice()
            .get(Self::WEIGHT_SLOT)
            .copied()
            .unwrap_or(0.0)
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
                kinds_observed: smallvec::SmallVec::new(),
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

    // ── RelationshipLineage: observe_kind, dominant_flow_kind ─────────────────

    fn empty_lineage() -> RelationshipLineage {
        RelationshipLineage {
            created_by: None,
            last_touched_by: None,
            change_count: 0,
            kinds_observed: smallvec::SmallVec::new(),
        }
    }

    #[test]
    fn observe_kind_appends_first_touch() {
        let mut lin = empty_lineage();
        lin.observe_kind(InfluenceKindId(1), BatchId(5));
        assert_eq!(lin.kinds_observed.len(), 1);
        assert_eq!(lin.kinds_observed[0].touch_count, 1);
        assert_eq!(lin.kinds_observed[0].last_batch, BatchId(5));
    }

    #[test]
    fn observe_kind_increments_existing_count() {
        let mut lin = empty_lineage();
        lin.observe_kind(InfluenceKindId(1), BatchId(1));
        lin.observe_kind(InfluenceKindId(1), BatchId(3));
        lin.observe_kind(InfluenceKindId(1), BatchId(7));
        assert_eq!(lin.kinds_observed.len(), 1);
        let obs = &lin.kinds_observed[0];
        assert_eq!(obs.touch_count, 3);
        assert_eq!(obs.last_batch, BatchId(7));
    }

    #[test]
    fn observe_kind_tracks_multiple_kinds_separately() {
        let mut lin = empty_lineage();
        lin.observe_kind(InfluenceKindId(1), BatchId(1));
        lin.observe_kind(InfluenceKindId(2), BatchId(2));
        lin.observe_kind(InfluenceKindId(1), BatchId(3));
        assert_eq!(lin.kinds_observed.len(), 2);
        assert_eq!(lin.touch_count_for(InfluenceKindId(1)), 2);
        assert_eq!(lin.touch_count_for(InfluenceKindId(2)), 1);
    }

    #[test]
    fn dominant_flow_kind_returns_most_touched() {
        let mut lin = empty_lineage();
        lin.observe_kind(InfluenceKindId(1), BatchId(1));
        lin.observe_kind(InfluenceKindId(2), BatchId(2));
        lin.observe_kind(InfluenceKindId(2), BatchId(3));
        lin.observe_kind(InfluenceKindId(2), BatchId(4));
        assert_eq!(lin.dominant_flow_kind(), Some(InfluenceKindId(2)));
    }

    #[test]
    fn dominant_flow_kind_breaks_tie_by_lower_id() {
        let mut lin = empty_lineage();
        lin.observe_kind(InfluenceKindId(5), BatchId(1));
        lin.observe_kind(InfluenceKindId(2), BatchId(2));
        // Both have touch_count = 1; lower id (2) should win.
        assert_eq!(lin.dominant_flow_kind(), Some(InfluenceKindId(2)));
    }

    #[test]
    fn dominant_flow_kind_empty_is_none() {
        assert_eq!(empty_lineage().dominant_flow_kind(), None);
    }

    #[test]
    fn touch_count_for_returns_zero_for_unseen() {
        let lin = empty_lineage();
        assert_eq!(lin.touch_count_for(InfluenceKindId(99)), 0);
    }

    #[test]
    fn has_seen_kind_true_and_false() {
        let mut lin = empty_lineage();
        lin.observe_kind(InfluenceKindId(1), BatchId(0));
        assert!(lin.has_seen_kind(InfluenceKindId(1)));
        assert!(!lin.has_seen_kind(InfluenceKindId(2)));
    }

    #[test]
    fn observed_kind_ids_yields_all_registered_kinds() {
        let mut lin = empty_lineage();
        lin.observe_kind(InfluenceKindId(3), BatchId(0));
        lin.observe_kind(InfluenceKindId(7), BatchId(0));
        let mut ids: Vec<_> = lin.observed_kind_ids().collect();
        ids.sort();
        assert_eq!(ids, vec![InfluenceKindId(3), InfluenceKindId(7)]);
    }
}
