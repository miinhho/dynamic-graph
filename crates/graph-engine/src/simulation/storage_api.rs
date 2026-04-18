use graph_core::WorldEvent;
#[cfg(feature = "storage")]
use graph_core::{BatchId, LocusId, RelationshipId};
#[cfg(feature = "storage")]
use graph_world::World;

use crate::emergence::EmergencePerspective;
use crate::engine;

use super::Simulation;

impl Simulation {
    #[cfg(feature = "storage")]
    pub(super) fn persist_step_batches(&mut self, world: &World, current_batch: BatchId) {
        if !self.auto_commit || self.storage.is_none() {
            return;
        }
        let mut had_error = false;
        for batch_idx in self.last_flushed_batch.0..current_batch.0 {
            let storage = self.storage.as_ref().unwrap();
            if let Err(e) = storage.commit_batch(world, BatchId(batch_idx)) {
                self.last_storage_error = Some(e);
                had_error = true;
                break;
            }
        }
        if !had_error {
            self.last_flushed_batch = current_batch;
            if self.last_storage_error.is_some() {
                self.last_storage_error = None;
            }
        }
    }

    #[cfg(feature = "storage")]
    pub fn promote_relationship(&mut self, rel_id: RelationshipId) -> bool {
        let Some(ref storage) = self.storage else {
            return false;
        };
        match storage.get_relationship(rel_id) {
            Ok(Some(rel)) => self.world.write().unwrap().restore_relationship(rel),
            _ => false,
        }
    }

    #[cfg(feature = "storage")]
    pub fn promote_relationships_for_locus(&mut self, locus_id: LocusId) -> usize {
        let Some(ref storage) = self.storage else {
            return 0;
        };
        match storage.relationships_for_locus(locus_id) {
            Ok(rels) => {
                let mut world = self.world.write().unwrap();
                rels.into_iter()
                    .filter(|r| world.restore_relationship(r.clone()))
                    .count()
            }
            Err(e) => {
                self.last_storage_error = Some(e);
                0
            }
        }
    }

    #[cfg(feature = "storage")]
    pub fn promote_all_cold(&mut self) -> usize {
        let Some(ref storage) = self.storage else {
            return 0;
        };
        match storage.all_relationships() {
            Ok(rels) => {
                let mut world = self.world.write().unwrap();
                rels.into_iter()
                    .filter(|r| world.restore_relationship(r.clone()))
                    .count()
            }
            Err(e) => {
                self.last_storage_error = Some(e);
                0
            }
        }
    }

    pub fn recognize_entities_with_promotion(
        &mut self,
        perspective: &dyn EmergencePerspective,
    ) -> Vec<WorldEvent> {
        #[cfg(feature = "storage")]
        self.promote_all_cold();
        engine::world_ops::recognize_entities(
            &mut self.world.write().unwrap(),
            &self.base_influences,
            perspective,
        )
    }
}
