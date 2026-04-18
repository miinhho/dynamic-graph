//! Serializable query API for graph-query.
//!
//! Provides a single [`Query`] enum that covers all query operations in this
//! crate, a matching [`QueryResult`] enum with owned return values, and an
//! [`execute`] function that dispatches to the underlying implementations.
//!
//! The entire module is `#[cfg_attr(feature = "serde", ...)]` — enable the
//! `serde` feature to get JSON/binary serialization for free.
//!
//! ## Usage
//!
//! ```ignore
//! use graph_query::api::{Query, execute};
//!
//! // Filtered + sorted + limited search — returns full summaries, no second lookup needed
//! let q = Query::FindRelationships {
//!     predicates: vec![
//!         RelationshipPredicate::OfKind(SUPPLY_KIND),
//!         RelationshipPredicate::ActivityAbove(0.3),
//!     ],
//!     sort_by: Some(RelSort::ActivityDesc),
//!     limit: Some(10),
//! };
//! let QueryResult::RelationshipSummaries(rows) = execute(&world, &q) else { unreachable!() };
//! for r in &rows { println!("L{}→L{}  activity={:.3}", r.from.0, r.to.0, r.activity); }
//! ```
//!
//! ## Predicate types
//!
//! [`LocusPredicate`], [`RelationshipPredicate`], and [`EntityPredicate`] are
//! flat AND-able filter lists. Closure-based predicates are intentionally
//! **not** supported (not serializable). Use the builder API for arbitrary closures.

use graph_core::{
    BatchId, ChangeId, CohereId, EndpointKey, Endpoints, EntityId, InfluenceKindId, LocusId,
    LocusKindId, RelationshipId, RelationshipKindId,
};
use graph_world::World;

// ─── Sort keys ────────────────────────────────────────────────────────────────

/// Sort order for [`Query::FindRelationships`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RelSort {
    /// Descending by `activity()`.
    ActivityDesc,
    /// Descending by `strength()` (activity + weight).
    StrengthDesc,
    /// Descending by `weight()`.
    WeightDesc,
    /// Descending by `change_count`.
    ChangeCountDesc,
    /// Ascending by `created_batch` (oldest first).
    CreatedBatchAsc,
}

/// Sort order for [`Query::FindLoci`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LocusSort {
    /// Descending by `state[slot]`.
    StateDesc(usize),
    /// Descending by total degree.
    DegreeDesc,
}

/// Sort order for [`Query::FindEntities`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EntitySort {
    /// Descending by coherence score.
    CoherenceDesc,
    /// Descending by member count.
    MemberCountDesc,
}

// ─── Predicate types ──────────────────────────────────────────────────────────

/// A serializable filter for loci. All elements in a `Vec<LocusPredicate>` are
/// ANDed together.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LocusPredicate {
    /// Keep only loci of the given kind.
    OfKind(LocusKindId),
    /// Keep only loci where `state[slot] >= min`.
    StateAbove { slot: usize, min: f32 },
    /// Keep only loci where `state[slot] <= max`.
    StateBelow { slot: usize, max: f32 },
    /// Keep only loci whose total degree ≥ `min`.
    MinDegree(usize),
    /// Keep only loci that have a string property `key` equal to `value`.
    StrPropertyEq { key: String, value: String },
    /// Keep only loci that have a numeric property `key` ≥ `min`.
    F64PropertyAbove { key: String, min: f64 },
    /// Keep only loci reachable from `start` within `depth` undirected hops.
    ReachableFrom { start: LocusId, depth: usize },
    /// Keep only loci downstream of `start` within `depth` directed hops.
    DownstreamOf { start: LocusId, depth: usize },
    /// Keep only loci upstream of `start` within `depth` directed hops.
    UpstreamOf { start: LocusId, depth: usize },
    /// Like `ReachableFrom` but only traverses edges with `activity >= min_activity`.
    ///
    /// Dormant edges are pruned *during* BFS — loci reachable only through
    /// them are excluded. Use this to query the **live-signal subgraph**.
    ReachableFromActive {
        start: LocusId,
        depth: usize,
        min_activity: f32,
    },
    /// Like `DownstreamOf` but only traverses forward edges with `activity >= min_activity`.
    DownstreamOfActive {
        start: LocusId,
        depth: usize,
        min_activity: f32,
    },
    /// Like `UpstreamOf` but only traverses backward edges with `activity >= min_activity`.
    UpstreamOfActive {
        start: LocusId,
        depth: usize,
        min_activity: f32,
    },
}

/// A serializable filter for relationships. All elements in a
/// `Vec<RelationshipPredicate>` are ANDed together.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RelationshipPredicate {
    /// Keep only relationships of the given influence kind.
    OfKind(InfluenceKindId),
    /// Keep only directed relationships originating from `locus`.
    From(LocusId),
    /// Keep only directed relationships terminating at `locus`.
    To(LocusId),
    /// Keep only relationships involving `locus` at either endpoint.
    Touching(LocusId),
    /// Keep only relationships whose activity > `min`.
    ActivityAbove(f32),
    /// Keep only relationships whose combined strength > `min`.
    StrengthAbove(f32),
    /// Keep only relationships where `state[slot] >= min`.
    SlotAbove { slot: usize, min: f32 },
    /// Keep only relationships created within `[from, to]` batch range.
    CreatedInRange { from: BatchId, to: BatchId },
    /// Keep only relationships older than `min_batches` (age = current - created).
    OlderThan {
        current_batch: BatchId,
        min_batches: u64,
    },
    /// Keep only relationships with change count ≥ `min`.
    MinChangeCount(u64),
}

/// A serializable filter for entities. All elements in a
/// `Vec<EntityPredicate>` are ANDed together.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EntityPredicate {
    /// Keep only entities with coherence ≥ `min`.
    CoherenceAbove(f32),
    /// Keep only entities that contain `locus` as a member.
    HasMember(LocusId),
    /// Keep only entities with at least `min` members.
    MinMembers(usize),
}

// ─── Summary types (owned, no borrowed references) ────────────────────────────

/// Owned summary of a single relationship — returned by `FindRelationships`.
///
/// Carries all fields callers typically need immediately after a search so a
/// second `world.relationships().get(id)` lookup is not required.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RelationshipSummary {
    pub id: RelationshipId,
    pub kind: InfluenceKindId,
    /// Source locus for directed edges; `a` endpoint for symmetric edges.
    pub from: LocusId,
    /// Target locus for directed edges; `b` endpoint for symmetric edges.
    pub to: LocusId,
    /// `true` for directed edges, `false` for symmetric.
    pub directed: bool,
    pub activity: f32,
    pub weight: f32,
    pub change_count: u64,
    pub created_batch: BatchId,
}

/// Owned summary of a single locus — returned by `FindLoci`.
///
/// Carries the full state vector so callers can read any slot without a second
/// lookup into the world.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LocusSummary {
    pub id: LocusId,
    pub kind: LocusKindId,
    /// Full state vector (cloned from the locus's `StateVector`).
    pub state: Vec<f32>,
}

/// Owned snapshot of an entity's deviation since a baseline batch.
///
/// Mirrors [`crate::EntityDiff`] with all fields owned and serde-compatible.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EntityDiffSummary {
    pub entity_id: EntityId,
    pub born_after_baseline: bool,
    pub went_dormant: bool,
    pub revived: bool,
    pub members_added: Vec<LocusId>,
    pub members_removed: Vec<LocusId>,
    pub membership_event_count: u32,
    pub coherence_at_baseline: f32,
    pub coherence_now: f32,
    pub coherence_delta: f32,
    pub member_count_delta: i64,
    pub latest_change_batch: Option<BatchId>,
}

/// Owned snapshot of a single cohere cluster.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CohereResult {
    pub id: CohereId,
    pub entity_ids: Vec<EntityId>,
    pub relationship_ids: Vec<RelationshipId>,
    pub strength: f32,
}

/// Activity trend over a batch window.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TrendResult {
    /// Activity increasing at `slope` per batch.
    Rising { slope: f32 },
    /// Activity decreasing at `slope` per batch (slope is negative).
    Falling { slope: f32 },
    /// Activity stable (|slope| ≤ threshold).
    Stable,
    /// Fewer than 2 data points — trend undefined.
    Insufficient,
}

// ─── Query enum ───────────────────────────────────────────────────────────────

