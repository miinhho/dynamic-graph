use graph_core::{BatchId, Entity, EntityStatus, LocusId, RelationshipId};

use super::EntitiesQuery;

impl<'w> EntitiesQuery<'w> {
    /// Keep only active entities.
    pub fn active(mut self) -> Self {
        self.candidates.retain(|e| e.status == EntityStatus::Active);
        self
    }

    /// Keep only dormant entities.
    pub fn dormant(mut self) -> Self {
        self.candidates
            .retain(|e| e.status == EntityStatus::Dormant);
        self
    }

    /// Keep only entities whose current member set contains `locus`.
    pub fn with_member(mut self, locus: LocusId) -> Self {
        self.candidates
            .retain(|e| e.current.members.contains(&locus));
        self
    }

    /// Keep only entities whose current member relationship set contains `rel`.
    pub fn with_relationship_member(mut self, rel: RelationshipId) -> Self {
        self.candidates
            .retain(|e| e.current.member_relationships.contains(&rel));
        self
    }

    /// Keep only entities whose current coherence score is ≥ `min`.
    pub fn with_min_coherence(mut self, min: f32) -> Self {
        self.candidates.retain(|e| e.current.coherence >= min);
        self
    }

    /// Keep only entities whose birth layer was deposited at or after `batch`.
    ///
    /// The birth batch is taken from the first layer in the sediment stack
    /// (the `Born` transition layer). Entities with no layers are excluded.
    pub fn born_after(mut self, batch: BatchId) -> Self {
        self.candidates
            .retain(|e| e.layers.first().map(|l| l.batch >= batch).unwrap_or(false));
        self
    }

    /// Keep only entities born within `[from, to]` (inclusive).
    pub fn born_in_range(mut self, from: BatchId, to: BatchId) -> Self {
        self.candidates.retain(|e| {
            e.layers
                .first()
                .map(|l| l.batch >= from && l.batch <= to)
                .unwrap_or(false)
        });
        self
    }

    /// Keep only entities with at least `min` sediment layers.
    pub fn with_min_layer_count(mut self, min: usize) -> Self {
        self.candidates.retain(|e| e.layers.len() >= min);
        self
    }

    /// Keep only entities whose current member count is at least `min`.
    pub fn with_min_member_count(mut self, min: usize) -> Self {
        self.candidates.retain(|e| e.current.members.len() >= min);
        self
    }

    /// Keep only entities matching an arbitrary predicate.
    pub fn matching<F>(mut self, pred: F) -> Self
    where
        F: Fn(&Entity) -> bool,
    {
        self.candidates.retain(|e| pred(e));
        self
    }
}
