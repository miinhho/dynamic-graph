use graph_core::{Cohere, CohereMembers, EntityId, RelationshipId};

use super::CohereQuery;

impl<'w> CohereQuery<'w> {
    /// Keep only coheres with strength ≥ `min`.
    pub fn with_min_strength(mut self, min: f32) -> Self {
        self.candidates.retain(|c| c.strength >= min);
        self
    }

    /// Keep only coheres that contain `entity` as a member.
    pub fn with_entity_member(mut self, entity: EntityId) -> Self {
        self.candidates
            .retain(|cohere| has_entity_member(cohere, entity));
        self
    }

    /// Keep only coheres that contain `rel` as a member.
    pub fn with_relationship_member(mut self, rel: RelationshipId) -> Self {
        self.candidates
            .retain(|cohere| has_relationship_member(cohere, rel));
        self
    }

    /// Keep only coheres with at least `min` entity members.
    pub fn with_min_entity_count(mut self, min: usize) -> Self {
        self.candidates.retain(|c| c.members.entity_count() >= min);
        self
    }

    /// Keep only coheres with at least `min` relationship members.
    pub fn with_min_relationship_count(mut self, min: usize) -> Self {
        self.candidates
            .retain(|c| c.members.relationship_count() >= min);
        self
    }

    /// Keep only coheres matching an arbitrary predicate.
    pub fn matching<F>(mut self, pred: F) -> Self
    where
        F: Fn(&Cohere) -> bool,
    {
        self.candidates.retain(|c| pred(c));
        self
    }
}

pub(super) fn has_entity_member(cohere: &Cohere, entity: EntityId) -> bool {
    match &cohere.members {
        CohereMembers::Entities(ids) => ids.contains(&entity),
        CohereMembers::Mixed { entities, .. } => entities.contains(&entity),
        CohereMembers::Relationships(_) => false,
    }
}

pub(super) fn has_relationship_member(cohere: &Cohere, relationship: RelationshipId) -> bool {
    match &cohere.members {
        CohereMembers::Relationships(ids) => ids.contains(&relationship),
        CohereMembers::Mixed { relationships, .. } => relationships.contains(&relationship),
        CohereMembers::Entities(_) => false,
    }
}

pub(super) fn strongest_cohere(candidates: Vec<&Cohere>) -> Option<&Cohere> {
    candidates.into_iter().max_by(|a, b| {
        a.strength
            .partial_cmp(&b.strength)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}
