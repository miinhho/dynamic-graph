//! The substrate's mutable container.
//!
//! `World` owns:
//! - the `LocusStore` (Layer 0 state),
//! - the `ChangeLog` (Layer 1 history),
//! - the monotonic `BatchId` clock,
//! - the next `ChangeId` counter the engine uses when minting changes.
//!
//! It does *not* own the kind registries (`LocusKindRegistry`,
//! `InfluenceKindRegistry`); those live next to the engine and are
//! threaded into ticks. Keeping the world free of program references
//! makes snapshots cheap and replay clean.

mod partition;
mod query;
mod snapshot;

pub use partition::{PartitionFn, PartitionIndex};
pub use snapshot::{WorldMeta, WorldSnapshot};

use graph_core::{BatchId, Change, ChangeId, Endpoints, KindObservation, Locus, LocusId, Relationship, RelationshipId, RelationshipKindId, RelationshipLineage, StateVector};
use rustc_hash::FxHashMap;

use crate::store::change_log::ChangeLog;
use crate::store::cohere_store::CohereStore;
use crate::store::entity_store::EntityStore;
use crate::store::locus_store::LocusStore;
use crate::store::name_index::NameIndex;
use crate::store::property_store::PropertyStore;
use crate::store::relationship_store::RelationshipStore;
use crate::store::subscription_store::SubscriptionStore;

#[derive(Debug, Default, Clone)]
pub struct World {
    pub(crate) loci: LocusStore,
    pub(crate) relationships: RelationshipStore,
    pub(crate) entities: EntityStore,
    coheres: CohereStore,
    properties: PropertyStore,
    names: NameIndex,
    pub(crate) log: ChangeLog,
    pub(crate) current_batch: BatchId,
    pub(crate) next_change_id: u64,
    subscriptions: SubscriptionStore,
    /// Append-only log of relationships pruned by `flush_relationship_decay`.
    /// Each entry is `(relationship_id, batch_at_pruning_time)`.
    /// Trim via `trim_pruned_log_before` alongside `ChangeLog::trim_before_batch`.
    pruned_log: Vec<(RelationshipId, BatchId)>,
    /// Per-locus BCM sliding threshold θ_M. Updated by `apply_hebbian_updates`
    /// when `PlasticityConfig::bcm` is true. Empty for non-BCM simulations.
    bcm_thresholds: FxHashMap<LocusId, f32>,
    /// Optional partition assignment — set by callers who want E4 partition
    /// parallelism. `None` (the default) means single-partition mode with no
    /// overhead.
    partition_index: Option<PartitionIndex>,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_locus(&mut self, locus: Locus) {
        if let Some(idx) = &mut self.partition_index {
            idx.assign(&locus);
        }
        self.loci.insert(locus);
    }

    pub fn locus(&self, id: LocusId) -> Option<&Locus> {
        self.loci.get(id)
    }

    pub fn locus_mut(&mut self, id: LocusId) -> Option<&mut Locus> {
        self.loci.get_mut(id)
    }

    pub fn loci(&self) -> &LocusStore {
        &self.loci
    }

    pub fn loci_mut(&mut self) -> &mut LocusStore {
        &mut self.loci
    }

    pub fn relationships(&self) -> &RelationshipStore {
        &self.relationships
    }

    pub fn relationships_mut(&mut self) -> &mut RelationshipStore {
        &mut self.relationships
    }

    pub fn entities(&self) -> &EntityStore {
        &self.entities
    }

    pub fn entities_mut(&mut self) -> &mut EntityStore {
        &mut self.entities
    }

    pub fn coheres(&self) -> &CohereStore {
        &self.coheres
    }

    pub fn coheres_mut(&mut self) -> &mut CohereStore {
        &mut self.coheres
    }

    pub fn subscriptions(&self) -> &SubscriptionStore {
        &self.subscriptions
    }

    pub fn subscriptions_mut(&mut self) -> &mut SubscriptionStore {
        &mut self.subscriptions
    }

    pub fn log(&self) -> &ChangeLog {
        &self.log
    }

    pub fn log_mut(&mut self) -> &mut ChangeLog {
        &mut self.log
    }

    pub fn properties(&self) -> &PropertyStore {
        &self.properties
    }

    pub fn properties_mut(&mut self) -> &mut PropertyStore {
        &mut self.properties
    }

    pub fn names(&self) -> &NameIndex {
        &self.names
    }

    pub fn names_mut(&mut self) -> &mut NameIndex {
        &mut self.names
    }