/// A serializable query that can be executed against a [`World`].
///
/// Covers all major operations in `graph-query`:
/// - Structural traversal (path, reachability, components)
/// - Centrality metrics (PageRank, betweenness, closeness, Louvain)
/// - Causal log queries (ancestors, descendants, root stimuli)
/// - Filtered entity/relationship/locus lookups **with sort and limit**
/// - Relationship profiles
/// - Activity trend analysis
/// - Entity deviation detection
/// - Counterfactual relationship analysis
/// - Cohere cluster queries
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Query {
    // ── Structural traversal ─────────────────────────────────────────────────
    /// BFS shortest path between two loci (undirected). Returns `None` if
    /// unreachable.
    PathBetween { from: LocusId, to: LocusId },

    /// BFS shortest path restricted to a specific relationship kind.
    PathBetweenOfKind {
        from: LocusId,
        to: LocusId,
        kind: InfluenceKindId,
    },

    /// Directed (source → target) shortest path.
    DirectedPath { from: LocusId, to: LocusId },

    /// All loci reachable from `start` within `depth` undirected hops.
    ReachableFrom { start: LocusId, depth: usize },

    /// All loci reachable following directed edges forward from `start`.
    DownstreamOf { start: LocusId, depth: usize },

    /// All loci reachable following directed edges backward from `start`.
    UpstreamOf { start: LocusId, depth: usize },

    /// Partition all loci into connected components (undirected).
    ConnectedComponents,

    /// Connected components restricted to a specific relationship kind.
    ConnectedComponentsOfKind(InfluenceKindId),

    /// All loci reachable from `start` within `depth` undirected hops,
    /// traversing only edges with `activity >= min_activity`.
    ///
    /// Dormant edges are pruned during BFS — not post-filtered.
    /// Returns loci in the **live-signal subgraph** of the running simulation.
    ReachableFromActive {
        start: LocusId,
        depth: usize,
        min_activity: f32,
    },

    /// Directed downstream reachability restricted to active edges.
    DownstreamOfActive {
        start: LocusId,
        depth: usize,
        min_activity: f32,
    },

    /// Directed upstream reachability restricted to active edges.
    UpstreamOfActive {
        start: LocusId,
        depth: usize,
        min_activity: f32,
    },

    /// BFS shortest path restricted to edges with `activity >= min_activity`.
    ///
    /// Returns `None` if no path exists through sufficiently active edges.
    PathBetweenActive {
        from: LocusId,
        to: LocusId,
        min_activity: f32,
    },

    /// All immediate neighbors of `locus` (undirected).
    NeighborsOf(LocusId),

    /// Loci with no relationships.
    IsolatedLoci,

    /// Top `n` loci by total degree (most-connected first).
    HubLoci(usize),

    /// The reciprocal relationship of a given relationship (reverse direction), if any.
    ReciprocOf(RelationshipId),

    /// All pairs (fwd_id, rev_id) of reciprocal relationships.
    ReciprocPairs,

    /// Whether the relationship graph contains any directed cycle.
    HasCycle,

    /// The highest-weight path between two loci.
    StrongestPath { from: LocusId, to: LocusId },

    // ── Centrality ───────────────────────────────────────────────────────────
    /// PageRank score for every locus (or top `limit` if given).
    PageRank {
        /// Damping factor (typically 0.85).
        damping: f32,
        /// Max iterations.
        iterations: usize,
        /// Convergence tolerance.
        tolerance: f32,
        /// If `Some(n)`, return only the top `n` loci by score.
        limit: Option<usize>,
    },

    /// PageRank score for a single locus.
    PageRankFor {
        locus: LocusId,
        damping: f32,
        iterations: usize,
        tolerance: f32,
    },

    /// Betweenness centrality for every locus (or top `limit`).
    AllBetweenness { limit: Option<usize> },

    /// Betweenness centrality for a single locus.
    BetweennessFor(LocusId),

    /// Harmonic closeness centrality for every locus (or top `limit`).
    AllCloseness { limit: Option<usize> },

    /// Harmonic closeness centrality for a single locus.
    ClosenessFor(LocusId),

    /// Burt's structural constraint for every locus (or top `limit`).
    AllConstraints { limit: Option<usize> },

    /// Burt's structural constraint for a single locus.
    ConstraintFor(LocusId),

    /// Community detection via Louvain (default resolution = 1.0).
    Louvain,

    /// Community detection with a custom resolution parameter.
    LouvainWithResolution(f32),

    /// Newman–Girvan modularity of the current community partition.
    Modularity,

    // ── Causal log queries ───────────────────────────────────────────────────
    /// All causal ancestors of a change (BFS over predecessor DAG).
    CausalAncestors(ChangeId),

    /// All causal descendants of a change.
    CausalDescendants(ChangeId),

    /// Causal depth (longest predecessor chain) of a change.
    CausalDepth(ChangeId),

    /// Whether `ancestor` is a causal ancestor of `descendant`.
    IsAncestorOf {
        ancestor: ChangeId,
        descendant: ChangeId,
    },

    /// Leaf ancestors (no predecessors) of a change — original stimuli.
    RootStimuli(ChangeId),

    /// All changes to a locus within a batch range.
    ChangesToLocusInRange {
        locus: LocusId,
        from: BatchId,
        to: BatchId,
    },

    /// All changes to a relationship within a batch range.
    ChangesToRelationshipInRange {
        relationship: RelationshipId,
        from: BatchId,
        to: BatchId,
    },

    /// All loci changed in a specific batch.
    LociChangedInBatch(BatchId),

    /// All relationships changed in a specific batch.
    RelationshipsChangedInBatch(BatchId),

    // ── Filtered lookups with sort + limit ───────────────────────────────────
    /// Find loci matching all given predicates (AND), with optional sort and limit.
    ///
    /// Returns [`QueryResult::LocusSummaries`] — includes kind and full state
    /// vector so callers never need a second lookup.
    FindLoci {
        predicates: Vec<LocusPredicate>,
        sort_by: Option<LocusSort>,
        limit: Option<usize>,
    },

    /// Find relationships matching all given predicates (AND), with optional sort and limit.
    ///
    /// Returns [`QueryResult::RelationshipSummaries`] — includes kind, endpoints,
    /// activity, and weight so callers never need a second lookup.
    FindRelationships {
        predicates: Vec<RelationshipPredicate>,
        sort_by: Option<RelSort>,
        limit: Option<usize>,
    },

    /// Find active entities matching all given predicates (AND), with optional sort and limit.
    FindEntities {
        predicates: Vec<EntityPredicate>,
        sort_by: Option<EntitySort>,
        limit: Option<usize>,
    },

    // ── Single locus state lookup ────────────────────────────────────────────
    /// Read a single slot of a locus's state vector.
    ///
    /// Common pattern in examples:
    /// `world.locus(id).map(|l| l.state[slot]).unwrap_or(0.0)`
    LocusStateSlot { locus: LocusId, slot: usize },

    // ── Relationship profiles ────────────────────────────────────────────────
    /// Full relationship bundle (all edges between two loci + metadata).
    RelationshipProfile { from: LocusId, to: LocusId },

    // ── Activity trend analysis ──────────────────────────────────────────────
    /// OLS regression on a relationship's activity over a batch window.
    ///
    /// Returns `Trend::Insufficient` when fewer than 2 log entries exist.
    ActivityTrend {
        relationship: RelationshipId,
        from_batch: BatchId,
        to_batch: BatchId,
    },

    // ── Entity deviation detection ───────────────────────────────────────────
    /// How have entities changed since `baseline_batch`?
    ///
    /// Returns deviations for every entity that exists or existed since the
    /// baseline. Filter by `coherence_delta`, `went_dormant`, etc. after.
    EntityDeviationsSince(BatchId),

    // ── Counterfactual analysis ──────────────────────────────────────────────
    /// Which relationships would not exist without these root changes?
    ///
    /// `root_changes` is typically the output of `ChangesToLocusInRange` or
    /// `RootStimuli` for a specific stimulus batch.
    RelationshipsAbsentWithout(Vec<ChangeId>),

    // ── Cohere cluster queries ───────────────────────────────────────────────
    /// All cohere clusters stored under the default perspective key (`"default"`).
    Coheres,

    /// All cohere clusters stored under a named perspective key.
    CoheresNamed(String),

    // ── Metrics / aggregation ────────────────────────────────────────────────
    /// World-wide summary statistics.
    WorldMetrics,

    // ── Causal strength (STDP-weight derived) ────────────────────────────────
    /// Net causal direction between two loci for a given influence kind.
    ///
    /// Returns a value in `[-1.0, 1.0]` (positive = `from` causes `to`).
    /// Meaningful only when STDP plasticity is active on `kind`.
    CausalDirection {
        from: LocusId,
        to: LocusId,
        kind: InfluenceKindId,
    },

    /// Top-N loci that most consistently cause `target` (highest incoming weight).
    ///
    /// Returns `QueryResult::LocusScores` sorted descending by weight.
    DominantCauses {
        target: LocusId,
        kind: InfluenceKindId,
        n: usize,
    },

    /// Top-N loci most consistently caused by `source` (highest outgoing weight).
    ///
    /// Returns `QueryResult::LocusScores` sorted descending by weight.
    DominantEffects {
        source: LocusId,
        kind: InfluenceKindId,
        n: usize,
    },

    /// Sum of directed incoming weights to `locus` for `kind`.
    CausalInStrength {
        locus: LocusId,
        kind: InfluenceKindId,
    },

    /// Sum of directed outgoing weights from `locus` for `kind`.
    CausalOutStrength {
        locus: LocusId,
        kind: InfluenceKindId,
    },

    /// Locus pairs with roughly balanced A→B and B→A weights (oscillators/feedback).
    ///
    /// Returns `QueryResult::FeedbackPairs`.
    FeedbackPairs {
        kind: InfluenceKindId,
        min_weight: f32,
        min_balance: f32,
    },

    // ── D2: Granger-style temporal causality ─────────────────────────────────
    /// Empirical Granger score: fraction of `from`'s changes followed by a
    /// `to` change within `lag_batches`. Returns `QueryResult::Score`.
    GrangerScore {
        from: LocusId,
        to: LocusId,
        kind: InfluenceKindId,
        lag_batches: u64,
    },

    /// Top-N causes of `target` ranked by Granger score.
    /// Returns `QueryResult::LocusScores`.
    GrangerDominantCauses {
        target: LocusId,
        kind: InfluenceKindId,
        lag_batches: u64,
        n: usize,
    },

    /// Top-N effects of `source` ranked by Granger score.
    /// Returns `QueryResult::LocusScores`.
    GrangerDominantEffects {
        source: LocusId,
        kind: InfluenceKindId,
        lag_batches: u64,
        n: usize,
    },

    // ── B3: Time-travel queries ───────────────────────────────────────────
    /// Reconstruct the structural inverse diff to reach `target_batch`.
    /// Returns `QueryResult::TimeTravelResult`.
    TimeTravel { target_batch: BatchId },

    // ── D3: Structural counterfactual replay ─────────────────────────────
    /// Compute the structural impact of removing `remove_changes` from the world.
    /// Returns `QueryResult::Counterfactual`.
    CounterfactualReplay { remove_changes: Vec<ChangeId> },

    // ── D4: Entity-level causality ───────────────────────────────────────
    /// Get the lifecycle cause for `entity_id`'s layer at `at_batch`.
    /// Returns `QueryResult::EntityCause`.
    EntityTransitionCause {
        entity_id: EntityId,
        at_batch: BatchId,
    },

    /// Find upstream entity transitions that caused `entity_id`'s transition at `at_batch`.
    /// Returns `QueryResult::EntityTransitions`.
    EntityUpstreamTransitions {
        entity_id: EntityId,
        at_batch: BatchId,
    },

    /// List all lifecycle layers of `entity_id` in the batch range `[from, to)`.
    /// Returns `QueryResult::EntityLayers`.
    EntityLayersInRange {
        entity_id: EntityId,
        from: BatchId,
        to: BatchId,
    },
}

// ─── Result enum ─────────────────────────────────────────────────────────────

