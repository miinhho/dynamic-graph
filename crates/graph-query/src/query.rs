//! Fluent builder API for composable graph queries.
//!
//! Entry points:
//! - [`loci`] — query over loci with chainable filters and sorts.
//! - [`relationships`] — query over relationships with chainable filters.
//!
//! Both builders hold `&World` and an internal `Vec` of current candidates.
//! Methods narrow or reorder the set; terminal methods (`collect`, `ids`,
//! `count`, `first`) consume the builder and return the result.
//!
//! ## Examples
//!
//! ```ignore
//! use graph_query::{loci, relationships};
//!
//! // Top 5 most-convinced loci that are also reachable from INFLUENCER_A
//! let convinced_reach = loci(&world)
//!     .reachable_from(INFLUENCER_A, 3)
//!     .where_state(BELIEF_SLOT, |b| b > 0.5)
//!     .top_n_by_state(BELIEF_SLOT, 5)
//!     .collect();
//!
//! // All outgoing edges from an influencer stronger than 1.5
//! let strong_out = relationships(&world)
//!     .from(INFLUENCER_A)
//!     .above_strength(1.5)
//!     .top_n_by_strength(3)
//!     .collect();
//! ```

use graph_core::{
    BatchId, Endpoints, InfluenceKindId, Locus, LocusId, LocusKindId,
    Relationship, RelationshipId,
};
use graph_world::World;
use rustc_hash::FxHashSet;

// ─── LociQuery ────────────────────────────────────────────────────────────────

/// A composable query over loci.
///
/// Constructed via [`loci`] or [`loci_from_ids`].  Each filter method returns
/// `Self`, allowing chains like:
///
/// ```ignore
/// loci(&world)
///     .of_kind(KIND_ORG)
///     .where_state(0, |v| v > 0.8)
///     .top_n_by_state(0, 10)
///     .ids()
/// ```
pub struct LociQuery<'w> {
    world: &'w World,
    loci: Vec<&'w Locus>,
}

/// Start a query over **all** loci in `world`.
pub fn loci(world: &World) -> LociQuery<'_> {
    LociQuery {
        world,
        loci: world.loci().iter().collect(),
    }
}

/// Start a query over a **pre-selected** set of locus IDs.
///
/// Useful for seeding a query from a traversal result:
///
/// ```ignore
/// let ids = graph_query::reachable_from(&world, start, 2);
/// loci_from_ids(&world, &ids).where_state(0, |v| v > 0.5).collect()
/// ```
///
/// IDs that no longer exist in the world are silently skipped.
pub fn loci_from_ids<'w>(world: &'w World, ids: &[LocusId]) -> LociQuery<'w> {
    LociQuery {
        world,
        loci: ids.iter().filter_map(|&id| world.locus(id)).collect(),
    }
}

impl<'w> LociQuery<'w> {
    /// Create a query from a pre-built candidate list (used by cross-layer navigation).
    pub(crate) fn from_candidates(world: &'w World, candidates: Vec<&'w Locus>) -> Self {
        Self { world, loci: candidates }
    }

    // ── Filters ───────────────────────────────────────────────────────────────

    /// Keep only loci of the given kind.
    pub fn of_kind(mut self, kind: LocusKindId) -> Self {
        self.loci.retain(|l| l.kind == kind);
        self
    }

    /// Keep only loci where `state[slot]` satisfies `pred`.
    ///
    /// Loci whose state vector is shorter than `slot + 1` are excluded.
    pub fn where_state(mut self, slot: usize, pred: impl Fn(f32) -> bool) -> Self {
        self.loci
            .retain(|l| l.state.as_slice().get(slot).is_some_and(|&v| pred(v)));
        self
    }

    /// Keep only loci that have string property `key` satisfying `pred`.
    pub fn where_str_property(mut self, key: &str, pred: impl Fn(&str) -> bool) -> Self {
        self.loci.retain(|l| {
            self.world
                .properties()
                .get(l.id)
                .and_then(|p| p.get_str(key))
                .is_some_and(&pred)
        });
        self
    }

    /// Keep only loci that have numeric property `key` satisfying `pred`.
    pub fn where_f64_property(mut self, key: &str, pred: impl Fn(f64) -> bool) -> Self {
        self.loci.retain(|l| {
            self.world
                .properties()
                .get(l.id)
                .and_then(|p| p.get_f64(key))
                .is_some_and(&pred)
        });
        self
    }

