use graph_core::{Entity, EntityId};

use crate::query::{LociQuery, RelationshipsQuery};

use super::{EntitiesQuery, entity_projection};

impl<'w> EntitiesQuery<'w> {
    /// Consume the query and return all matching entities.
    pub fn collect(self) -> Vec<&'w Entity> {
        self.candidates
    }

    /// Return the `EntityId`s of all matching entities.
    pub fn ids(self) -> Vec<EntityId> {
        self.candidates.iter().map(|e| e.id).collect()
    }

    /// Number of matching entities.
    pub fn count(self) -> usize {
        self.candidates.len()
    }

    /// Mean coherence across matching entities, or `None` if empty.
    pub fn mean_coherence(self) -> Option<f32> {
        if self.candidates.is_empty() {
            return None;
        }
        let sum: f32 = self.candidates.iter().map(|e| e.current.coherence).sum();
        Some(sum / self.candidates.len() as f32)
    }

    /// The entity with the highest current coherence score, or `None` if empty.
    pub fn strongest(self) -> Option<&'w Entity> {
        entity_projection::strongest_entity(self.candidates)
    }

    /// Collect all loci that are members of any candidate entity (deduplicated),
    /// returning them as a [`LociQuery`] for further filtering.
    pub fn member_loci(self) -> LociQuery<'w> {
        entity_projection::member_loci_query(self.world, self.candidates)
    }

    /// Collect all relationships that are members of any candidate entity
    /// (deduplicated), returning them as a [`RelationshipsQuery`].
    pub fn member_relationships(self) -> RelationshipsQuery<'w> {
        entity_projection::member_relationships_query(self.world, self.candidates)
    }
}
