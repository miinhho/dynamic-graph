use graph_core::{BatchId, InfluenceKindId, InteractionEffect};
use graph_world::World;

use super::{RelationshipBundle, metrics};

impl RelationshipBundle<'_> {
    pub fn len(&self) -> usize {
        self.relationships.len()
    }

    pub fn is_empty(&self) -> bool {
        self.relationships.is_empty()
    }

    pub fn net_activity(&self) -> f32 {
        self.relationships
            .iter()
            .map(|relationship| relationship.activity())
            .sum()
    }

    pub fn net_activity_with_interactions<F>(&self, interaction_fn: F) -> f32
    where
        F: Fn(InfluenceKindId, InfluenceKindId) -> Option<InteractionEffect>,
    {
        metrics::net_activity_with_interactions(self, interaction_fn)
    }

    pub fn activity_by_kind(&self) -> Vec<(InfluenceKindId, f32)> {
        metrics::activity_by_kind(self)
    }

    pub fn dominant_kind(&self) -> Option<InfluenceKindId> {
        self.activity_by_kind()
            .into_iter()
            .next()
            .map(|(kind, _)| kind)
    }

    pub fn is_excitatory(&self) -> bool {
        self.net_activity() > 0.0
    }

    pub fn is_inhibitory(&self) -> bool {
        self.net_activity() < 0.0
    }

    pub fn profile_similarity(&self, other: &RelationshipBundle<'_>) -> f32 {
        metrics::profile_similarity(self, other)
    }

    pub fn profile_trend_similarity(
        &self,
        other: &RelationshipBundle<'_>,
        world: &World,
        from_batch: BatchId,
        to_batch: BatchId,
    ) -> f32 {
        metrics::profile_trend_similarity(self, other, world, from_batch, to_batch)
    }
}