/// The owned result of executing a [`Query`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum QueryResult {
    /// A single optional path (list of locus IDs from source to sink).
    Path(Option<Vec<LocusId>>),

    /// A flat list of locus IDs (traversal results: ReachableFrom, etc.).
    Loci(Vec<LocusId>),

    /// A partition of loci into connected components.
    Components(Vec<Vec<LocusId>>),

    /// A flat list of change IDs.
    Changes(Vec<ChangeId>),

    /// A flat list of relationship IDs (traversal: ReciprocPairs, etc.).
    Relationships(Vec<RelationshipId>),

    /// Full summaries for `FindRelationships` — activity, kind, endpoints included.
    RelationshipSummaries(Vec<RelationshipSummary>),

    /// Full summaries for `FindLoci` — kind and state vector included.
    LocusSummaries(Vec<LocusSummary>),

    /// A flat list of entity IDs.
    Entities(Vec<EntityId>),

    /// A boolean answer.
    Bool(bool),

    /// A single unsigned integer (count, depth, …).
    Count(usize),

    /// A single floating-point score.
    Score(f32),

    /// An optional floating-point score.
    MaybeScore(Option<f32>),

    /// Per-locus scores (locus ID, score), sorted descending.
    LocusScores(Vec<(LocusId, f32)>),

    /// Loci grouped by community (each inner Vec is one community).
    Communities(Vec<Vec<LocusId>>),

    /// Activity trend over a batch window.
    Trend(TrendResult),

    /// Entity deviations since a baseline batch.
    EntityDeviations(Vec<EntityDiffSummary>),

    /// Cohere cluster results.
    Coheres(Vec<CohereResult>),

    /// Relationship profile summary.
    RelationshipProfile(RelationshipProfileResult),

    /// World-wide metrics snapshot.
    WorldMetrics(WorldMetricsResult),

    /// Feedback-loop pairs: `(locus_a, locus_b, balance_ratio)` sorted by balance descending.
    FeedbackPairs(Vec<(LocusId, LocusId, f32)>),

    /// Time-travel inverse diff result.
    TimeTravelResult(crate::TimeTravelResult),

    /// Counterfactual structural impact.
    Counterfactual(crate::CounterfactualDiff),

    /// A lifecycle cause for an entity transition. `None` when entity or layer not found.
    EntityCause(Option<graph_core::LifecycleCause>),

    /// Upstream entity transitions: `(entity_id, batch_id)` pairs.
    EntityTransitions(Vec<(EntityId, BatchId)>),

    /// Entity lifecycle layers in a batch range:
    /// `(batch, transition, cause)` tuples.
    EntityLayers(
        Vec<(
            BatchId,
            graph_core::LayerTransition,
            graph_core::LifecycleCause,
        )>,
    ),
}

/// Owned snapshot of key relationship profile fields.
///
/// Returned by [`Query::RelationshipProfile`]. All fields are owned and
/// serde-compatible.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RelationshipProfileResult {
    pub from: LocusId,
    pub to: LocusId,
    /// IDs of all relationships between this pair (any kind, either direction).
    pub relationship_ids: Vec<RelationshipId>,
    /// Sum of activity across all edges in the bundle.
    pub total_activity: f32,
    /// Net directed influence (forward activity − backward activity).
    pub net_influence: f32,
    /// The influence kind with the highest total activity, if any.
    pub dominant_kind: Option<InfluenceKindId>,
    /// Per-kind activity breakdown, sorted descending by activity.
    pub activity_by_kind: Vec<(InfluenceKindId, f32)>,
}

/// Owned world-wide metrics snapshot.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WorldMetricsResult {
    pub locus_count: usize,
    pub relationship_count: usize,
    pub active_relationship_count: usize,
    pub mean_activity: f32,
    pub max_activity: f32,
    pub component_count: usize,
    pub largest_component_size: usize,
    pub max_degree: usize,
}

// ─── execute ─────────────────────────────────────────────────────────────────