    /// Returns the batch ID that **will be used for the next commit** — i.e.
    /// the batch the engine is currently building, not the last written batch.
    ///
    /// This is an "upcoming" pointer, not a "last written" pointer.  To get
    /// the highest batch already committed, use [`last_committed_batch`].
    ///
    /// ## Engine invariant
    ///
    /// The engine reads `current_batch()` at the start of each iteration,
    /// uses it for all `Change.batch` fields, then calls `advance_batch()` at
    /// the end.  Between those two calls, `current_batch()` equals the batch
    /// that is actively being assembled.
    ///
    /// [`last_committed_batch`]: World::last_committed_batch
    pub fn current_batch(&self) -> BatchId {
        self.current_batch
    }

    /// Returns the highest batch that has already been committed to the log,
    /// or `None` if no changes have been committed yet.
    ///
    /// Use this when you need the last *written* batch — for example, to
    /// capture a baseline before stimulating the world:
    ///
    /// ```ignore
    /// let baseline = world.last_committed_batch();
    /// sim.stimulate(changes);
    /// sim.tick();
    /// let root_changes = world.log().batch(world.last_committed_batch().unwrap())
    ///     .map(|c| c.id).collect::<Vec<_>>();
    /// ```
    ///
    /// Note: `current_batch()` returns the batch ID that will be used *next*,
    /// which is one ahead of this value.
    pub fn last_committed_batch(&self) -> Option<BatchId> {
        let c = self.current_batch.0;
        if c == 0 { None } else { Some(BatchId(c - 1)) }
    }

    /// Mint the next `ChangeId`. Engine-only — exposed on `World` so the
    /// counter advances atomically with appends.
    pub fn mint_change_id(&mut self) -> ChangeId {
        let id = ChangeId(self.next_change_id);
        self.next_change_id += 1;
        id
    }

    /// Reserve a contiguous block of `n` `ChangeId`s in one step.
    ///
    /// Returns the base `ChangeId` (the first of the reserved block).
    /// The caller is responsible for assigning IDs `base`, `base+1`, …,
    /// `base+n-1` and appending the corresponding `Change`s in that order
    /// to preserve the density invariant.
    ///
    /// Engine-only — call `mint_change_id` for single-change paths.
    pub fn reserve_change_ids(&mut self, n: usize) -> ChangeId {
        let base = ChangeId(self.next_change_id);
        self.next_change_id += n as u64;
        base
    }

    /// Append a change to the log. The engine is expected to have set
    /// `change.batch` to `current_batch` already; this method does not
    /// re-check that.
    pub fn append_change(&mut self, change: Change) -> ChangeId {
        self.log.append(change)
    }

    /// Bulk-append a Vec of pre-built changes from the same batch.
    ///
    /// Delegates to [`ChangeLog::extend_batch`]: all reverse indices are
    /// updated in one grouping pass instead of one HashMap op per change.
    /// Use this instead of repeated `append_change` calls when committing
    /// an entire batch at once.
    pub fn extend_batch_changes(&mut self, changes: Vec<Change>) {
        self.log.extend_batch(changes);
    }

    /// Advance to the next batch. Called once per batch by the engine
    /// after all changes for the current batch have committed.
    pub fn advance_batch(&mut self) -> BatchId {
        self.current_batch = BatchId(self.current_batch.0 + 1);
        self.current_batch
    }

    /// Record that a relationship was pruned at the current batch.
    ///
    /// Called by the engine's `flush_relationship_decay` after removing a
    /// relationship whose activity fell below `prune_activity_threshold`.
    /// The log is queryable via `pruned_log()` and reflected in `WorldDiff`.
    pub fn record_pruned(&mut self, rel_id: RelationshipId) {
        self.pruned_log.push((rel_id, self.current_batch));
    }

    /// All pruning events recorded since the last `trim_pruned_log_before` call.
    /// Each entry is `(relationship_id, batch_at_pruning_time)`.
    pub fn pruned_log(&self) -> &[(RelationshipId, BatchId)] {
        &self.pruned_log
    }

    /// Discard pruning log entries older than `batch`. Call alongside
    /// `ChangeLog::trim_before_batch` to keep memory bounded.
    pub fn trim_pruned_log_before(&mut self, batch: BatchId) {
        self.pruned_log.retain(|(_, b)| b.0 >= batch.0);
    }

    /// Return the BCM sliding threshold θ_M for `id`, defaulting to `0.0`.
    pub fn bcm_threshold(&self, id: LocusId) -> f32 {
        self.bcm_thresholds.get(&id).copied().unwrap_or(0.0)
    }

    /// Read-only access to the full BCM threshold map.
    pub fn bcm_thresholds(&self) -> &FxHashMap<LocusId, f32> {
        &self.bcm_thresholds
    }

    /// Mutable access to the BCM threshold map.
    pub fn bcm_thresholds_mut(&mut self) -> &mut FxHashMap<LocusId, f32> {
        &mut self.bcm_thresholds
    }

