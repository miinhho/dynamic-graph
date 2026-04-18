use graph_world::World;

use super::{Query, QueryResult};

pub(super) fn execute_causal_analysis(world: &World, query: &Query) -> Option<QueryResult> {
    match query {
        Query::CausalDirection { .. }
        | Query::DominantCauses { .. }
        | Query::DominantEffects { .. }
        | Query::CausalInStrength { .. }
        | Query::CausalOutStrength { .. }
        | Query::FeedbackPairs { .. } => Some(execute_weighted_causality_query(world, query)),
        Query::GrangerScore { .. }
        | Query::GrangerDominantCauses { .. }
        | Query::GrangerDominantEffects { .. } => Some(execute_granger_query(world, query)),
        Query::EntityTransitionCause { .. }
        | Query::EntityUpstreamTransitions { .. }
        | Query::EntityLayersInRange { .. } => Some(execute_entity_causality_query(world, query)),
        _ => None,
    }
}

fn execute_weighted_causality_query(world: &World, query: &Query) -> QueryResult {
    use crate::causal_strength::{
        causal_direction, causal_in_strength, causal_out_strength, dominant_causes,
        dominant_effects, feedback_pairs,
    };

    match query {
        Query::CausalDirection { from, to, kind } => {
            QueryResult::Score(causal_direction(world, *from, *to, *kind))
        }
        Query::DominantCauses { target, kind, n } => {
            QueryResult::LocusScores(dominant_causes(world, *target, *kind, *n))
        }
        Query::DominantEffects { source, kind, n } => {
            QueryResult::LocusScores(dominant_effects(world, *source, *kind, *n))
        }
        Query::CausalInStrength { locus, kind } => {
            QueryResult::Score(causal_in_strength(world, *locus, *kind))
        }
        Query::CausalOutStrength { locus, kind } => {
            QueryResult::Score(causal_out_strength(world, *locus, *kind))
        }
        Query::FeedbackPairs {
            kind,
            min_weight,
            min_balance,
        } => QueryResult::FeedbackPairs(feedback_pairs(world, *kind, *min_weight, *min_balance)),
        _ => unreachable!("weighted causality dispatcher received non-weighted query"),
    }
}

fn execute_granger_query(world: &World, query: &Query) -> QueryResult {
    use crate::causal_strength::{
        granger_dominant_causes, granger_dominant_effects, granger_score,
    };

    match query {
        Query::GrangerScore {
            from,
            to,
            kind,
            lag_batches,
        } => QueryResult::Score(granger_score(world, *from, *to, *kind, *lag_batches)),
        Query::GrangerDominantCauses {
            target,
            kind,
            lag_batches,
            n,
        } => QueryResult::LocusScores(granger_dominant_causes(
            world,
            *target,
            *kind,
            *lag_batches,
            *n,
        )),
        Query::GrangerDominantEffects {
            source,
            kind,
            lag_batches,
            n,
        } => QueryResult::LocusScores(granger_dominant_effects(
            world,
            *source,
            *kind,
            *lag_batches,
            *n,
        )),
        _ => unreachable!("granger dispatcher received non-granger query"),
    }
}

fn execute_entity_causality_query(world: &World, query: &Query) -> QueryResult {
    use crate::entity_causality::{
        entity_layers_in_range, entity_transition_cause, entity_upstream_transitions,
    };

    match query {
        Query::EntityTransitionCause {
            entity_id,
            at_batch,
        } => QueryResult::EntityCause(entity_transition_cause(world, *entity_id, *at_batch)),
        Query::EntityUpstreamTransitions {
            entity_id,
            at_batch,
        } => QueryResult::EntityTransitions(entity_upstream_transitions(
            world, *entity_id, *at_batch,
        )),
        Query::EntityLayersInRange {
            entity_id,
            from,
            to,
        } => QueryResult::EntityLayers(entity_layers_in_range(world, *entity_id, *from, *to)),
        _ => unreachable!("entity causality dispatcher received non-entity query"),
    }
}