    /// Keep only loci matching a custom predicate.
    pub fn matching(mut self, pred: impl Fn(&Locus) -> bool) -> Self {
        self.loci.retain(|l| pred(l));
        self
    }

    /// Keep only loci with total degree ≥ `min`.
    pub fn min_degree(mut self, min: usize) -> Self {
        self.loci.retain(|l| self.world.degree(l.id) >= min);
        self
    }

    /// Keep only loci that are reachable from `start` within `depth`
    /// undirected hops (excluding `start` itself).
    ///
    /// Intersects the current candidate set with the reachable set — O(V+E)
    /// BFS plus O(candidates) filter.
    pub fn reachable_from(self, start: LocusId, depth: usize) -> Self {
        let reachable: FxHashSet<LocusId> =
            crate::traversal::reachable_from(self.world, start, depth)
                .into_iter()
                .collect();
        Self {
            world: self.world,
            loci: self.loci.into_iter().filter(|l| reachable.contains(&l.id)).collect(),
        }
    }

    /// Keep only loci reachable by following edges **forward** from `start`
    /// within `depth` directed hops.
    pub fn downstream_of(self, start: LocusId, depth: usize) -> Self {
        let set: FxHashSet<LocusId> =
            crate::traversal::downstream_of(self.world, start, depth)
                .into_iter()
                .collect();
        Self {
            world: self.world,
            loci: self.loci.into_iter().filter(|l| set.contains(&l.id)).collect(),
        }
    }

    /// Keep only loci reachable by following edges **backward** from `start`
    /// within `depth` directed hops.
    pub fn upstream_of(self, start: LocusId, depth: usize) -> Self {
        let set: FxHashSet<LocusId> =
            crate::traversal::upstream_of(self.world, start, depth)
                .into_iter()
                .collect();
        Self {
            world: self.world,
            loci: self.loci.into_iter().filter(|l| set.contains(&l.id)).collect(),
        }
    }

    // ── Ordering ──────────────────────────────────────────────────────────────

    /// Keep the top `n` loci by `state[slot]` in descending order.
    ///
    /// Loci missing `slot` sort last. Equivalent to sorting then truncating.
    pub fn top_n_by_state(mut self, slot: usize, n: usize) -> Self {
        self.loci.sort_unstable_by(|a, b| {
            let va = a.state.as_slice().get(slot).copied().unwrap_or(f32::NEG_INFINITY);
            let vb = b.state.as_slice().get(slot).copied().unwrap_or(f32::NEG_INFINITY);
            vb.total_cmp(&va)
        });
        self.loci.truncate(n);
        self
    }

    /// Sort the current set by `state[slot]` descending (no truncation).
    pub fn sort_by_state(mut self, slot: usize) -> Self {
        self.loci.sort_unstable_by(|a, b| {
            let va = a.state.as_slice().get(slot).copied().unwrap_or(f32::NEG_INFINITY);
            let vb = b.state.as_slice().get(slot).copied().unwrap_or(f32::NEG_INFINITY);
            vb.total_cmp(&va)
        });
        self
    }

    /// Keep the top `n` loci by total degree (most-connected first).
    pub fn top_n_by_degree(mut self, n: usize) -> Self {
        self.loci
            .sort_unstable_by_key(|l| std::cmp::Reverse(self.world.degree(l.id)));
        self.loci.truncate(n);
        self
    }

    // ── Terminal methods ──────────────────────────────────────────────────────

