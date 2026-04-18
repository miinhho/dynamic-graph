use graph_core::LocusId;
use graph_world::World;

use super::{Query, QueryResult};

pub(super) fn execute_centrality(world: &World, query: &Query) -> Option<QueryResult> {
    use crate::*;

    match query {
        Query::PageRank {
            damping,
            iterations,
            tolerance,
            limit,
        } => {
            let mut scores = pagerank(world, *damping, *iterations, *tolerance);
            scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
            if let Some(n) = limit {
                scores.truncate(*n);
            }
            Some(QueryResult::LocusScores(scores))
        }
        Query::PageRankFor {
            locus,
            damping,
            iterations,
            tolerance,
        } => {
            let scores = pagerank(world, *damping, *iterations, *tolerance);
            let map: rustc_hash::FxHashMap<LocusId, f32> = scores.into_iter().collect();
            Some(QueryResult::MaybeScore(map.get(locus).copied()))
        }
        Query::AllBetweenness { limit } => {
            let mut scores: Vec<(LocusId, f32)> = all_betweenness(world).into_iter().collect();
            scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
            if let Some(n) = limit {
                scores.truncate(*n);
            }
            Some(QueryResult::LocusScores(scores))
        }
        Query::BetweennessFor(locus) => {
            Some(QueryResult::Score(betweenness_centrality(world, *locus)))
        }
        Query::AllCloseness { limit } => {
            let mut scores: Vec<(LocusId, f32)> = all_closeness(world).into_iter().collect();
            scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
            if let Some(n) = limit {
                scores.truncate(*n);
            }
            Some(QueryResult::LocusScores(scores))
        }
        Query::ClosenessFor(locus) => {
            Some(QueryResult::MaybeScore(closeness_centrality(world, *locus)))
        }
        Query::AllConstraints { limit } => {
            let mut scores: Vec<(LocusId, f32)> = all_constraints(world).into_iter().collect();
            scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
            if let Some(n) = limit {
                scores.truncate(*n);
            }
            Some(QueryResult::LocusScores(scores))
        }
        Query::ConstraintFor(locus) => Some(QueryResult::MaybeScore(structural_constraint(
            world, *locus,
        ))),
        Query::Louvain => Some(QueryResult::Communities(louvain(world))),
        Query::LouvainWithResolution(resolution) => Some(QueryResult::Communities(
            louvain_with_resolution(world, *resolution),
        )),
        Query::Modularity => {
            let communities = louvain(world);
            Some(QueryResult::Score(modularity(world, &communities)))
        }
        _ => None,
    }
}
