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

mod batching;
mod mutation;
mod partition;
mod partitioning;
mod query;
mod snapshot;

pub use partition::{PartitionFn, PartitionIndex};
pub use snapshot::{WorldMeta, WorldSnapshot};

use graph_core::{BatchId, Locus, LocusId, RelationshipId};
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{KindObservation, LocusKindId, StateVector};

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
                endpoints: Endpoints::Directed {
                    from: LocusId(i),
                    to: LocusId(i + 1),
                },
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
                endpoints: Endpoints::Directed {
                    from: LocusId(0),
                    to: LocusId(i),
                },
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
        assert!(endpoints.iter().any(|e| matches!(
            e,
            Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1)
            }
        )));
        assert!(endpoints.iter().any(|e| matches!(
            e,
            Endpoints::Directed {
                from: LocusId(1),
                to: LocusId(2)
            }
        )));
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
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(10),
            },
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
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(10),
            },
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
