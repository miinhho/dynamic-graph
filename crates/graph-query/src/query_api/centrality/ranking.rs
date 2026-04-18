use graph_core::LocusId;
use graph_world::World;

use crate::{
    all_betweenness, all_closeness, all_constraints, pagerank,
    query_api::{Query, QueryResult},
};

pub(super) fn execute_ranking_query(world: &World, query: &Query) -> Option<QueryResult> {
    match query {
        Query::PageRank {
            damping,
            iterations,
            tolerance,
            limit,
        } => Some(QueryResult::LocusScores(rank_scores(
            pagerank(world, *damping, *iterations, *tolerance),
            *limit,
        ))),
        Query::AllBetweenness { limit } => Some(QueryResult::LocusScores(rank_scores(
            all_betweenness(world).into_iter().collect(),
            *limit,
        ))),
        Query::AllCloseness { limit } => Some(QueryResult::LocusScores(rank_scores(
            all_closeness(world).into_iter().collect(),
            *limit,
        ))),
        Query::AllConstraints { limit } => Some(QueryResult::LocusScores(rank_scores(
            all_constraints(world).into_iter().collect(),
            *limit,
        ))),
        _ => None,
    }
}

fn rank_scores(mut scores: Vec<(LocusId, f32)>, limit: Option<usize>) -> Vec<(LocusId, f32)> {
    scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
    if let Some(n) = limit {
        scores.truncate(n);
    }
    scores
}
