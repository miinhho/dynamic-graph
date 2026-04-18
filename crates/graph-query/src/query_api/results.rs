use graph_core::{BatchId, EntityId, LocusId, RelationshipId};

use super::{
    CohereResult, EntityDiffSummary, LocusSummary, QueryResult, RelationshipProfileResult,
    RelationshipSummary, TrendResult, WorldMetricsResult,
};

impl QueryResult {
    pub fn into_loci(self) -> Option<Vec<LocusId>> {
        match self {
            QueryResult::Loci(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_locus_summaries(self) -> Option<Vec<LocusSummary>> {
        match self {
            QueryResult::LocusSummaries(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_relationship_summaries(self) -> Option<Vec<RelationshipSummary>> {
        match self {
            QueryResult::RelationshipSummaries(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_relationships(self) -> Option<Vec<RelationshipId>> {
        match self {
            QueryResult::Relationships(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_entities(self) -> Option<Vec<EntityId>> {
        match self {
            QueryResult::Entities(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_changes(self) -> Option<Vec<graph_core::ChangeId>> {
        match self {
            QueryResult::Changes(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_components(self) -> Option<Vec<Vec<LocusId>>> {
        match self {
            QueryResult::Components(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_communities(self) -> Option<Vec<Vec<LocusId>>> {
        match self {
            QueryResult::Communities(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_path(self) -> Option<Option<Vec<LocusId>>> {
        match self {
            QueryResult::Path(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_scores(self) -> Option<Vec<(LocusId, f32)>> {
        match self {
            QueryResult::LocusScores(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_bool(self) -> Option<bool> {
        match self {
            QueryResult::Bool(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_count(self) -> Option<usize> {
        match self {
            QueryResult::Count(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_score(self) -> Option<f32> {
        match self {
            QueryResult::Score(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_maybe_score(self) -> Option<Option<f32>> {
        match self {
            QueryResult::MaybeScore(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_trend(self) -> Option<TrendResult> {
        match self {
            QueryResult::Trend(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_entity_deviations(self) -> Option<Vec<EntityDiffSummary>> {
        match self {
            QueryResult::EntityDeviations(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_coheres(self) -> Option<Vec<CohereResult>> {
        match self {
            QueryResult::Coheres(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_relationship_profile(self) -> Option<RelationshipProfileResult> {
        match self {
            QueryResult::RelationshipProfile(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_world_metrics(self) -> Option<WorldMetricsResult> {
        match self {
            QueryResult::WorldMetrics(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_feedback_pairs(self) -> Option<Vec<(LocusId, LocusId, f32)>> {
        match self {
            QueryResult::FeedbackPairs(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_entity_layers(
        self,
    ) -> Option<
        Vec<(
            BatchId,
            graph_core::LayerTransition,
            graph_core::LifecycleCause,
        )>,
    > {
        match self {
            QueryResult::EntityLayers(v) => Some(v),
            _ => None,
        }
    }
}
