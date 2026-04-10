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

mod query;
mod snapshot;

pub use snapshot::{WorldMeta, WorldSnapshot};

use graph_core::{BatchId, Change, ChangeId, Locus, LocusId};

use crate::store::change_log::ChangeLog;
use crate::store::cohere_store::CohereStore;
use crate::store::entity_store::EntityStore;
use crate::store::locus_store::LocusStore;
use crate::store::name_index::NameIndex;
use crate::store::property_store::PropertyStore;
use crate::store::relationship_store::RelationshipStore;

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
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_locus(&mut self, locus: Locus) {
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

    pub fn current_batch(&self) -> BatchId {
        self.current_batch
    }

    /// Mint the next `ChangeId`. Engine-only — exposed on `World` so the
    /// counter advances atomically with appends.
    pub fn mint_change_id(&mut self) -> ChangeId {
        let id = ChangeId(self.next_change_id);
        self.next_change_id += 1;
        id
    }

    /// Append a change to the log. The engine is expected to have set
    /// `change.batch` to `current_batch` already; this method does not
    /// re-check that.
    pub fn append_change(&mut self, change: Change) -> ChangeId {
        self.log.append(change)
    }

    /// Advance to the next batch. Called once per batch by the engine
    /// after all changes for the current batch have committed.
    pub fn advance_batch(&mut self) -> BatchId {
        self.current_batch = BatchId(self.current_batch.0 + 1);
        self.current_batch
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
                    kinds_observed: vec![rel_kind],
                },
                last_decayed_batch: 0,
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
                    kinds_observed: vec![rk],
                },
                last_decayed_batch: 0,
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
                kinds_observed: vec![rk],
            },
            last_decayed_batch: 0,
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
                kinds_observed: vec![rk],
            },
            last_decayed_batch: 0, // idle since batch 0
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
