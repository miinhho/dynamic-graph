use graph_core::{
    BatchId, ChangeId, CohereId, EntityId, InfluenceKindId, LocusId, LocusKindId, RelationshipId,
};

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

/// Owned summary of a single relationship.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RelationshipSummary {
    pub id: RelationshipId,
    pub kind: InfluenceKindId,
    pub from: LocusId,
    pub to: LocusId,
    pub directed: bool,
    pub activity: f32,
    pub weight: f32,
    pub change_count: u64,
    pub created_batch: BatchId,
}

/// Owned summary of a single locus.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LocusSummary {
    pub id: LocusId,
    pub kind: LocusKindId,
    pub state: Vec<f32>,
}

/// Owned snapshot of an entity's deviation since a baseline batch.
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
    Rising { slope: f32 },
    Falling { slope: f32 },
    Stable,
    Insufficient,
}

/// A serializable query that can be executed against a [`graph_world::World`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Query {
    PathBetween {
        from: LocusId,
        to: LocusId,
    },
    PathBetweenOfKind {
        from: LocusId,
        to: LocusId,
        kind: InfluenceKindId,
    },
    DirectedPath {
        from: LocusId,
        to: LocusId,
    },
    ReachableFrom {
        start: LocusId,
        depth: usize,
    },
    DownstreamOf {
        start: LocusId,
        depth: usize,
    },
    UpstreamOf {
        start: LocusId,
        depth: usize,
    },
    ConnectedComponents,
    ConnectedComponentsOfKind(InfluenceKindId),
    ReachableFromActive {
        start: LocusId,
        depth: usize,
        min_activity: f32,
    },
    DownstreamOfActive {
        start: LocusId,
        depth: usize,
        min_activity: f32,
    },
    UpstreamOfActive {
        start: LocusId,
        depth: usize,
        min_activity: f32,
    },
    PathBetweenActive {
        from: LocusId,
        to: LocusId,
        min_activity: f32,
    },
    NeighborsOf(LocusId),
    IsolatedLoci,
    HubLoci(usize),
    ReciprocOf(RelationshipId),
    ReciprocPairs,
    HasCycle,
    StrongestPath {
        from: LocusId,
        to: LocusId,
    },
    PageRank {
        damping: f32,
        iterations: usize,
        tolerance: f32,
        limit: Option<usize>,
    },
    PageRankFor {
        locus: LocusId,
        damping: f32,
        iterations: usize,
        tolerance: f32,
    },
    AllBetweenness {
        limit: Option<usize>,
    },
    BetweennessFor(LocusId),
    AllCloseness {
        limit: Option<usize>,
    },
    ClosenessFor(LocusId),
    AllConstraints {
        limit: Option<usize>,
    },
    ConstraintFor(LocusId),
    Louvain,
    LouvainWithResolution(f32),
    Modularity,
    CausalAncestors(ChangeId),
    CausalDescendants(ChangeId),
    CausalDepth(ChangeId),
    IsAncestorOf {
        ancestor: ChangeId,
        descendant: ChangeId,
    },
    RootStimuli(ChangeId),
    ChangesToLocusInRange {
        locus: LocusId,
        from: BatchId,
        to: BatchId,
    },
    ChangesToRelationshipInRange {
        relationship: RelationshipId,
        from: BatchId,
        to: BatchId,
    },
    LociChangedInBatch(BatchId),
    RelationshipsChangedInBatch(BatchId),
    FindLoci {
        predicates: Vec<LocusPredicate>,
        sort_by: Option<LocusSort>,
        limit: Option<usize>,
    },
    FindRelationships {
        predicates: Vec<RelationshipPredicate>,
        sort_by: Option<RelSort>,
        limit: Option<usize>,
    },
    FindEntities {
        predicates: Vec<EntityPredicate>,
        sort_by: Option<EntitySort>,
        limit: Option<usize>,
    },
    LocusStateSlot {
        locus: LocusId,
        slot: usize,
    },
    RelationshipProfile {
        from: LocusId,
        to: LocusId,
    },
    ActivityTrend {
        relationship: RelationshipId,
        from_batch: BatchId,
        to_batch: BatchId,
    },
    EntityDeviationsSince(BatchId),
    RelationshipsAbsentWithout(Vec<ChangeId>),
    Coheres,
    CoheresNamed(String),
    WorldMetrics,
    CausalDirection {
        from: LocusId,
        to: LocusId,
        kind: InfluenceKindId,
    },
    DominantCauses {
        target: LocusId,
        kind: InfluenceKindId,
        n: usize,
    },
    DominantEffects {
        source: LocusId,
        kind: InfluenceKindId,
        n: usize,
    },
    CausalInStrength {
        locus: LocusId,
        kind: InfluenceKindId,
    },
    CausalOutStrength {
        locus: LocusId,
        kind: InfluenceKindId,
    },
    FeedbackPairs {
        kind: InfluenceKindId,
        min_weight: f32,
        min_balance: f32,
    },
    GrangerScore {
        from: LocusId,
        to: LocusId,
        kind: InfluenceKindId,
        lag_batches: u64,
    },
    GrangerDominantCauses {
        target: LocusId,
        kind: InfluenceKindId,
        lag_batches: u64,
        n: usize,
    },
    GrangerDominantEffects {
        source: LocusId,
        kind: InfluenceKindId,
        lag_batches: u64,
        n: usize,
    },
    TimeTravel {
        target_batch: BatchId,
    },
    CounterfactualReplay {
        remove_changes: Vec<ChangeId>,
    },
    EntityTransitionCause {
        entity_id: EntityId,
        at_batch: BatchId,
    },
    EntityUpstreamTransitions {
        entity_id: EntityId,
        at_batch: BatchId,
    },
    EntityLayersInRange {
        entity_id: EntityId,
        from: BatchId,
        to: BatchId,
    },
}

/// The owned result of executing a [`Query`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum QueryResult {
    Path(Option<Vec<LocusId>>),
    Loci(Vec<LocusId>),
    Components(Vec<Vec<LocusId>>),
    Changes(Vec<ChangeId>),
    Relationships(Vec<RelationshipId>),
    RelationshipSummaries(Vec<RelationshipSummary>),
    LocusSummaries(Vec<LocusSummary>),
    Entities(Vec<EntityId>),
    Bool(bool),
    Count(usize),
    Score(f32),
    MaybeScore(Option<f32>),
    LocusScores(Vec<(LocusId, f32)>),
    Communities(Vec<Vec<LocusId>>),
    Trend(TrendResult),
    EntityDeviations(Vec<EntityDiffSummary>),
    Coheres(Vec<CohereResult>),
    RelationshipProfile(RelationshipProfileResult),
    WorldMetrics(WorldMetricsResult),
    FeedbackPairs(Vec<(LocusId, LocusId, f32)>),
    TimeTravelResult(Box<crate::TimeTravelResult>),
    Counterfactual(crate::CounterfactualDiff),
    EntityCause(Option<graph_core::LifecycleCause>),
    EntityTransitions(Vec<(EntityId, BatchId)>),
    EntityLayers(
        Vec<(
            BatchId,
            graph_core::LayerTransition,
            graph_core::LifecycleCause,
        )>,
    ),
}

/// Owned snapshot of key relationship profile fields.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RelationshipProfileResult {
    pub from: LocusId,
    pub to: LocusId,
    pub relationship_ids: Vec<RelationshipId>,
    pub total_activity: f32,
    pub net_influence: f32,
    pub dominant_kind: Option<InfluenceKindId>,
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