    /// Collect matching loci as `&Locus` references.
    pub fn collect(self) -> Vec<&'w Locus> {
        self.loci
    }

    /// Collect just the `LocusId`s.
    pub fn ids(self) -> Vec<LocusId> {
        self.loci.into_iter().map(|l| l.id).collect()
    }

    /// Number of matching loci.
    pub fn count(self) -> usize {
        self.loci.len()
    }

    /// First matching locus (in current order), or `None` if the set is empty.
    pub fn first(self) -> Option<&'w Locus> {
        self.loci.into_iter().next()
    }

    /// `true` when no loci match the current constraints.
    pub fn is_empty(&self) -> bool {
        self.loci.is_empty()
    }

    // ── Aggregation terminals ─────────────────────────────────────────────────

    /// Sum of `state[slot]` across all matching loci.
    ///
    /// Loci whose state vector is shorter than `slot + 1` contribute `0.0`.
    pub fn sum_state_slot(self, slot: usize) -> f32 {
        self.loci
            .iter()
            .map(|l| l.state.as_slice().get(slot).copied().unwrap_or(0.0))
            .sum()
    }

    /// Mean of `state[slot]` across all matching loci that have the slot,
    /// or `None` when no loci have that slot (or the set is empty).
    pub fn mean_state_slot(self, slot: usize) -> Option<f32> {
        let values: Vec<f32> = self
            .loci
            .iter()
            .filter_map(|l| l.state.as_slice().get(slot).copied())
            .collect();
        let n = values.len();
        if n == 0 {
            return None;
        }
        Some(values.iter().sum::<f32>() / n as f32)
    }

    /// Maximum of `state[slot]` across all matching loci that have the slot,
    /// or `None` when no loci have that slot (or the set is empty).
    pub fn max_state_slot(self, slot: usize) -> Option<f32> {
        self.loci
            .iter()
            .filter_map(|l| l.state.as_slice().get(slot).copied())
            .reduce(f32::max)
    }

    // ── Cross-builder navigation ──────────────────────────────────────────────

    /// Pivot to a [`RelationshipsQuery`] containing all directed relationships
    /// **originating** from any locus in the current set.
    ///
    /// Symmetric edges are excluded (they have no single source).
    pub fn outgoing_relationships(self) -> RelationshipsQuery<'w> {
        let ids: FxHashSet<LocusId> = self.loci.iter().map(|l| l.id).collect();
        RelationshipsQuery {
            world: self.world,
            rels: self
                .world
                .relationships()
                .iter()
                .filter(|r| {
                    matches!(r.endpoints, Endpoints::Directed { from, .. } if ids.contains(&from))
                })
                .collect(),
        }
    }

    /// Pivot to a [`RelationshipsQuery`] containing all directed relationships
    /// **terminating** at any locus in the current set.
    ///
    /// Symmetric edges are excluded.
    pub fn incoming_relationships(self) -> RelationshipsQuery<'w> {
        let ids: FxHashSet<LocusId> = self.loci.iter().map(|l| l.id).collect();
        RelationshipsQuery {
            world: self.world,
            rels: self
                .world
                .relationships()
                .iter()
                .filter(|r| {
                    matches!(r.endpoints, Endpoints::Directed { to, .. } if ids.contains(&to))
                })
                .collect(),
        }
    }

    /// Pivot to a [`RelationshipsQuery`] containing all relationships that
    /// touch any locus in the current set at either endpoint (directed or
    /// symmetric).
    pub fn touching_relationships(self) -> RelationshipsQuery<'w> {
        let ids: FxHashSet<LocusId> = self.loci.iter().map(|l| l.id).collect();
        RelationshipsQuery {
            world: self.world,
            rels: self
                .world
                .relationships()
                .iter()
                .filter(|r| ids.iter().any(|&id| r.endpoints.involves(id)))
                .collect(),
        }
    }
}

// ─── RelationshipsQuery ───────────────────────────────────────────────────────

/// Aggregate statistics for the activity slot across a `RelationshipsQuery` result set.
///
/// Produced by [`RelationshipsQuery::activity_stats`].
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityStats {
    /// Number of relationships in the set.
    pub count: usize,
    /// Sum of `activity()` values.
    pub sum: f32,
    /// Mean activity (`sum / count`).
    pub mean: f32,
    /// Minimum activity value in the set.
    pub min: f32,
    /// Maximum activity value in the set.
    pub max: f32,
}

/// A composable query over relationships.
///
/// Constructed via [`relationships`] or [`relationships_from_ids`].
///
/// ```ignore
/// relationships(&world)
///     .from(INFLUENCER_A)
///     .above_strength(1.0)
///     .top_n_by_strength(3)
///     .collect()
/// ```
pub struct RelationshipsQuery<'w> {
    world: &'w World,
    rels: Vec<&'w Relationship>,
}

/// Start a query over **all** relationships in `world`.
pub fn relationships(world: &World) -> RelationshipsQuery<'_> {
    RelationshipsQuery {
        world,
        rels: world.relationships().iter().collect(),
    }
}

