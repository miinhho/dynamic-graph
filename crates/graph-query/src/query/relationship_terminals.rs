use graph_core::RelationshipId;

use super::{ActivityStats, RelationshipsQuery, relationship_aggregation::activity_stats_for};

impl<'w> RelationshipsQuery<'w> {
    /// Collect matching relationships as `&Relationship` references.
    pub fn collect(self) -> Vec<&'w graph_core::Relationship> {
        self.rels
    }

    /// Collect just the `RelationshipId`s.
    pub fn ids(self) -> Vec<RelationshipId> {
        self.rels.into_iter().map(|r| r.id).collect()
    }

    /// Number of matching relationships.
    pub fn count(self) -> usize {
        self.rels.len()
    }

    /// First matching relationship (in current order), or `None` if empty.
    pub fn first(self) -> Option<&'w graph_core::Relationship> {
        self.rels.into_iter().next()
    }

    /// `true` when no relationships match the current constraints.
    pub fn is_empty(&self) -> bool {
        self.rels.is_empty()
    }

    /// Sum of `activity()` across all matching relationships.
    pub fn sum_activity(self) -> f32 {
        self.rels.iter().map(|r| r.activity()).sum()
    }

    /// Mean `activity()` across all matching relationships.
    pub fn mean_activity(self) -> Option<f32> {
        let count = self.rels.len();
        (count > 0).then(|| self.rels.iter().map(|r| r.activity()).sum::<f32>() / count as f32)
    }

    /// Aggregate activity statistics for the current result set.
    pub fn activity_stats(self) -> Option<ActivityStats> {
        activity_stats_for(self.rels)
    }
}
