use graph_core::LocusId;
use graph_world::World;
use rustc_hash::FxHashMap;

use crate::{
    betweenness_centrality, closeness_centrality, pagerank,
    query_api::{Query, QueryResult},
    structural_constraint,
};

pub(super) fn execute_lookup_query(world: &World, query: &Query) -> Option<QueryResult> {
    match query {
        Query::PageRankFor {
            locus,
            damping,
            iterations,
            tolerance,
        } => Some(QueryResult::MaybeScore(pagerank_for(
            world,
            *locus,
            *damping,
            *iterations,
            *tolerance,
        ))),
        Query::BetweennessFor(locus) => {
            Some(QueryResult::Score(betweenness_centrality(world, *locus)))
        }
        Query::ClosenessFor(locus) => {
            Some(QueryResult::MaybeScore(closeness_centrality(world, *locus)))
        }
        Query::ConstraintFor(locus) => Some(QueryResult::MaybeScore(structural_constraint(
            world, *locus,
        ))),
        _ => None,
    }
}

fn pagerank_for(
    world: &World,
    locus: LocusId,
    damping: f32,
    iterations: usize,
    tolerance: f32,
) -> Option<f32> {
    let score_map: FxHashMap<LocusId, f32> = pagerank(world, damping, iterations, tolerance)
        .into_iter()
        .collect();
    score_map.get(&locus).copied()
}