/// Start a query over a **pre-selected** set of relationship IDs.
///
/// IDs that no longer exist (deleted or cold-evicted) are silently skipped.
pub fn relationships_from_ids<'w>(
    world: &'w World,
    ids: &[RelationshipId],
) -> RelationshipsQuery<'w> {
    RelationshipsQuery {
        world,
        rels: ids
            .iter()
            .filter_map(|&id| world.relationships().get(id))
            .collect(),
    }
}

impl<'w> RelationshipsQuery<'w> {
    /// Create a query from a pre-built candidate list (used by cross-layer navigation).
    pub(crate) fn from_candidates(world: &'w World, candidates: Vec<&'w Relationship>) -> Self {
        Self { world, rels: candidates }
    }

    // ── Filters ───────────────────────────────────────────────────────────────

    /// Keep only relationships of the given influence kind.
    pub fn of_kind(mut self, kind: InfluenceKindId) -> Self {
        self.rels.retain(|r| r.kind == kind);
        self
    }

    /// Keep only directed relationships whose source is `locus`.
    ///
    /// Symmetric edges are excluded (they have no single source).
    pub fn from(mut self, locus: LocusId) -> Self {
        self.rels.retain(|r| matches!(r.endpoints, Endpoints::Directed { from, .. } if from == locus));
        self
    }

    /// Keep only directed relationships whose target is `locus`.
    ///
    /// Symmetric edges are excluded.
    pub fn to(mut self, locus: LocusId) -> Self {
        self.rels.retain(|r| matches!(r.endpoints, Endpoints::Directed { to, .. } if to == locus));
        self
    }

    /// Keep only relationships that involve `locus` at either endpoint
    /// (directed or symmetric).
    pub fn touching(mut self, locus: LocusId) -> Self {
        self.rels.retain(|r| r.endpoints.involves(locus));
        self
    }

    /// Keep only relationships connecting `a` and `b` (direction-agnostic).
    pub fn between(mut self, a: LocusId, b: LocusId) -> Self {
        self.rels
            .retain(|r| r.endpoints.involves(a) && r.endpoints.involves(b));
        self
    }

    /// Keep only relationships whose activity exceeds `threshold`.
    pub fn above_activity(mut self, threshold: f32) -> Self {
        self.rels.retain(|r| r.activity() > threshold);
        self
    }

    /// Keep only relationships whose combined strength (`activity + weight`)
    /// exceeds `threshold`.
    pub fn above_strength(mut self, threshold: f32) -> Self {
        self.rels.retain(|r| r.strength() > threshold);
        self
    }

    /// Keep only relationships where `state[slot_idx]` satisfies `pred`.
    ///
    /// Relationships whose state vector is shorter than `slot_idx + 1` are excluded.
    pub fn where_slot(mut self, slot_idx: usize, pred: impl Fn(f32) -> bool) -> Self {
        self.rels.retain(|r| {
            r.state
                .as_slice()
                .get(slot_idx)
                .is_some_and(|&v| pred(v))
        });
        self
    }

    /// Keep only relationships matching a custom predicate.
    pub fn matching(mut self, pred: impl Fn(&Relationship) -> bool) -> Self {
        self.rels.retain(|r| pred(r));
        self
    }

    /// Keep only directed relationships whose source is any of `loci`.
    ///
    /// Symmetric edges are excluded.
    pub fn from_any(mut self, loci: &[LocusId]) -> Self {
        let set: FxHashSet<LocusId> = loci.iter().copied().collect();
        self.rels.retain(|r| {
            matches!(r.endpoints, Endpoints::Directed { from, .. } if set.contains(&from))
        });
        self
    }

    /// Keep only directed relationships whose target is any of `loci`.
    ///
    /// Symmetric edges are excluded.
    pub fn to_any(mut self, loci: &[LocusId]) -> Self {
        let set: FxHashSet<LocusId> = loci.iter().copied().collect();
        self.rels.retain(|r| {
            matches!(r.endpoints, Endpoints::Directed { to, .. } if set.contains(&to))
        });
        self
    }

    /// Keep only relationships that involve any of `loci` at either endpoint
    /// (directed or symmetric).
    pub fn touching_any(mut self, loci: &[LocusId]) -> Self {
        let set: FxHashSet<LocusId> = loci.iter().copied().collect();
        self.rels
            .retain(|r| set.iter().any(|&id| r.endpoints.involves(id)));
        self
    }

    // ── Lifecycle filters ─────────────────────────────────────────────────────

