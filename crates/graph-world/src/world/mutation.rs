use graph_core::{
    BatchId, Endpoints, KindObservation, Locus, Relationship, RelationshipId, RelationshipKindId,
    RelationshipLineage, StateVector,
};

use super::World;

impl World {
    pub fn insert_locus(&mut self, locus: Locus) {
        self.assign_partition_for_locus(&locus);
        self.loci.insert(locus);
    }

    pub fn add_relationship(
        &mut self,
        endpoints: Endpoints,
        kind: RelationshipKindId,
        state: StateVector,
    ) -> RelationshipId {
        let id = self.relationships.mint_id();
        let relationship = self.synthetic_relationship(id, endpoints, kind, state);
        self.relationships.insert(relationship);
        id
    }

    pub fn restore_relationship(&mut self, rel: Relationship) -> bool {
        if self.relationships.get(rel.id).is_some() {
            return false;
        }
        self.relationships.insert(rel);
        true
    }

    pub fn evict_cold_relationships(
        &mut self,
        threshold: f32,
        min_idle_batches: u64,
        current_batch: BatchId,
    ) -> Vec<RelationshipId> {
        let cold_ids = self.collect_cold_relationships(threshold, min_idle_batches, current_batch);
        self.remove_relationships(&cold_ids);
        cold_ids
    }

    fn synthetic_relationship(
        &self,
        id: RelationshipId,
        endpoints: Endpoints,
        kind: RelationshipKindId,
        state: StateVector,
    ) -> Relationship {
        Relationship {
            id,
            kind,
            endpoints,
            state,
            lineage: synthetic_lineage(kind),
            created_batch: self.current_batch,
            last_decayed_batch: self.current_batch.0,
            metadata: None,
        }
    }

    fn collect_cold_relationships(
        &self,
        threshold: f32,
        min_idle_batches: u64,
        current_batch: BatchId,
    ) -> Vec<RelationshipId> {
        self.relationships
            .iter()
            .filter(|relationship| {
                // Compare magnitude — Phase 1 of the trigger-axis roadmap allows
                // signed activity (inhibitory edges decrement); a strongly
                // negative edge is not "cold".
                relationship.activity().abs() < threshold
                    && current_batch
                        .0
                        .saturating_sub(relationship.last_decayed_batch)
                        >= min_idle_batches
            })
            .map(|relationship| relationship.id)
            .collect()
    }

    fn remove_relationships(&mut self, relationship_ids: &[RelationshipId]) {
        for &id in relationship_ids {
            self.relationships.remove(id);
        }
    }
}

fn synthetic_lineage(kind: RelationshipKindId) -> RelationshipLineage {
    RelationshipLineage {
        created_by: None,
        last_touched_by: None,
        change_count: 0,
        kinds_observed: smallvec::smallvec![KindObservation::synthetic(kind)],
    }
}
