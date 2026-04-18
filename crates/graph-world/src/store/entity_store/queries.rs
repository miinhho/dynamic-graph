use graph_core::{BatchId, EntityId, EntityLayer, LocusId};
use rustc_hash::FxHashSet;

use super::EntityStore;

impl EntityStore {
    pub fn candidates_for_members(&self, loci: &FxHashSet<LocusId>) -> FxHashSet<EntityId> {
        let mut out = FxHashSet::default();
        for locus in loci {
            if let Some(ids) = self.by_member.get(locus) {
                out.extend(ids);
            }
        }
        out
    }

    pub fn layer_at_batch(&self, id: EntityId, batch: BatchId) -> Option<&EntityLayer> {
        let entity = self.get(id)?;
        let pos = entity.layers.partition_point(|l| l.batch <= batch);
        entity.layers.get(pos.wrapping_sub(1))
    }
}