    /// Keep only relationships created within the inclusive batch range
    /// `[from, to]`.
    pub fn created_in(mut self, from: BatchId, to: BatchId) -> Self {
        self.rels
            .retain(|r| r.created_batch >= from && r.created_batch <= to);
        self
    }

    /// Keep only relationships whose age (`current_batch − created_batch`) is
    /// at least `min_batches`.
    pub fn older_than(mut self, current_batch: BatchId, min_batches: u64) -> Self {
        self.rels
            .retain(|r| r.age_in_batches(current_batch) >= min_batches);
        self
    }

    /// Keep only relationships idle for at least `min_batches` batches
    /// (`current_batch − last_decayed_batch >= min_batches`).
    pub fn idle_for(mut self, current_batch: BatchId, min_batches: u64) -> Self {
        self.rels.retain(|r| {
            current_batch.0.saturating_sub(r.last_decayed_batch) >= min_batches
        });
        self
    }

    // ── Ordering ──────────────────────────────────────────────────────────────

    /// Keep the top `n` relationships by strength (`activity + weight`)
    /// in descending order.
    pub fn top_n_by_strength(mut self, n: usize) -> Self {
        self.rels
            .sort_unstable_by(|a, b| b.strength().total_cmp(&a.strength()));
        self.rels.truncate(n);
        self
    }

    /// Keep the top `n` relationships by activity in descending order.
    pub fn top_n_by_activity(mut self, n: usize) -> Self {
        self.rels
            .sort_unstable_by(|a, b| b.activity().total_cmp(&a.activity()));
        self.rels.truncate(n);
        self
    }

    /// Keep the top `n` relationships by `change_count` in descending order.
    pub fn top_n_by_change_count(mut self, n: usize) -> Self {
        self.rels
            .sort_unstable_by_key(|r| std::cmp::Reverse(r.lineage.change_count));
        self.rels.truncate(n);
        self
    }

    // ── Terminal methods ──────────────────────────────────────────────────────

