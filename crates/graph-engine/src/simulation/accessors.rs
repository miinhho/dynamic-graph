use std::sync::{Arc, RwLock};

use graph_core::{BatchId, InfluenceKindId, RelationshipId};
use graph_world::World;

use crate::engine::Engine;
use crate::regime::{AdaptiveGuardRail, BatchHistory};

use super::{EventHistory, Simulation};

impl Simulation {
    #[inline]
    pub fn world(&self) -> std::sync::RwLockReadGuard<'_, World> {
        self.world.read().unwrap()
    }

    pub fn activity_decay_rates(&self) -> graph_query::DecayRates {
        let mut rates = graph_query::DecayRates::default();
        for kind in self.base_influences.kinds() {
            if let Some(cfg) = self.base_influences.get(kind) {
                rates.insert(kind, cfg.decay_per_batch);
            }
        }
        rates
    }

    #[inline]
    pub fn world_mut(&mut self) -> std::sync::RwLockWriteGuard<'_, World> {
        self.world.write().unwrap()
    }

    #[inline]
    pub fn world_handle(&self) -> Arc<RwLock<World>> {
        Arc::clone(&self.world)
    }

    pub fn into_world(self) -> World {
        Arc::try_unwrap(self.world)
            .unwrap_or_else(|_| {
                panic!("world Arc has other owners; drop all world_handle clones first")
            })
            .into_inner()
            .unwrap()
    }

    pub fn event_history(&self) -> Option<&EventHistory> {
        self.event_history.as_ref()
    }

    pub fn entities_at_batch(
        &self,
        batch: BatchId,
    ) -> Vec<(graph_core::EntityId, graph_core::EntityLayer)> {
        self.world
            .read()
            .unwrap()
            .entities_at_batch(batch)
            .into_iter()
            .map(|(id, layer)| (id, layer.clone()))
            .collect()
    }

    pub fn rel_slot_value(
        &self,
        rel_id: RelationshipId,
        kind: InfluenceKindId,
        slot_name: &str,
    ) -> Option<f32> {
        let world = self.world.read().unwrap();
        let rel_state = world.relationships().get(rel_id)?.state.clone();
        drop(world);
        self.base_influences
            .get(kind)?
            .read_slot(&rel_state, slot_name)
    }

    pub fn slot_history(
        &self,
        rel_id: RelationshipId,
        kind: InfluenceKindId,
        slot_name: &str,
        since: BatchId,
    ) -> Vec<(BatchId, f32)> {
        let slot_idx = match self
            .base_influences
            .get(kind)
            .and_then(|c| c.slot_index(slot_name))
        {
            Some(idx) => idx,
            None => return Vec::new(),
        };
        let world = self.world.read().unwrap();
        world
            .changes_to_relationship(rel_id)
            .take_while(|c| c.batch.0 >= since.0)
            .filter_map(|c| {
                c.after
                    .as_slice()
                    .get(slot_idx)
                    .copied()
                    .map(|v| (c.batch, v))
            })
            .collect()
    }

    pub fn current_batch(&self) -> BatchId {
        self.world.read().unwrap().current_batch()
    }

    pub fn locus(&self, id: graph_core::LocusId) -> Option<graph_core::Locus> {
        self.world.read().unwrap().locus(id).cloned()
    }

    pub fn relationship(&self, id: RelationshipId) -> Option<graph_core::Relationship> {
        self.world.read().unwrap().relationships().get(id).cloned()
    }

    pub fn loci_of_kind(&self, kind: graph_core::LocusKindId) -> Vec<graph_core::Locus> {
        let world = self.world.read().unwrap();
        graph_query::loci_of_kind(&world, kind)
            .into_iter()
            .cloned()
            .collect()
    }

    pub fn relationship_between(
        &self,
        a: graph_core::LocusId,
        b: graph_core::LocusId,
    ) -> Option<graph_core::Relationship> {
        self.world
            .read()
            .unwrap()
            .relationships()
            .relationships_between(a, b)
            .next()
            .cloned()
    }

    pub fn locus_kind_id(&self, name: &str) -> Option<graph_core::LocusKindId> {
        self.locus_kind_names.get(name).copied()
    }

    pub fn influence_kind_id(&self, name: &str) -> Option<graph_core::InfluenceKindId> {
        self.influence_kind_names.get(name).copied()
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn guard_rail(&self) -> &AdaptiveGuardRail {
        &self.guard_rail
    }

    pub fn history(&self) -> &BatchHistory {
        &self.history
    }
}