/// Execute a [`Query`] against `world` and return an owned [`QueryResult`].
///
/// This is a pure read operation — `world` is borrowed immutably.
pub fn execute(world: &World, query: &Query) -> QueryResult {
    use crate::traversal::{
        downstream_of_active, path_between_active, reachable_from_active, upstream_of_active,
    };
    use crate::*;

    match query {
        // ── Structural traversal ─────────────────────────────────────────────
        Query::PathBetween { from, to } => QueryResult::Path(path_between(world, *from, *to)),

        Query::PathBetweenOfKind { from, to, kind } => {
            QueryResult::Path(path_between_of_kind(world, *from, *to, *kind))
        }

        Query::DirectedPath { from, to } => QueryResult::Path(directed_path(world, *from, *to)),

        Query::ReachableFrom { start, depth } => {
            QueryResult::Loci(reachable_from(world, *start, *depth))
        }

        Query::DownstreamOf { start, depth } => {
            QueryResult::Loci(downstream_of(world, *start, *depth))
        }

        Query::UpstreamOf { start, depth } => QueryResult::Loci(upstream_of(world, *start, *depth)),

        Query::ReachableFromActive {
            start,
            depth,
            min_activity,
        } => QueryResult::Loci(reachable_from_active(world, *start, *depth, *min_activity)),

        Query::DownstreamOfActive {
            start,
            depth,
            min_activity,
        } => QueryResult::Loci(downstream_of_active(world, *start, *depth, *min_activity)),

        Query::UpstreamOfActive {
            start,
            depth,
            min_activity,
        } => QueryResult::Loci(upstream_of_active(world, *start, *depth, *min_activity)),

        Query::PathBetweenActive {
            from,
            to,
            min_activity,
        } => QueryResult::Path(path_between_active(world, *from, *to, *min_activity)),

        Query::ConnectedComponents => QueryResult::Components(connected_components(world)),

        Query::ConnectedComponentsOfKind(kind) => {
            QueryResult::Components(connected_components_of_kind(world, *kind))
        }

        Query::NeighborsOf(locus) => QueryResult::Loci(neighbors_of(world, *locus)),

        Query::IsolatedLoci => QueryResult::Loci(isolated_loci(world)),

        Query::HubLoci(n) => QueryResult::Loci(hub_loci(world, *n)),

        Query::ReciprocOf(rel_id) => {
            let result = reciprocal_of(world, *rel_id);
            QueryResult::Relationships(result.map(|id| vec![id]).unwrap_or_default())
        }

        Query::ReciprocPairs => {
            let pairs = reciprocal_pairs(world);
            let flat: Vec<RelationshipId> = pairs.into_iter().flat_map(|(a, b)| [a, b]).collect();
            QueryResult::Relationships(flat)
        }

        Query::HasCycle => QueryResult::Bool(has_cycle(world)),

        Query::StrongestPath { from, to } => QueryResult::Path(strongest_path(world, *from, *to)),

        // ── Centrality ───────────────────────────────────────────────────────
        Query::PageRank {
            damping,
            iterations,
            tolerance,
            limit,
        } => {
            let mut scores = pagerank(world, *damping, *iterations, *tolerance);
            scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
            if let Some(n) = limit {
                scores.truncate(*n);
            }
            QueryResult::LocusScores(scores)
        }

        Query::PageRankFor {
            locus,
            damping,
            iterations,
            tolerance,
        } => {
            let scores = pagerank(world, *damping, *iterations, *tolerance);
            let map: rustc_hash::FxHashMap<LocusId, f32> = scores.into_iter().collect();
            QueryResult::MaybeScore(map.get(locus).copied())
        }

        Query::AllBetweenness { limit } => {
            let mut scores: Vec<(LocusId, f32)> = all_betweenness(world).into_iter().collect();
            scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
            if let Some(n) = limit {
                scores.truncate(*n);
            }
            QueryResult::LocusScores(scores)
        }

        Query::BetweennessFor(locus) => QueryResult::Score(betweenness_centrality(world, *locus)),

        Query::AllCloseness { limit } => {
            let mut scores: Vec<(LocusId, f32)> = all_closeness(world).into_iter().collect();
            scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
            if let Some(n) = limit {
                scores.truncate(*n);
            }
            QueryResult::LocusScores(scores)
        }

        Query::ClosenessFor(locus) => QueryResult::MaybeScore(closeness_centrality(world, *locus)),

        Query::AllConstraints { limit } => {
            let mut scores: Vec<(LocusId, f32)> = all_constraints(world).into_iter().collect();
            scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
            if let Some(n) = limit {
                scores.truncate(*n);
            }
            QueryResult::LocusScores(scores)
        }

        Query::ConstraintFor(locus) => {
            QueryResult::MaybeScore(structural_constraint(world, *locus))
        }

        Query::Louvain => QueryResult::Communities(louvain(world)),

        Query::LouvainWithResolution(resolution) => {
            QueryResult::Communities(louvain_with_resolution(world, *resolution))
        }

        Query::Modularity => {
            let communities = louvain(world);
            QueryResult::Score(modularity(world, &communities))
        }

        // ── Causal log queries ───────────────────────────────────────────────
        Query::CausalAncestors(change_id) => {
            QueryResult::Changes(causal_ancestors(world, *change_id))
        }

        Query::CausalDescendants(change_id) => {
            QueryResult::Changes(causal_descendants(world, *change_id))
        }

        Query::CausalDepth(change_id) => QueryResult::Count(causal_depth(world, *change_id)),

        Query::IsAncestorOf {
            ancestor,
            descendant,
        } => QueryResult::Bool(is_ancestor_of(world, *ancestor, *descendant)),

        Query::RootStimuli(change_id) => QueryResult::Changes(root_stimuli(world, *change_id)),

        Query::ChangesToLocusInRange { locus, from, to } => {
            let changes = changes_to_locus_in_range(world, *locus, *from, *to);
            QueryResult::Changes(changes.into_iter().map(|c| c.id).collect())
        }

        Query::ChangesToRelationshipInRange {
            relationship,
            from,
            to,
        } => {
            let changes = changes_to_relationship_in_range(world, *relationship, *from, *to);
            QueryResult::Changes(changes.into_iter().map(|c| c.id).collect())
        }

        Query::LociChangedInBatch(batch) => QueryResult::Loci(loci_changed_in_batch(world, *batch)),

        Query::RelationshipsChangedInBatch(batch) => {
            QueryResult::Relationships(relationships_changed_in_batch(world, *batch))
        }

        // ── Filtered lookups ─────────────────────────────────────────────────
        Query::FindLoci {
            predicates,
            sort_by,
            limit,
        } => {
            let mut summaries = find_loci_summaries(world, predicates);
            if let Some(sort) = sort_by {
                match sort {
                    LocusSort::StateDesc(slot) => {
                        summaries.sort_unstable_by(|a, b| {
                            let va = a.state.get(*slot).copied().unwrap_or(f32::NEG_INFINITY);
                            let vb = b.state.get(*slot).copied().unwrap_or(f32::NEG_INFINITY);
                            vb.total_cmp(&va)
                        });
                    }
                    LocusSort::DegreeDesc => {
                        summaries.sort_unstable_by_key(|s| std::cmp::Reverse(world.degree(s.id)));
                    }
                }
            }
            if let Some(n) = limit {
                summaries.truncate(*n);
            }
            QueryResult::LocusSummaries(summaries)
        }

        Query::FindRelationships {
            predicates,
            sort_by,
            limit,
        } => {
            let summaries =
                find_relationship_summaries(world, predicates, sort_by.as_ref(), *limit);
            QueryResult::RelationshipSummaries(summaries)
        }

        Query::FindEntities {
            predicates,
            sort_by,
            limit,
        } => {
            let mut ids = find_entities_inner(world, predicates);
            if let Some(sort) = sort_by {
                match sort {
                    EntitySort::CoherenceDesc => {
                        ids.sort_unstable_by(|a, b| {
                            let ca = world
                                .entities()
                                .get(*a)
                                .map(|e| e.current.coherence)
                                .unwrap_or(0.0);
                            let cb = world
                                .entities()
                                .get(*b)
                                .map(|e| e.current.coherence)
                                .unwrap_or(0.0);
                            cb.total_cmp(&ca)
                        });
                    }
                    EntitySort::MemberCountDesc => {
                        ids.sort_unstable_by_key(|id| {
                            std::cmp::Reverse(
                                world
                                    .entities()
                                    .get(*id)
                                    .map(|e| e.current.members.len())
                                    .unwrap_or(0),
                            )
                        });
                    }
                }
            }
            if let Some(n) = limit {
                ids.truncate(*n);
            }
            QueryResult::Entities(ids)
        }

        // ── Single locus state lookup ────────────────────────────────────────
        Query::LocusStateSlot { locus, slot } => {
            let v = world
                .locus(*locus)
                .and_then(|l| l.state.as_slice().get(*slot).copied());
            QueryResult::MaybeScore(v)
        }

        // ── Relationship profiles ────────────────────────────────────────────
        Query::RelationshipProfile { from, to } => {
            let bundle = relationship_profile(world, *from, *to);
            use graph_core::Endpoints;
            let forward: f32 = bundle
                .relationships
                .iter()
                .filter(
                    |r| matches!(r.endpoints, Endpoints::Directed { from: f, .. } if f == *from),
                )
                .map(|r| r.activity())
                .sum();
            let backward: f32 = bundle
                .relationships
                .iter()
                .filter(|r| matches!(r.endpoints, Endpoints::Directed { to: t, .. } if t == *from))
                .map(|r| r.activity())
                .sum();
            let activity_by_kind = bundle.activity_by_kind();
            let dominant_kind = bundle.dominant_kind();
            QueryResult::RelationshipProfile(RelationshipProfileResult {
                from: *from,
                to: *to,
                relationship_ids: bundle.relationships.iter().map(|r| r.id).collect(),
                total_activity: bundle.net_activity(),
                net_influence: forward - backward,
                dominant_kind,
                activity_by_kind,
            })
        }

        // ── Activity trend ───────────────────────────────────────────────────
        Query::ActivityTrend {
            relationship,
            from_batch,
            to_batch,
        } => {
            let trend = relationship_activity_trend(world, *relationship, *from_batch, *to_batch);
            let result = match trend {
                None => TrendResult::Insufficient,
                Some(crate::Trend::Rising { slope }) => TrendResult::Rising { slope },
                Some(crate::Trend::Falling { slope }) => TrendResult::Falling { slope },
                Some(crate::Trend::Stable) => TrendResult::Stable,
            };
            QueryResult::Trend(result)
        }

        // ── Entity deviation detection ───────────────────────────────────────
        Query::EntityDeviationsSince(baseline) => {
            let diffs = entity_deviations_since(world, *baseline);
            let summaries = diffs
                .into_iter()
                .map(|d| EntityDiffSummary {
                    entity_id: d.entity_id,
                    born_after_baseline: d.born_after_baseline,
                    went_dormant: d.went_dormant,
                    revived: d.revived,
                    members_added: d.members_added,
                    members_removed: d.members_removed,
                    membership_event_count: d.membership_event_count,
                    coherence_at_baseline: d.coherence_at_baseline,
                    coherence_now: d.coherence_now,
                    coherence_delta: d.coherence_delta,
                    member_count_delta: d.member_count_delta,
                    latest_change_batch: d.latest_change_batch,
                })
                .collect();
            QueryResult::EntityDeviations(summaries)
        }

        // ── Counterfactual analysis ──────────────────────────────────────────
        Query::RelationshipsAbsentWithout(root_changes) => {
            let absent_ids = relationships_absent_without(world, root_changes);
            let summaries = absent_ids
                .iter()
                .filter_map(|&id| world.relationships().get(id))
                .map(rel_to_summary)
                .collect();
            QueryResult::RelationshipSummaries(summaries)
        }

        // ── Cohere cluster queries ───────────────────────────────────────────
        Query::Coheres => QueryResult::Coheres(coheres_to_results(
            world.coheres().get("default").unwrap_or(&[]),
        )),

        Query::CoheresNamed(key) => QueryResult::Coheres(coheres_to_results(
            world.coheres().get(key.as_str()).unwrap_or(&[]),
        )),

        // ── Causal strength ──────────────────────────────────────────────────
        Query::CausalDirection { from, to, kind } => QueryResult::Score(
            crate::causal_strength::causal_direction(world, *from, *to, *kind),
        ),
        Query::DominantCauses { target, kind, n } => QueryResult::LocusScores(
            crate::causal_strength::dominant_causes(world, *target, *kind, *n),
        ),
        Query::DominantEffects { source, kind, n } => QueryResult::LocusScores(
            crate::causal_strength::dominant_effects(world, *source, *kind, *n),
        ),
        Query::CausalInStrength { locus, kind } => QueryResult::Score(
            crate::causal_strength::causal_in_strength(world, *locus, *kind),
        ),
        Query::CausalOutStrength { locus, kind } => QueryResult::Score(
            crate::causal_strength::causal_out_strength(world, *locus, *kind),
        ),
        Query::FeedbackPairs {
            kind,
            min_weight,
            min_balance,
        } => QueryResult::FeedbackPairs(crate::causal_strength::feedback_pairs(
            world,
            *kind,
            *min_weight,
            *min_balance,
        )),

        // ── D2: Granger-style causality ──────────────────────────────────────
        Query::GrangerScore {
            from,
            to,
            kind,
            lag_batches,
        } => QueryResult::Score(crate::causal_strength::granger_score(
            world,
            *from,
            *to,
            *kind,
            *lag_batches,
        )),
        Query::GrangerDominantCauses {
            target,
            kind,
            lag_batches,
            n,
        } => QueryResult::LocusScores(crate::causal_strength::granger_dominant_causes(
            world,
            *target,
            *kind,
            *lag_batches,
            *n,
        )),
        Query::GrangerDominantEffects {
            source,
            kind,
            lag_batches,
            n,
        } => QueryResult::LocusScores(crate::causal_strength::granger_dominant_effects(
            world,
            *source,
            *kind,
            *lag_batches,
            *n,
        )),

        // ── B3: Time-travel queries ───────────────────────────────────────────
        Query::TimeTravel { target_batch } => {
            QueryResult::TimeTravelResult(crate::time_travel::time_travel(world, *target_batch))
        }

        // ── D3: Structural counterfactual replay ─────────────────────────────
        Query::CounterfactualReplay { remove_changes } => {
            QueryResult::Counterfactual(crate::counterfactual_replay(world, remove_changes.clone()))
        }

        // ── D4: Entity-level causality ───────────────────────────────────────
        Query::EntityTransitionCause {
            entity_id,
            at_batch,
        } => QueryResult::EntityCause(crate::entity_causality::entity_transition_cause(
            world, *entity_id, *at_batch,
        )),
        Query::EntityUpstreamTransitions {
            entity_id,
            at_batch,
        } => QueryResult::EntityTransitions(crate::entity_causality::entity_upstream_transitions(
            world, *entity_id, *at_batch,
        )),
        Query::EntityLayersInRange {
            entity_id,
            from,
            to,
        } => QueryResult::EntityLayers(crate::entity_causality::entity_layers_in_range(
            world, *entity_id, *from, *to,
        )),

        // ── Metrics ──────────────────────────────────────────────────────────
        Query::WorldMetrics => {
            let m = world.metrics();
            QueryResult::WorldMetrics(WorldMetricsResult {
                locus_count: m.locus_count,
                relationship_count: m.relationship_count,
                active_relationship_count: m.active_relationship_count,
                mean_activity: m.mean_activity,
                max_activity: m.max_activity,
                component_count: m.component_count,
                largest_component_size: m.largest_component_size,
                max_degree: m.max_degree,
            })
        }
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

fn rel_to_summary(r: &graph_core::Relationship) -> RelationshipSummary {
    use graph_core::Endpoints;
    let (from, to, directed) = match r.endpoints {
        Endpoints::Directed { from, to } => (from, to, true),
        Endpoints::Symmetric { a, b } => (a, b, false),
    };
    RelationshipSummary {
        id: r.id,
        kind: r.kind,
        from,
        to,
        directed,
        activity: r.activity(),
        weight: r.weight(),
        change_count: r.lineage.change_count,
        created_batch: r.created_batch,
    }
}

fn coheres_to_results(coheres: &[graph_core::Cohere]) -> Vec<CohereResult> {
    use graph_core::CohereMembers;
    coheres
        .iter()
        .map(|c| {
            let (entity_ids, relationship_ids) = match &c.members {
                CohereMembers::Entities(ids) => (ids.clone(), vec![]),
                CohereMembers::Relationships(ids) => (vec![], ids.clone()),
                CohereMembers::Mixed {
                    entities,
                    relationships,
                } => (entities.clone(), relationships.clone()),
            };
            CohereResult {
                id: c.id,
                entity_ids,
                relationship_ids,
                strength: c.strength,
            }
        })
        .collect()
}

fn find_loci_summaries(world: &World, predicates: &[LocusPredicate]) -> Vec<LocusSummary> {
    use crate::planner::plan_loci_predicates;
    use crate::traversal::{
        downstream_of, downstream_of_active, reachable_from, reachable_from_active, upstream_of,
        upstream_of_active,
    };
    use rustc_hash::FxHashSet;

    let mut candidates: Vec<LocusId> = world.loci().iter().map(|l| l.id).collect();

    // Apply predicates cheapest-first (planner-sorted) to prune the candidate
    // set quickly. Traversal predicates (BFS) come last since they are most
    // expensive but are then applied to an already-pruned candidate set.
    for pred in plan_loci_predicates(predicates) {
        match pred {
            LocusPredicate::OfKind(kind) => {
                candidates.retain(|&id| world.locus(id).is_some_and(|l| l.kind == *kind));
            }
            LocusPredicate::StateAbove { slot, min } => {
                candidates.retain(|&id| {
                    world
                        .locus(id)
                        .and_then(|l| l.state.as_slice().get(*slot).copied())
                        .is_some_and(|v| v >= *min)
                });
            }
            LocusPredicate::StateBelow { slot, max } => {
                candidates.retain(|&id| {
                    world
                        .locus(id)
                        .and_then(|l| l.state.as_slice().get(*slot).copied())
                        .is_some_and(|v| v <= *max)
                });
            }
            LocusPredicate::StrPropertyEq { key, value } => {
                candidates.retain(|&id| {
                    world
                        .properties()
                        .get(id)
                        .and_then(|p| p.get_str(key))
                        .is_some_and(|v| v == value.as_str())
                });
            }
            LocusPredicate::F64PropertyAbove { key, min } => {
                candidates.retain(|&id| {
                    world
                        .properties()
                        .get(id)
                        .and_then(|p| p.get_f64(key))
                        .is_some_and(|v| v >= *min)
                });
            }
            LocusPredicate::MinDegree(min) => {
                candidates.retain(|&id| world.degree(id) >= *min);
            }
            // ── Active traversal predicates (priority 85) ─────────────────
            // Prune dormant edges during BFS — smaller reachable set than the
            // full-graph variants below, so applied first.
            LocusPredicate::ReachableFromActive {
                start,
                depth,
                min_activity,
            } => {
                let set: FxHashSet<LocusId> =
                    reachable_from_active(world, *start, *depth, *min_activity)
                        .into_iter()
                        .collect();
                candidates.retain(|id| set.contains(id));
            }
            LocusPredicate::DownstreamOfActive {
                start,
                depth,
                min_activity,
            } => {
                let set: FxHashSet<LocusId> =
                    downstream_of_active(world, *start, *depth, *min_activity)
                        .into_iter()
                        .collect();
                candidates.retain(|id| set.contains(id));
            }
            LocusPredicate::UpstreamOfActive {
                start,
                depth,
                min_activity,
            } => {
                let set: FxHashSet<LocusId> =
                    upstream_of_active(world, *start, *depth, *min_activity)
                        .into_iter()
                        .collect();
                candidates.retain(|id| set.contains(id));
            }
            // ── Full-graph traversal predicates (priority 90) ─────────────
            LocusPredicate::ReachableFrom { start, depth } => {
                let reachable: FxHashSet<LocusId> =
                    reachable_from(world, *start, *depth).into_iter().collect();
                candidates.retain(|id| reachable.contains(id));
            }
            LocusPredicate::DownstreamOf { start, depth } => {
                let set: FxHashSet<LocusId> =
                    downstream_of(world, *start, *depth).into_iter().collect();
                candidates.retain(|id| set.contains(id));
            }
            LocusPredicate::UpstreamOf { start, depth } => {
                let set: FxHashSet<LocusId> =
                    upstream_of(world, *start, *depth).into_iter().collect();
                candidates.retain(|id| set.contains(id));
            }
        }
    }

    candidates
        .into_iter()
        .filter_map(|id| {
            world.locus(id).map(|l| LocusSummary {
                id: l.id,
                kind: l.kind,
                state: l.state.as_slice().to_vec(),
            })
        })
        .collect()
}

/// Test whether a single relationship passes one predicate.
///
/// Takes a pre-fetched `&Relationship` reference to avoid re-looking up the
/// same relationship for each predicate in the sorted list.
fn rel_pred_matches(r: &graph_core::Relationship, pred: &RelationshipPredicate) -> bool {
    match pred {
        RelationshipPredicate::OfKind(kind) => r.kind == *kind,
        RelationshipPredicate::From(locus) => {
            matches!(r.endpoints, Endpoints::Directed { from, .. } if from == *locus)
        }
        RelationshipPredicate::To(locus) => {
            matches!(r.endpoints, Endpoints::Directed { to, .. }   if to   == *locus)
        }
        RelationshipPredicate::Touching(locus) => r.endpoints.involves(*locus),
        RelationshipPredicate::ActivityAbove(min) => r.activity() > *min,
        RelationshipPredicate::StrengthAbove(min) => r.strength() > *min,
        RelationshipPredicate::SlotAbove { slot, min } => {
            r.state.as_slice().get(*slot).is_some_and(|&v| v >= *min)
        }
        RelationshipPredicate::CreatedInRange { from, to } => {
            r.created_batch >= *from && r.created_batch <= *to
        }
        RelationshipPredicate::OlderThan {
            current_batch,
            min_batches,
        } => r.age_in_batches(*current_batch) >= *min_batches,
        RelationshipPredicate::MinChangeCount(min) => r.lineage.change_count >= *min,
    }
}

/// Execute a `FindRelationships` query.
///
/// ## Optimisations
///
/// **Seed selection** (planner-driven):
/// - `From(a) + To(b) + OfKind(k)` → `DirectLookup`: single O(1) hash lookup
///   via the `(EndpointKey, kind)` index in `RelationshipStore`.
/// - `From(a) + To(b)` → `Between`: `relationships_between(a, b)`, O(min_degree).
/// - `From(a)` / `To(b)` / `Touching(c)` → adjacency index, O(degree).
/// - No endpoint predicate → full O(edges) scan.
///
/// **Lazy early termination**: when `sort_by` is `None` and `limit` is `Some(n)`,
/// the predicate filter chain is evaluated lazily and collection stops at `n`
/// results — avoiding a full scan of the remaining candidates.
fn find_relationship_summaries(
    world: &World,
    predicates: &[RelationshipPredicate],
    sort_by: Option<&RelSort>,
    limit: Option<usize>,
) -> Vec<RelationshipSummary> {
    use crate::planner::{SeedKind, plan_rel_predicates};

    let plan = plan_rel_predicates(predicates);

    // ── Seed: build the initial candidate ID set ─────────────────────────────
    let candidates: Vec<RelationshipId> = match &plan.seed_locus {
        // O(1) — single hash lookup by (EndpointKey, kind).
        Some(SeedKind::DirectLookup { from, to, kind }) => {
            let key = EndpointKey::Directed(*from, *to);
            world
                .relationships()
                .lookup(&key, *kind)
                .map(|id| vec![id])
                .unwrap_or_default()
        }
        // O(min_degree) — scan edges touching `a`, keep those also touching `b`.
        Some(SeedKind::Between { a, b }) => {
            world.relationships_between(*a, *b).map(|r| r.id).collect()
        }
        Some(SeedKind::From(locus)) => world
            .relationships_for_locus(*locus)
            .filter(|r| matches!(r.endpoints, Endpoints::Directed { from, .. } if from == *locus))
            .map(|r| r.id)
            .collect(),
        Some(SeedKind::To(locus)) => world
            .relationships_for_locus(*locus)
            .filter(|r| matches!(r.endpoints, Endpoints::Directed { to, .. } if to == *locus))
            .map(|r| r.id)
            .collect(),
        Some(SeedKind::Touching(locus)) => world
            .relationships_for_locus(*locus)
            .map(|r| r.id)
            .collect(),
        None => world.relationships().iter().map(|r| r.id).collect(),
    };

    // ── Filter: single-pass over candidates ──────────────────────────────────
    // Fetch each relationship once, apply all predicates, then convert to
    // summary in the same closure — one HashMap lookup per candidate total.
    let preds = &plan.predicates_ordered;
    let filtered = candidates.into_iter().filter_map(|id| {
        world.relationships().get(id).and_then(|r| {
            preds
                .iter()
                .all(|pred| rel_pred_matches(r, pred))
                .then(|| rel_to_summary(r))
        })
    });

    // ── Sort + limit ──────────────────────────────────────────────────────────
    match sort_by {
        None => {
            // No sort → can short-circuit: stop collecting once we reach `limit`.
            match limit {
                Some(n) => filtered.take(n).collect(),
                None => filtered.collect(),
            }
        }
        Some(sort) => {
            // Must collect the full filtered set before sorting.
            let mut summaries: Vec<RelationshipSummary> = filtered.collect();
            match sort {
                RelSort::ActivityDesc => {
                    summaries.sort_unstable_by(|a, b| b.activity.total_cmp(&a.activity))
                }
                RelSort::StrengthDesc => summaries.sort_unstable_by(|a, b| {
                    (b.activity + b.weight).total_cmp(&(a.activity + a.weight))
                }),
                RelSort::WeightDesc => {
                    summaries.sort_unstable_by(|a, b| b.weight.total_cmp(&a.weight))
                }
                RelSort::ChangeCountDesc => {
                    summaries.sort_unstable_by(|a, b| b.change_count.cmp(&a.change_count))
                }
                RelSort::CreatedBatchAsc => {
                    summaries.sort_unstable_by_key(|s| s.created_batch.0);
                }
            }
            if let Some(n) = limit {
                summaries.truncate(n);
            }
            summaries
        }
    }
}

fn find_entities_inner(world: &World, predicates: &[EntityPredicate]) -> Vec<EntityId> {
    let mut candidates: Vec<EntityId> = world.entities().active().map(|e| e.id).collect();
    for pred in predicates {
        match pred {
            EntityPredicate::CoherenceAbove(min) => {
                candidates.retain(|&id| {
                    world
                        .entities()
                        .get(id)
                        .is_some_and(|e| e.current.coherence >= *min)
                });
            }
            EntityPredicate::HasMember(locus) => {
                candidates.retain(|&id| {
                    world
                        .entities()
                        .get(id)
                        .is_some_and(|e| e.current.members.contains(locus))
                });
            }
            EntityPredicate::MinMembers(min) => {
                candidates.retain(|&id| {
                    world
                        .entities()
                        .get(id)
                        .is_some_and(|e| e.current.members.len() >= *min)
                });
            }
        }
    }
    candidates
}

// ─── Fluent builders ─────────────────────────────────────────────────────────

/// Fluent builder for [`Query::FindRelationships`].
///
/// Construct via [`Query::find_relationships()`], chain predicate methods, then
/// call [`.build()`](FindRelationshipsBuilder::build) to obtain a [`Query`] or
/// [`.run(world)`](FindRelationshipsBuilder::run) to execute immediately.
///
/// ```ignore
/// let rows = Query::find_relationships()
///     .of_kind(SUPPLY_KIND)
///     .activity_above(0.3)
///     .sort_by(RelSort::ActivityDesc)
///     .limit(10)
///     .run(&world)
///     .into_relationship_summaries()
///     .unwrap();
/// ```
#[derive(Debug, Clone, Default)]
pub struct FindRelationshipsBuilder {
    predicates: Vec<RelationshipPredicate>,
    sort_by: Option<RelSort>,
    limit: Option<usize>,
}

impl FindRelationshipsBuilder {
    /// Keep only relationships of the given influence kind.
    pub fn of_kind(mut self, kind: InfluenceKindId) -> Self {
        self.predicates.push(RelationshipPredicate::OfKind(kind));
        self
    }

    /// Keep only directed relationships originating from `locus`.
    pub fn from_locus(mut self, locus: LocusId) -> Self {
        self.predicates.push(RelationshipPredicate::From(locus));
        self
    }

    /// Keep only directed relationships terminating at `locus`.
    pub fn to_locus(mut self, locus: LocusId) -> Self {
        self.predicates.push(RelationshipPredicate::To(locus));
        self
    }

    /// Keep only relationships involving `locus` at either endpoint.
    pub fn touching(mut self, locus: LocusId) -> Self {
        self.predicates.push(RelationshipPredicate::Touching(locus));
        self
    }

    /// Keep only relationships whose activity > `min`.
    pub fn activity_above(mut self, min: f32) -> Self {
        self.predicates
            .push(RelationshipPredicate::ActivityAbove(min));
        self
    }

    /// Keep only relationships whose combined strength > `min`.
    pub fn strength_above(mut self, min: f32) -> Self {
        self.predicates
            .push(RelationshipPredicate::StrengthAbove(min));
        self
    }

    /// Keep only relationships where `state[slot] >= min`.
    pub fn slot_above(mut self, slot: usize, min: f32) -> Self {
        self.predicates
            .push(RelationshipPredicate::SlotAbove { slot, min });
        self
    }

    /// Keep only relationships created within `[from, to]` batch range.
    pub fn created_in_range(mut self, from: BatchId, to: BatchId) -> Self {
        self.predicates
            .push(RelationshipPredicate::CreatedInRange { from, to });
        self
    }

    /// Keep only relationships older than `min_batches`.
    pub fn older_than(mut self, current_batch: BatchId, min_batches: u64) -> Self {
        self.predicates.push(RelationshipPredicate::OlderThan {
            current_batch,
            min_batches,
        });
        self
    }

    /// Keep only relationships with change count ≥ `min`.
    pub fn min_change_count(mut self, min: u64) -> Self {
        self.predicates
            .push(RelationshipPredicate::MinChangeCount(min));
        self
    }

    /// Set sort order. Without this, results are returned in index order.
    pub fn sort_by(mut self, sort: RelSort) -> Self {
        self.sort_by = Some(sort);
        self
    }

    /// Return at most `n` results.
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Build the [`Query`] without executing it.
    pub fn build(self) -> Query {
        Query::FindRelationships {
            predicates: self.predicates,
            sort_by: self.sort_by,
            limit: self.limit,
        }
    }

    /// Execute against `world` and return the result directly.
    pub fn run(self, world: &World) -> QueryResult {
        execute(world, &self.build())
    }
}

/// Fluent builder for [`Query::FindLoci`].
///
/// Construct via [`Query::find_loci()`], chain predicate methods, then
/// call [`.build()`](FindLociBuilder::build) or [`.run(world)`](FindLociBuilder::run).
///
/// ```ignore
/// let loci = Query::find_loci()
///     .of_kind(NODE_KIND)
///     .state_above(0, 0.5)
///     .sort_by(LocusSort::StateDesc(0))
///     .limit(5)
///     .run(&world)
///     .into_locus_summaries()
///     .unwrap();
/// ```
#[derive(Debug, Clone, Default)]
pub struct FindLociBuilder {
    predicates: Vec<LocusPredicate>,
    sort_by: Option<LocusSort>,
    limit: Option<usize>,
}

impl FindLociBuilder {
    /// Keep only loci of the given kind.
    pub fn of_kind(mut self, kind: LocusKindId) -> Self {
        self.predicates.push(LocusPredicate::OfKind(kind));
        self
    }

    /// Keep only loci where `state[slot] >= min`.
    pub fn state_above(mut self, slot: usize, min: f32) -> Self {
        self.predicates
            .push(LocusPredicate::StateAbove { slot, min });
        self
    }

    /// Keep only loci where `state[slot] <= max`.
    pub fn state_below(mut self, slot: usize, max: f32) -> Self {
        self.predicates
            .push(LocusPredicate::StateBelow { slot, max });
        self
    }

    /// Keep only loci whose total degree ≥ `min`.
    pub fn min_degree(mut self, min: usize) -> Self {
        self.predicates.push(LocusPredicate::MinDegree(min));
        self
    }

    /// Keep only loci that have a string property `key` equal to `value`.
    pub fn str_property_eq(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.predicates.push(LocusPredicate::StrPropertyEq {
            key: key.into(),
            value: value.into(),
        });
        self
    }

    /// Keep only loci that have a numeric property `key` ≥ `min`.
    pub fn f64_property_above(mut self, key: impl Into<String>, min: f64) -> Self {
        self.predicates.push(LocusPredicate::F64PropertyAbove {
            key: key.into(),
            min,
        });
        self
    }

    /// Keep only loci reachable from `start` within `depth` undirected hops.
    pub fn reachable_from(mut self, start: LocusId, depth: usize) -> Self {
        self.predicates
            .push(LocusPredicate::ReachableFrom { start, depth });
        self
    }

    /// Keep only loci downstream of `start` within `depth` directed hops.
    pub fn downstream_of(mut self, start: LocusId, depth: usize) -> Self {
        self.predicates
            .push(LocusPredicate::DownstreamOf { start, depth });
        self
    }

    /// Keep only loci upstream of `start` within `depth` directed hops.
    pub fn upstream_of(mut self, start: LocusId, depth: usize) -> Self {
        self.predicates
            .push(LocusPredicate::UpstreamOf { start, depth });
        self
    }

    /// Like `reachable_from` but only traverses edges with `activity >= min_activity`.
    pub fn reachable_from_active(
        mut self,
        start: LocusId,
        depth: usize,
        min_activity: f32,
    ) -> Self {
        self.predicates.push(LocusPredicate::ReachableFromActive {
            start,
            depth,
            min_activity,
        });
        self
    }

    /// Like `downstream_of` but only traverses forward edges with `activity >= min_activity`.
    pub fn downstream_of_active(mut self, start: LocusId, depth: usize, min_activity: f32) -> Self {
        self.predicates.push(LocusPredicate::DownstreamOfActive {
            start,
            depth,
            min_activity,
        });
        self
    }

    /// Like `upstream_of` but only traverses backward edges with `activity >= min_activity`.
    pub fn upstream_of_active(mut self, start: LocusId, depth: usize, min_activity: f32) -> Self {
        self.predicates.push(LocusPredicate::UpstreamOfActive {
            start,
            depth,
            min_activity,
        });
        self
    }

    /// Set sort order.
    pub fn sort_by(mut self, sort: LocusSort) -> Self {
        self.sort_by = Some(sort);
        self
    }

    /// Return at most `n` results.
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Build the [`Query`] without executing it.
    pub fn build(self) -> Query {
        Query::FindLoci {
            predicates: self.predicates,
            sort_by: self.sort_by,
            limit: self.limit,
        }
    }

    /// Execute against `world` and return the result directly.
    pub fn run(self, world: &World) -> QueryResult {
        execute(world, &self.build())
    }
}

/// Fluent builder for [`Query::FindEntities`].
///
/// Construct via [`Query::find_entities()`], chain predicate methods, then
/// call [`.build()`](FindEntitiesBuilder::build) or [`.run(world)`](FindEntitiesBuilder::run).
///
/// ```ignore
/// let entities = Query::find_entities()
///     .coherence_above(0.6)
///     .min_members(3)
///     .sort_by(EntitySort::CoherenceDesc)
///     .run(&world)
///     .into_entities()
///     .unwrap();
/// ```
#[derive(Debug, Clone, Default)]
pub struct FindEntitiesBuilder {
    predicates: Vec<EntityPredicate>,
    sort_by: Option<EntitySort>,
    limit: Option<usize>,
}

impl FindEntitiesBuilder {
    /// Keep only entities with coherence ≥ `min`.
    pub fn coherence_above(mut self, min: f32) -> Self {
        self.predicates.push(EntityPredicate::CoherenceAbove(min));
        self
    }

    /// Keep only entities that contain `locus` as a member.
    pub fn has_member(mut self, locus: LocusId) -> Self {
        self.predicates.push(EntityPredicate::HasMember(locus));
        self
    }

    /// Keep only entities with at least `min` members.
    pub fn min_members(mut self, min: usize) -> Self {
        self.predicates.push(EntityPredicate::MinMembers(min));
        self
    }

    /// Set sort order.
    pub fn sort_by(mut self, sort: EntitySort) -> Self {
        self.sort_by = Some(sort);
        self
    }

    /// Return at most `n` results.
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Build the [`Query`] without executing it.
    pub fn build(self) -> Query {
        Query::FindEntities {
            predicates: self.predicates,
            sort_by: self.sort_by,
            limit: self.limit,
        }
    }

    /// Execute against `world` and return the result directly.
    pub fn run(self, world: &World) -> QueryResult {
        execute(world, &self.build())
    }
}

impl Query {
    /// Start building a [`Query::FindRelationships`] with a fluent API.
    ///
    /// See [`FindRelationshipsBuilder`] for available predicate and sort methods.
    pub fn find_relationships() -> FindRelationshipsBuilder {
        FindRelationshipsBuilder::default()
    }

    /// Start building a [`Query::FindLoci`] with a fluent API.
    ///
    /// See [`FindLociBuilder`] for available predicate and sort methods.
    pub fn find_loci() -> FindLociBuilder {
        FindLociBuilder::default()
    }

    /// Start building a [`Query::FindEntities`] with a fluent API.
    ///
    /// See [`FindEntitiesBuilder`] for available predicate and sort methods.
    pub fn find_entities() -> FindEntitiesBuilder {
        FindEntitiesBuilder::default()
    }
}

// ─── QueryResult convenience extractors ──────────────────────────────────────

impl QueryResult {
    /// Extract `Vec<LocusId>` from a [`QueryResult::Loci`] variant.
    /// Returns `None` for any other variant.
    pub fn into_loci(self) -> Option<Vec<LocusId>> {
        match self {
            QueryResult::Loci(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Vec<LocusSummary>` from a [`QueryResult::LocusSummaries`] variant.
    /// Returns `None` for any other variant.
    pub fn into_locus_summaries(self) -> Option<Vec<LocusSummary>> {
        match self {
            QueryResult::LocusSummaries(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Vec<RelationshipSummary>` from a [`QueryResult::RelationshipSummaries`] variant.
    /// Returns `None` for any other variant.
    pub fn into_relationship_summaries(self) -> Option<Vec<RelationshipSummary>> {
        match self {
            QueryResult::RelationshipSummaries(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Vec<RelationshipId>` from a [`QueryResult::Relationships`] variant.
    /// Returns `None` for any other variant.
    pub fn into_relationships(self) -> Option<Vec<RelationshipId>> {
        match self {
            QueryResult::Relationships(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Vec<EntityId>` from a [`QueryResult::Entities`] variant.
    /// Returns `None` for any other variant.
    pub fn into_entities(self) -> Option<Vec<EntityId>> {
        match self {
            QueryResult::Entities(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Vec<ChangeId>` from a [`QueryResult::Changes`] variant.
    /// Returns `None` for any other variant.
    pub fn into_changes(self) -> Option<Vec<ChangeId>> {
        match self {
            QueryResult::Changes(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Vec<Vec<LocusId>>` from a [`QueryResult::Components`] variant.
    /// Returns `None` for any other variant.
    pub fn into_components(self) -> Option<Vec<Vec<LocusId>>> {
        match self {
            QueryResult::Components(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Vec<Vec<LocusId>>` from a [`QueryResult::Communities`] variant.
    /// Returns `None` for any other variant.
    pub fn into_communities(self) -> Option<Vec<Vec<LocusId>>> {
        match self {
            QueryResult::Communities(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Option<Vec<LocusId>>` from a [`QueryResult::Path`] variant.
    /// The inner `Option` is `None` when no path exists.
    /// Returns `None` for any other variant.
    pub fn into_path(self) -> Option<Option<Vec<LocusId>>> {
        match self {
            QueryResult::Path(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Vec<(LocusId, f32)>` from a [`QueryResult::LocusScores`] variant.
    /// Returns `None` for any other variant.
    pub fn into_scores(self) -> Option<Vec<(LocusId, f32)>> {
        match self {
            QueryResult::LocusScores(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `bool` from a [`QueryResult::Bool`] variant.
    /// Returns `None` for any other variant.
    pub fn into_bool(self) -> Option<bool> {
        match self {
            QueryResult::Bool(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `usize` from a [`QueryResult::Count`] variant.
    /// Returns `None` for any other variant.
    pub fn into_count(self) -> Option<usize> {
        match self {
            QueryResult::Count(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `f32` from a [`QueryResult::Score`] variant.
    /// Returns `None` for any other variant.
    pub fn into_score(self) -> Option<f32> {
        match self {
            QueryResult::Score(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Option<f32>` from a [`QueryResult::MaybeScore`] variant.
    /// Returns `None` for any other variant.
    pub fn into_maybe_score(self) -> Option<Option<f32>> {
        match self {
            QueryResult::MaybeScore(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `TrendResult` from a [`QueryResult::Trend`] variant.
    /// Returns `None` for any other variant.
    pub fn into_trend(self) -> Option<TrendResult> {
        match self {
            QueryResult::Trend(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Vec<EntityDiffSummary>` from a [`QueryResult::EntityDeviations`] variant.
    /// Returns `None` for any other variant.
    pub fn into_entity_deviations(self) -> Option<Vec<EntityDiffSummary>> {
        match self {
            QueryResult::EntityDeviations(v) => Some(v),
            _ => None,
        }
    }

    /// Extract `Vec<CohereResult>` from a [`QueryResult::Coheres`] variant.
    /// Returns `None` for any other variant.
    pub fn into_coheres(self) -> Option<Vec<CohereResult>> {
        match self {
            QueryResult::Coheres(v) => Some(v),
            _ => None,
        }
    }

    /// Extract [`RelationshipProfileResult`] from a [`QueryResult::RelationshipProfile`] variant.
    /// Returns `None` for any other variant.
    pub fn into_relationship_profile(self) -> Option<RelationshipProfileResult> {
        match self {
            QueryResult::RelationshipProfile(v) => Some(v),
            _ => None,
        }
    }

    /// Extract [`WorldMetricsResult`] from a [`QueryResult::WorldMetrics`] variant.
    /// Returns `None` for any other variant.
    pub fn into_world_metrics(self) -> Option<WorldMetricsResult> {
        match self {
            QueryResult::WorldMetrics(v) => Some(v),
            _ => None,
        }
    }

    /// Extract feedback pairs from a [`QueryResult::FeedbackPairs`] variant.
    pub fn into_feedback_pairs(self) -> Option<Vec<(LocusId, LocusId, f32)>> {
        match self {
            QueryResult::FeedbackPairs(v) => Some(v),
            _ => None,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId, Relationship,
        RelationshipLineage, StateVector,
    };
    use graph_world::World;

    fn simple_world() -> World {
        let mut w = World::new();
        let rk = InfluenceKindId(1);
        for (id, kind, v) in [(0u64, 1u64, 0.9f32), (1, 1, 0.4), (2, 2, 0.7)] {
            w.insert_locus(Locus::new(
                LocusId(id),
                LocusKindId(kind),
                StateVector::from_slice(&[v]),
            ));
        }
        for (from, to, activity) in [(0u64, 1u64, 0.8f32), (1, 2, 0.3)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::directed(LocusId(from), LocusId(to)),
                state: StateVector::from_slice(&[activity, 0.5]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 3,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    #[test]
    fn path_between_finds_path() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::PathBetween {
                from: LocusId(0),
                to: LocusId(2),
            },
        );
        match result {
            QueryResult::Path(Some(path)) => {
                assert!(path.contains(&LocusId(0)));
                assert!(path.contains(&LocusId(2)));
            }
            _ => panic!("expected Some(path)"),
        }
    }

    #[test]
    fn reachable_from_returns_loci() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::ReachableFrom {
                start: LocusId(0),
                depth: 2,
            },
        );
        match result {
            QueryResult::Loci(ids) => {
                assert!(ids.contains(&LocusId(1)));
                assert!(ids.contains(&LocusId(2)));
            }
            _ => panic!("expected Loci"),
        }
    }

    #[test]
    fn connected_components_returns_components() {
        let w = simple_world();
        let result = execute(&w, &Query::ConnectedComponents);
        match result {
            QueryResult::Components(comps) => {
                assert_eq!(comps.len(), 1);
                assert_eq!(comps[0].len(), 3);
            }
            _ => panic!("expected Components"),
        }
    }

    // ── FindLoci ─────────────────────────────────────────────────────────────

    #[test]
    fn find_loci_returns_summaries_with_state() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::FindLoci {
                predicates: vec![LocusPredicate::OfKind(LocusKindId(1))],
                sort_by: None,
                limit: None,
            },
        );
        match result {
            QueryResult::LocusSummaries(rows) => {
                assert_eq!(rows.len(), 2);
                // state vector included — no second lookup needed
                for row in &rows {
                    assert!(!row.state.is_empty());
                }
            }
            _ => panic!("expected LocusSummaries"),
        }
    }

    #[test]
    fn find_loci_sort_state_desc() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::FindLoci {
                predicates: vec![],
                sort_by: Some(LocusSort::StateDesc(0)),
                limit: Some(2),
            },
        );
        match result {
            QueryResult::LocusSummaries(rows) => {
                assert_eq!(rows.len(), 2);
                // L0(0.9) > L2(0.7) after top-2 limit
                assert_eq!(rows[0].id, LocusId(0));
                assert_eq!(rows[1].id, LocusId(2));
            }
            _ => panic!("expected LocusSummaries"),
        }
    }

    // ── FindRelationships ────────────────────────────────────────────────────

    #[test]
    fn find_relationships_returns_summaries() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::FindRelationships {
                predicates: vec![RelationshipPredicate::ActivityAbove(0.5)],
                sort_by: None,
                limit: None,
            },
        );
        match result {
            QueryResult::RelationshipSummaries(rows) => {
                assert_eq!(rows.len(), 1);
                // full data in result — no second lookup needed
                assert_eq!(rows[0].from, LocusId(0));
                assert_eq!(rows[0].to, LocusId(1));
                assert!(rows[0].activity > 0.5);
                assert!(rows[0].directed);
            }
            _ => panic!("expected RelationshipSummaries"),
        }
    }

    #[test]
    fn find_relationships_sort_activity_desc_with_limit() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::FindRelationships {
                predicates: vec![],
                sort_by: Some(RelSort::ActivityDesc),
                limit: Some(1),
            },
        );
        match result {
            QueryResult::RelationshipSummaries(rows) => {
                assert_eq!(rows.len(), 1);
                // top-1 by activity = L0→L1 (0.8)
                assert_eq!(rows[0].from, LocusId(0));
                assert!((rows[0].activity - 0.8).abs() < 1e-5);
            }
            _ => panic!("expected RelationshipSummaries"),
        }
    }

    #[test]
    fn find_relationships_compound_predicate() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::FindRelationships {
                predicates: vec![
                    RelationshipPredicate::OfKind(InfluenceKindId(1)),
                    RelationshipPredicate::ActivityAbove(0.5),
                ],
                sort_by: Some(RelSort::ActivityDesc),
                limit: None,
            },
        );
        match result {
            QueryResult::RelationshipSummaries(rows) => {
                assert_eq!(rows.len(), 1);
            }
            _ => panic!(),
        }
    }

    // ── LocusStateSlot ───────────────────────────────────────────────────────

    #[test]
    fn locus_state_slot_returns_value() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::LocusStateSlot {
                locus: LocusId(0),
                slot: 0,
            },
        );
        match result {
            QueryResult::MaybeScore(Some(v)) => assert!((v - 0.9).abs() < 1e-5),
            _ => panic!("expected MaybeScore(Some)"),
        }
    }

    #[test]
    fn locus_state_slot_missing_returns_none() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::LocusStateSlot {
                locus: LocusId(99),
                slot: 0,
            },
        );
        assert_eq!(result, QueryResult::MaybeScore(None));
    }

    // ── RelationshipProfile ──────────────────────────────────────────────────

    #[test]
    fn relationship_profile_includes_dominant_kind() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::RelationshipProfile {
                from: LocusId(0),
                to: LocusId(1),
            },
        );
        match result {
            QueryResult::RelationshipProfile(p) => {
                assert_eq!(p.relationship_ids.len(), 1);
                // dominant_kind now present
                assert_eq!(p.dominant_kind, Some(InfluenceKindId(1)));
                // activity_by_kind now present
                assert_eq!(p.activity_by_kind.len(), 1);
                assert_eq!(p.activity_by_kind[0].0, InfluenceKindId(1));
            }
            _ => panic!("expected RelationshipProfile"),
        }
    }

    // ── AllBetweenness with limit ────────────────────────────────────────────

    #[test]
    fn all_betweenness_with_limit() {
        let w = simple_world();
        let result = execute(&w, &Query::AllBetweenness { limit: Some(1) });
        match result {
            QueryResult::LocusScores(scores) => {
                assert_eq!(scores.len(), 1);
                // scores are sorted descending — first is highest betweenness
                assert!(scores[0].1 >= 0.0);
            }
            _ => panic!("expected LocusScores"),
        }
    }

    #[test]
    fn all_betweenness_no_limit_returns_all() {
        let w = simple_world();
        let result = execute(&w, &Query::AllBetweenness { limit: None });
        match result {
            QueryResult::LocusScores(scores) => {
                assert_eq!(scores.len(), 3);
                // verify descending order
                for w in scores.windows(2) {
                    assert!(w[0].1 >= w[1].1);
                }
            }
            _ => panic!("expected LocusScores"),
        }
    }

    // ── PageRank with limit ──────────────────────────────────────────────────

    #[test]
    fn pagerank_with_limit_returns_top_n() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::PageRank {
                damping: 0.85,
                iterations: 20,
                tolerance: 1e-4,
                limit: Some(2),
            },
        );
        match result {
            QueryResult::LocusScores(scores) => {
                assert_eq!(scores.len(), 2);
                assert!(scores[0].1 >= scores[1].1);
            }
            _ => panic!("expected LocusScores"),
        }
    }

    // ── Betweenness single locus ─────────────────────────────────────────────

    #[test]
    fn betweenness_for_middle_locus() {
        let w = simple_world();
        let result = execute(&w, &Query::BetweennessFor(LocusId(1)));
        match result {
            QueryResult::Score(v) => assert!(v >= 0.0),
            _ => panic!("expected Score"),
        }
    }

    // ── HasCycle ─────────────────────────────────────────────────────────────

    #[test]
    fn has_cycle_false_for_dag() {
        let w = simple_world();
        let result = execute(&w, &Query::HasCycle);
        match result {
            QueryResult::Bool(v) => assert!(!v),
            _ => panic!("expected Bool"),
        }
    }

    // ── WorldMetrics ─────────────────────────────────────────────────────────

    #[test]
    fn world_metrics_returns_correct_counts() {
        let w = simple_world();
        let result = execute(&w, &Query::WorldMetrics);
        match result {
            QueryResult::WorldMetrics(m) => {
                assert_eq!(m.locus_count, 3);
                assert_eq!(m.relationship_count, 2);
            }
            _ => panic!("expected WorldMetrics"),
        }
    }

    // ── Fluent builder tests ──────────────────────────────────────────────────

    #[test]
    fn builder_find_relationships_equals_enum() {
        let via_builder = Query::find_relationships()
            .of_kind(InfluenceKindId(1))
            .activity_above(0.5)
            .sort_by(RelSort::ActivityDesc)
            .limit(10)
            .build();
        let via_enum = Query::FindRelationships {
            predicates: vec![
                RelationshipPredicate::OfKind(InfluenceKindId(1)),
                RelationshipPredicate::ActivityAbove(0.5),
            ],
            sort_by: Some(RelSort::ActivityDesc),
            limit: Some(10),
        };
        assert_eq!(via_builder, via_enum);
    }

    #[test]
    fn builder_find_relationships_run_returns_summaries() {
        let w = simple_world();
        let rows = Query::find_relationships()
            .activity_above(0.5)
            .run(&w)
            .into_relationship_summaries()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].from, LocusId(0));
    }

    #[test]
    fn builder_find_loci_equals_enum() {
        let via_builder = Query::find_loci()
            .of_kind(LocusKindId(1))
            .state_above(0, 0.5)
            .sort_by(LocusSort::StateDesc(0))
            .limit(5)
            .build();
        let via_enum = Query::FindLoci {
            predicates: vec![
                LocusPredicate::OfKind(LocusKindId(1)),
                LocusPredicate::StateAbove { slot: 0, min: 0.5 },
            ],
            sort_by: Some(LocusSort::StateDesc(0)),
            limit: Some(5),
        };
        assert_eq!(via_builder, via_enum);
    }

    #[test]
    fn builder_find_loci_run_returns_summaries() {
        let w = simple_world();
        let summaries = Query::find_loci()
            .of_kind(LocusKindId(1))
            .run(&w)
            .into_locus_summaries()
            .unwrap();
        assert_eq!(summaries.len(), 2);
    }

    #[test]
    fn builder_find_entities_run_returns_entities() {
        let w = simple_world();
        // No entities in simple_world — just verify the builder wires through correctly.
        let ids = Query::find_entities().run(&w).into_entities().unwrap();
        assert!(ids.is_empty());
    }

    // ── QueryResult extractors ────────────────────────────────────────────────

    #[test]
    fn into_loci_extracts_correct_variant() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::ReachableFrom {
                start: LocusId(0),
                depth: 2,
            },
        );
        assert!(result.into_loci().is_some());
    }

    #[test]
    fn into_loci_returns_none_for_wrong_variant() {
        let w = simple_world();
        let result = execute(&w, &Query::HasCycle);
        assert!(result.into_loci().is_none());
    }

    #[test]
    fn into_bool_extracts_correct_variant() {
        let w = simple_world();
        let result = execute(&w, &Query::HasCycle);
        assert_eq!(result.into_bool(), Some(false));
    }

    #[test]
    fn into_path_extracts_correct_variant() {
        let w = simple_world();
        let result = execute(
            &w,
            &Query::PathBetween {
                from: LocusId(0),
                to: LocusId(2),
            },
        );
        let path = result.into_path().unwrap();
        assert!(path.is_some());
    }

    #[test]
    fn into_world_metrics_extracts_correct_variant() {
        let w = simple_world();
        let m = execute(&w, &Query::WorldMetrics)
            .into_world_metrics()
            .unwrap();
        assert_eq!(m.locus_count, 3);
    }

    // ── Serde round-trip ─────────────────────────────────────────────────────

    // ── Causal strength Query variants ──────────────────────────────────────

    fn world_with_stdp_weights() -> World {
        use graph_core::{Endpoints, StateVector};
        let mut w = World::new();
        // A→B weight=0.8, B→A weight=0.2 — A strongly causes B
        w.add_relationship(
            Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            InfluenceKindId(1),
            StateVector::from_slice(&[0.0, 0.8]),
        );
        w.add_relationship(
            Endpoints::Directed {
                from: LocusId(1),
                to: LocusId(0),
            },
            InfluenceKindId(1),
            StateVector::from_slice(&[0.0, 0.2]),
        );
        w
    }

    #[test]
    fn query_causal_direction_forward() {
        let w = world_with_stdp_weights();
        let score = execute(
            &w,
            &Query::CausalDirection {
                from: LocusId(0),
                to: LocusId(1),
                kind: InfluenceKindId(1),
            },
        )
        .into_score()
        .unwrap();
        assert!(score > 0.5, "expected A→B direction, got {score}");
    }

    #[test]
    fn query_dominant_causes_returns_locus_scores() {
        let w = world_with_stdp_weights();
        let scores = execute(
            &w,
            &Query::DominantCauses {
                target: LocusId(1),
                kind: InfluenceKindId(1),
                n: 5,
            },
        )
        .into_scores()
        .unwrap();
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0].0, LocusId(0));
    }

    #[test]
    fn query_causal_in_out_strength() {
        let w = world_with_stdp_weights();
        let in_s = execute(
            &w,
            &Query::CausalInStrength {
                locus: LocusId(1),
                kind: InfluenceKindId(1),
            },
        )
        .into_score()
        .unwrap();
        let out_s = execute(
            &w,
            &Query::CausalOutStrength {
                locus: LocusId(0),
                kind: InfluenceKindId(1),
            },
        )
        .into_score()
        .unwrap();
        assert!((in_s - 0.8).abs() < 1e-5, "in_strength={in_s}");
        assert!((out_s - 0.8).abs() < 1e-5, "out_strength={out_s}");
    }

    #[test]
    fn query_feedback_pairs_detects_loop() {
        use graph_core::{Endpoints, StateVector};
        let mut w = World::new();
        // A↔B roughly balanced
        w.add_relationship(
            Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            InfluenceKindId(1),
            StateVector::from_slice(&[0.0, 0.8]),
        );
        w.add_relationship(
            Endpoints::Directed {
                from: LocusId(1),
                to: LocusId(0),
            },
            InfluenceKindId(1),
            StateVector::from_slice(&[0.0, 0.7]),
        );
        let pairs = execute(
            &w,
            &Query::FeedbackPairs {
                kind: InfluenceKindId(1),
                min_weight: 0.1,
                min_balance: 0.5,
            },
        )
        .into_feedback_pairs()
        .unwrap();
        assert_eq!(pairs.len(), 1);
        let (_, _, balance) = pairs[0];
        assert!(balance >= 0.5 && balance <= 1.0);
    }

    #[test]
    fn explain_causal_direction_is_scan() {
        use crate::api::explain;
        let w = world_with_stdp_weights();
        let plan = explain(
            &w,
            &Query::CausalDirection {
                from: LocusId(0),
                to: LocusId(1),
                kind: InfluenceKindId(1),
            },
        );
        use crate::api::CostClass;
        assert!(plan.steps.iter().any(|s| s.cost_class == CostClass::Scan));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn query_round_trips_through_json() {
        let q = Query::FindLoci {
            predicates: vec![
                LocusPredicate::OfKind(LocusKindId(1)),
                LocusPredicate::StateAbove { slot: 0, min: 0.5 },
            ],
            sort_by: Some(LocusSort::StateDesc(0)),
            limit: Some(5),
        };
        let json = serde_json::to_string(&q).unwrap();
        let q2: Query = serde_json::from_str(&json).unwrap();
        assert_eq!(q, q2);
    }
}