    /// Collect matching relationships as `&Relationship` references.
    pub fn collect(self) -> Vec<&'w Relationship> {
        self.rels
    }

    /// Collect just the `RelationshipId`s.
    pub fn ids(self) -> Vec<RelationshipId> {
        self.rels.into_iter().map(|r| r.id).collect()
    }

    /// Number of matching relationships.
    pub fn count(self) -> usize {
        self.rels.len()
    }

    /// First matching relationship (in current order), or `None` if empty.
    pub fn first(self) -> Option<&'w Relationship> {
        self.rels.into_iter().next()
    }

    /// `true` when no relationships match the current constraints.
    pub fn is_empty(&self) -> bool {
        self.rels.is_empty()
    }

    // ── Aggregation terminals ─────────────────────────────────────────────────

    /// Sum of `activity()` across all matching relationships.
    pub fn sum_activity(self) -> f32 {
        self.rels.iter().map(|r| r.activity()).sum()
    }

    /// Mean `activity()` across all matching relationships, or `None` if the
    /// set is empty.
    pub fn mean_activity(self) -> Option<f32> {
        let n = self.rels.len();
        if n == 0 {
            return None;
        }
        Some(self.rels.iter().map(|r| r.activity()).sum::<f32>() / n as f32)
    }

    /// Aggregate activity statistics for the current result set, or `None` if
    /// the set is empty.
    pub fn activity_stats(self) -> Option<ActivityStats> {
        let n = self.rels.len();
        if n == 0 {
            return None;
        }
        let activities: Vec<f32> = self.rels.iter().map(|r| r.activity()).collect();
        let sum: f32 = activities.iter().sum();
        let min = activities.iter().copied().fold(f32::INFINITY, f32::min);
        let max = activities.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        Some(ActivityStats {
            count: n,
            sum,
            mean: sum / n as f32,
            min,
            max,
        })
    }

    // ── Cross-builder navigation ──────────────────────────────────────────────

    /// Pivot to a [`LociQuery`] over the **source** loci of all directed
    /// relationships in the current set.
    ///
    /// Symmetric edges have no source and are skipped. Duplicate loci are
    /// deduplicated; loci no longer present in the world are silently omitted.
    pub fn source_loci(self) -> LociQuery<'w> {
        let mut seen = FxHashSet::default();
        let loci: Vec<&'w Locus> = self
            .rels
            .iter()
            .filter_map(|r| r.endpoints.source())
            .filter(|&id| seen.insert(id))
            .filter_map(|id| self.world.locus(id))
            .collect();
        LociQuery { world: self.world, loci }
    }

    /// Pivot to a [`LociQuery`] over the **target** loci of all directed
    /// relationships in the current set.
    ///
    /// Symmetric edges have no target and are skipped. Duplicates deduplicated.
    pub fn target_loci(self) -> LociQuery<'w> {
        let mut seen = FxHashSet::default();
        let loci: Vec<&'w Locus> = self
            .rels
            .iter()
            .filter_map(|r| r.endpoints.target())
            .filter(|&id| seen.insert(id))
            .filter_map(|id| self.world.locus(id))
            .collect();
        LociQuery { world: self.world, loci }
    }

    /// Pivot to a [`LociQuery`] over **all** endpoint loci of the current
    /// relationship set (both directed endpoints and symmetric peers).
    ///
    /// Duplicates are deduplicated; loci no longer in the world are omitted.
    pub fn endpoint_loci(self) -> LociQuery<'w> {
        let mut seen = FxHashSet::default();
        let ids: Vec<LocusId> = self
            .rels
            .iter()
            .flat_map(|r| match r.endpoints {
                Endpoints::Directed { from, to } => [from, to],
                Endpoints::Symmetric { a, b } => [a, b],
            })
            .filter(|&id| seen.insert(id))
            .collect();
        LociQuery {
            world: self.world,
            loci: ids
                .into_iter()
                .filter_map(|id| self.world.locus(id))
                .collect(),
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId, Relationship,
        RelationshipLineage, StateVector,
    };
    use graph_world::World;

    // Build a small world:
    //   L0(kind=1, state=[0.9]) → L1(kind=1, state=[0.4]) → L2(kind=2, state=[0.2])
    //   L3(kind=2, state=[0.7]) is isolated
    fn world() -> World {
        let mut w = World::new();
        let rk = InfluenceKindId(1);
        for (id, kind, v) in [(0u64, 1u64, 0.9f32), (1, 1, 0.4), (2, 2, 0.2), (3, 2, 0.7)] {
            w.insert_locus(Locus::new(
                LocusId(id),
                LocusKindId(kind),
                StateVector::from_slice(&[v]),
            ));
        }
        w.properties_mut().insert(LocusId(0), graph_core::props! { "tag" => "hub" });

        for (from, to) in [(0u64, 1u64), (1, 2)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::directed(LocusId(from), LocusId(to)),
                state: StateVector::from_slice(&[1.5, 0.3]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 4,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    // ── LociQuery ─────────────────────────────────────────────────────────────

    #[test]
    fn loci_query_of_kind_filters() {
        let w = world();
        assert_eq!(loci(&w).of_kind(LocusKindId(1)).count(), 2);
        assert_eq!(loci(&w).of_kind(LocusKindId(2)).count(), 2);
        assert_eq!(loci(&w).of_kind(LocusKindId(99)).count(), 0);
    }

    #[test]
    fn loci_query_where_state_chains() {
        let w = world();
        // kind=1 AND state > 0.5: only L0 (0.9), not L1 (0.4)
        let result = loci(&w).of_kind(LocusKindId(1)).where_state(0, |v| v > 0.5).ids();
        assert_eq!(result, vec![LocusId(0)]);
    }

    #[test]
    fn loci_query_reachable_from_seeds_bfs() {
        let w = world();
        // L0 can reach L1 (depth 1) and L2 (depth 2)
        let reach2 = loci(&w).reachable_from(LocusId(0), 2).ids();
        assert!(reach2.contains(&LocusId(1)));
        assert!(reach2.contains(&LocusId(2)));
        assert!(!reach2.contains(&LocusId(3))); // isolated
    }

    #[test]
    fn loci_query_top_n_by_state_sorts_and_truncates() {
        let w = world();
        let top2 = loci(&w).sort_by_state(0).collect();
        // Descending: L0(0.9), L3(0.7), L1(0.4), L2(0.2)
        assert_eq!(top2[0].id, LocusId(0));
        assert_eq!(top2[1].id, LocusId(3));

        let top1 = loci(&w).top_n_by_state(0, 1).first().unwrap();
        assert_eq!(top1.id, LocusId(0));
    }

    #[test]
    fn loci_query_min_degree_excludes_isolated() {
        let w = world();
        // L3 is isolated (degree 0)
        let connected = loci(&w).min_degree(1).ids();
        assert!(!connected.contains(&LocusId(3)));
        assert!(connected.contains(&LocusId(0)));
    }

    #[test]
    fn loci_from_ids_skips_missing() {
        let w = world();
        let ids = vec![LocusId(0), LocusId(99), LocusId(1)];
        let found = loci_from_ids(&w, &ids).count();
        assert_eq!(found, 2); // 99 is missing
    }

    #[test]
    fn loci_query_where_str_property() {
        let w = world();
        let tagged = loci(&w).where_str_property("tag", |v| v == "hub").ids();
        assert_eq!(tagged, vec![LocusId(0)]);
    }

    #[test]
    fn loci_query_is_empty() {
        let w = world();
        assert!(!loci(&w).is_empty());
        assert!(loci(&w).of_kind(LocusKindId(99)).is_empty());
    }

    // ── RelationshipsQuery ────────────────────────────────────────────────────

    #[test]
    fn rel_query_from_filters_directed_source() {
        let w = world();
        // Only L0→L1 originates from L0
        let from0 = relationships(&w).from(LocusId(0)).collect();
        assert_eq!(from0.len(), 1);
        assert_eq!(from0[0].endpoints.source(), Some(LocusId(0)));
    }

    #[test]
    fn rel_query_to_filters_directed_target() {
        let w = world();
        let to2 = relationships(&w).to(LocusId(2)).collect();
        assert_eq!(to2.len(), 1);
        assert_eq!(to2[0].endpoints.target(), Some(LocusId(2)));
    }

    #[test]
    fn rel_query_chained_kind_and_strength() {
        let w = world();
        let rk = InfluenceKindId(1);
        // All edges are kind 1 with strength 1.8; above_strength(1.0) keeps all
        let strong = relationships(&w).of_kind(rk).above_strength(1.0).count();
        assert_eq!(strong, 2);
        // above_strength(2.0) keeps none (max = 1.8)
        assert_eq!(relationships(&w).above_strength(2.0).count(), 0);
    }

    #[test]
    fn rel_query_touching_covers_both_endpoints() {
        let w = world();
        // L1 appears in both L0→L1 and L1→L2
        let touching1 = relationships(&w).touching(LocusId(1)).count();
        assert_eq!(touching1, 2);
        // L3 is isolated
        assert_eq!(relationships(&w).touching(LocusId(3)).count(), 0);
    }

    #[test]
    fn rel_query_top_n_by_strength_sorted() {
        let w = world();
        let top1 = relationships(&w).top_n_by_strength(1).first().unwrap();
        // Both edges have equal strength; top_n returns one
        assert!((top1.strength() - 1.8).abs() < 1e-5);
    }

    #[test]
    fn rel_from_ids_skips_missing() {
        let w = world();
        let all_ids: Vec<_> = w.relationships().iter().map(|r| r.id).collect();
        let with_bad: Vec<_> = all_ids
            .iter()
            .copied()
            .chain([graph_core::RelationshipId(999)])
            .collect();
        let found = relationships_from_ids(&w, &with_bad).count();
        assert_eq!(found, all_ids.len()); // 999 skipped
    }

    #[test]
    fn rel_query_is_empty() {
        let w = world();
        assert!(!relationships(&w).is_empty());
        assert!(relationships(&w).of_kind(InfluenceKindId(99)).is_empty());
    }

    // ── Composition: traversal × filter ──────────────────────────────────────

    #[test]
    fn compose_reachable_and_state_filter() {
        let w = world();
        // Loci reachable from L0 within 2 hops, with state > 0.1
        // L1 (0.4) and L2 (0.2) qualify; L3 is not reachable
        let result = loci(&w)
            .reachable_from(LocusId(0), 2)
            .where_state(0, |v| v > 0.1)
            .ids();
        assert!(result.contains(&LocusId(1)));
        assert!(result.contains(&LocusId(2)));
        assert!(!result.contains(&LocusId(3)));
    }

    #[test]
    fn compose_from_and_strength_on_relationships() {
        let w = world();
        // Outgoing from L0, strength > 1.0
        let result = relationships(&w)
            .from(LocusId(0))
            .above_strength(1.0)
            .collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].endpoints.source(), Some(LocusId(0)));
    }
}
