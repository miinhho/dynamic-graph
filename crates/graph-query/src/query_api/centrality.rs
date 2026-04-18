mod community;
mod lookup;
mod ranking;

use graph_world::World;

use super::{Query, QueryResult};

pub(super) fn execute_centrality(world: &World, query: &Query) -> Option<QueryResult> {
    ranking::execute_ranking_query(world, query)
        .or_else(|| lookup::execute_lookup_query(world, query))
        .or_else(|| community::execute_community_query(world, query))
}
