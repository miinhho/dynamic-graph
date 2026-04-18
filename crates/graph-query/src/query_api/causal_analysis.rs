use graph_world::World;

use super::{Query, QueryResult};

pub(super) fn execute_causal_analysis(world: &World, query: &Query) -> Option<QueryResult> {
    match query {
        Query::CausalDirection { from, to, kind } => Some(QueryResult::Score(
            crate::causal_strength::causal_direction(world, *from, *to, *kind),
        )),
        Query::DominantCauses { target, kind, n } => Some(QueryResult::LocusScores(
            crate::causal_strength::dominant_causes(world, *target, *kind, *n),
        )),
        Query::DominantEffects { source, kind, n } => Some(QueryResult::LocusScores(
            crate::causal_strength::dominant_effects(world, *source, *kind, *n),
        )),
        Query::CausalInStrength { locus, kind } => Some(QueryResult::Score(
            crate::causal_strength::causal_in_strength(world, *locus, *kind),
        )),
        Query::CausalOutStrength { locus, kind } => Some(QueryResult::Score(
            crate::causal_strength::causal_out_strength(world, *locus, *kind),
        )),
        Query::FeedbackPairs {
            kind,
            min_weight,
            min_balance,
        } => Some(QueryResult::FeedbackPairs(
            crate::causal_strength::feedback_pairs(world, *kind, *min_weight, *min_balance),
        )),
        Query::GrangerScore {
            from,
            to,
            kind,
            lag_batches,
        } => Some(QueryResult::Score(crate::causal_strength::granger_score(
            world,
            *from,
            *to,
            *kind,
            *lag_batches,
        ))),
        Query::GrangerDominantCauses {
            target,
            kind,
            lag_batches,
            n,
        } => Some(QueryResult::LocusScores(
            crate::causal_strength::granger_dominant_causes(
                world,
                *target,
                *kind,
                *lag_batches,
                *n,
            ),
        )),
        Query::GrangerDominantEffects {
            source,
            kind,
            lag_batches,
            n,
        } => Some(QueryResult::LocusScores(
            crate::causal_strength::granger_dominant_effects(
                world,
                *source,
                *kind,
                *lag_batches,
                *n,
            ),
        )),
        Query::EntityTransitionCause {
            entity_id,
            at_batch,
        } => Some(QueryResult::EntityCause(
            crate::entity_causality::entity_transition_cause(world, *entity_id, *at_batch),
        )),
        Query::EntityUpstreamTransitions {
            entity_id,
            at_batch,
        } => Some(QueryResult::EntityTransitions(
            crate::entity_causality::entity_upstream_transitions(world, *entity_id, *at_batch),
        )),
        Query::EntityLayersInRange {
            entity_id,
            from,
            to,
        } => Some(QueryResult::EntityLayers(
            crate::entity_causality::entity_layers_in_range(world, *entity_id, *from, *to),
        )),
        _ => None,
    }
}