    /// Insert a new relationship with `last_decayed_batch` pre-set to
    /// `current_batch()`, preventing spurious decay debt on first access.
    ///
    /// This is the preferred way to pre-create relationships in a running
    /// world. Using `relationships_mut().insert()` directly leaves
    /// `last_decayed_batch = 0`, which causes all accumulated batches to be
    /// replayed as decay on the first touch.
    pub fn add_relationship(
        &mut self,
        endpoints: Endpoints,
        kind: RelationshipKindId,
        state: StateVector,
    ) -> RelationshipId {
        let id = self.relationships.mint_id();
        let current_batch = self.current_batch.0;
        self.relationships.insert(Relationship {
            id,
            kind,
            endpoints,
            state,
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(kind)],
            },
            created_batch: self.current_batch,
            last_decayed_batch: current_batch,
            metadata: None,
        });
        id
    }

    /// Restore a relationship that was previously evicted to cold storage.
    ///
    /// Unlike `relationships_mut().insert()`, this is idempotent: if the
    /// relationship is already present in memory (not evicted), the call
    /// is a no-op and returns `false`. Returns `true` if the relationship
    /// was actually inserted.
    pub fn restore_relationship(&mut self, rel: graph_core::Relationship) -> bool {
        if self.relationships.get(rel.id).is_some() {
            return false;
        }
        self.relationships.insert(rel);
        true
    }

    /// Remove relationships whose activity is below `threshold` and that
    /// have not been touched (decayed) for at least `min_idle_batches`.
    ///
    /// Returns the IDs of evicted relationships. The caller is responsible
    /// for ensuring these are already persisted in storage before calling
    /// this method.
    pub fn evict_cold_relationships(
        &mut self,
        threshold: f32,
        min_idle_batches: u64,
        current_batch: BatchId,
    ) -> Vec<graph_core::RelationshipId> {
        let cold_ids: Vec<_> = self
            .relationships
            .iter()
            .filter(|r| {
                r.activity() < threshold
                    && current_batch.0.saturating_sub(r.last_decayed_batch) >= min_idle_batches
            })
            .map(|r| r.id)
            .collect();

        for &id in &cold_ids {
            self.relationships.remove(id);
        }

        cold_ids
    }

    // ── E4 Partition API ──────────────────────────────────────────────────

    /// Attach a partition assignment function. All currently loaded loci are
    /// immediately assigned. Loci created after this call are assigned on
    /// insertion. Pass `None` to revert to single-partition mode.
    pub fn set_partition_fn(&mut self, f: Option<PartitionFn>) {
        match f {
            None => { self.partition_index = None; }
            Some(fn_) => {
                let mut idx = PartitionIndex::new(fn_);
                for locus in self.loci.iter() {
                    idx.assign(locus);
                }
                self.partition_index = Some(idx);
            }
        }
    }

    /// Re-evaluate every locus through the current partition fn and rebuild
    /// the assignment index. O(L). No-op if no partition fn is set.
    pub fn repartition(&mut self) {
        if let Some(idx) = self.partition_index.take() {
            let fn_ = idx.fn_.clone();
            let mut new_idx = PartitionIndex::new(fn_);
            for locus in self.loci.iter() {
                new_idx.assign(locus);
            }
            self.partition_index = Some(new_idx);
        }
    }

    /// Read-only access to the partition index, if one is active.
    pub fn partition_index(&self) -> Option<&PartitionIndex> {
        self.partition_index.as_ref()
    }


    /// Return the partition bucket for `locus_id`, or `None` if no partition
    /// fn is active or the locus was not yet assigned.
    pub fn partition_of(&self, locus_id: LocusId) -> Option<u64> {
        self.partition_index.as_ref()?.bucket_of(locus_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{LocusKindId, StateVector};

    #[test]
    fn change_id_counter_is_monotonic() {
        let mut w = World::new();
        let a = w.mint_change_id();
        let b = w.mint_change_id();
        assert_eq!(a.0, 0);
        assert_eq!(b.0, 1);
    }

    #[test]
    fn advance_batch_increments() {
        let mut w = World::new();
        assert_eq!(w.current_batch(), BatchId(0));
        assert_eq!(w.advance_batch(), BatchId(1));
        assert_eq!(w.current_batch(), BatchId(1));
    }

    #[test]
    fn world_holds_loci() {
        let mut w = World::new();
        w.insert_locus(Locus::new(
            LocusId(42),
            LocusKindId(1),
            StateVector::zeros(2),
        ));
        assert!(w.locus(LocusId(42)).is_some());
    }

    fn chain_world_manual(n: u64) -> World {
        use graph_core::{
            Endpoints, InfluenceKindId, Relationship, RelationshipKindId, RelationshipLineage,
            StateVector as SV,
        };
        let kind_id = LocusKindId(1);
        let rel_kind: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0..n {
            w.insert_locus(Locus::new(LocusId(i), kind_id, SV::zeros(1)));
        }
        for i in 0..(n - 1) {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rel_kind,
                endpoints: Endpoints::Directed { from: LocusId(i), to: LocusId(i + 1) },
                state: SV::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rel_kind)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    fn star_world_with_activity(arms: u64, activity: f32) -> World {
        use graph_core::{
            Endpoints, InfluenceKindId, Relationship, RelationshipKindId, RelationshipLineage,
            StateVector as SV,
        };
        let rk: RelationshipKindId = InfluenceKindId(1);
        let lk = LocusKindId(1);
        let mut w = World::new();
        for i in 0..=arms {
            w.insert_locus(Locus::new(LocusId(i), lk, SV::zeros(1)));
        }
        for i in 1..=arms {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(i) },
                state: SV::from_slice(&[activity, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    #[test]
    fn induced_subgraph_returns_only_internal_edges() {
        let w = chain_world_manual(4);
        let sub = w.induced_subgraph(&[LocusId(0), LocusId(1), LocusId(2)]);
        assert_eq!(sub.len(), 2);
        let endpoints: Vec<_> = sub.iter().map(|r| &r.endpoints).collect();
        use graph_core::Endpoints;
        assert!(endpoints.iter().any(|e| matches!(e, Endpoints::Directed { from: LocusId(0), to: LocusId(1) })));
        assert!(endpoints.iter().any(|e| matches!(e, Endpoints::Directed { from: LocusId(1), to: LocusId(2) })));
    }

    #[test]
    fn induced_subgraph_empty_set_returns_empty() {
        let w = chain_world_manual(4);
        assert!(w.induced_subgraph(&[]).is_empty());
    }

    #[test]
    fn induced_subgraph_singleton_returns_empty() {
        let w = chain_world_manual(4);
        assert!(w.induced_subgraph(&[LocusId(0)]).is_empty());
    }

    #[test]
    fn relationships_active_above_filters_by_threshold() {
        use graph_core::{
            Endpoints, InfluenceKindId, Relationship, RelationshipKindId, RelationshipLineage,
            StateVector as SV,
        };
        let rk: RelationshipKindId = InfluenceKindId(1);
        let lk = LocusKindId(1);
        let mut w = star_world_with_activity(3, 0.5);
        w.insert_locus(Locus::new(LocusId(10), lk, SV::zeros(1)));
        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(10) },
            state: SV::from_slice(&[0.05, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });

        let above_0 = w.relationships_active_above(0.0).count();
        assert_eq!(above_0, 4);
        let above_01 = w.relationships_active_above(0.1).count();
        assert_eq!(above_01, 3);
        let above_1 = w.relationships_active_above(1.0).count();
        assert_eq!(above_1, 0);
    }

    #[test]
    fn metrics_active_relationship_count_matches_manual_filter() {
        let w = star_world_with_activity(4, 0.5);
        let m = w.metrics();
        assert_eq!(m.active_relationship_count, 4);
    }

    #[test]
    fn evict_cold_relationships_removes_below_threshold() {
        // Star with 3 active (0.5) + 1 cold (0.05).
        use graph_core::{
            Endpoints, InfluenceKindId, Relationship, RelationshipKindId, RelationshipLineage,
            StateVector as SV,
        };
        let rk: RelationshipKindId = InfluenceKindId(1);
        let lk = LocusKindId(1);
        let mut w = star_world_with_activity(3, 0.5);

        // Add a cold relationship.
        w.insert_locus(Locus::new(LocusId(10), lk, SV::zeros(1)));
        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::Directed { from: LocusId(0), to: LocusId(10) },
            state: SV::from_slice(&[0.01, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0, // idle since batch 0
            metadata: None,
        });

        assert_eq!(w.relationships().len(), 4);

        // Evict with threshold=0.1, min_idle=10, current=100.
        let evicted = w.evict_cold_relationships(0.1, 10, BatchId(100));
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0], id);
        assert_eq!(w.relationships().len(), 3);
    }

    #[test]
    fn evict_cold_relationships_spares_recently_touched() {
        let mut w = star_world_with_activity(2, 0.01); // low activity
        // But last_decayed_batch = 0, and we say current = 5, min_idle = 10.
        // These should NOT be evicted (not idle long enough).
        let evicted = w.evict_cold_relationships(0.1, 10, BatchId(5));
        assert!(evicted.is_empty());
        assert_eq!(w.relationships().len(), 2);
    }
}
