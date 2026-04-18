use graph_world::World;

use crate::{
    louvain, louvain_with_resolution, modularity,
    query_api::{Query, QueryResult},
};

pub(super) fn execute_community_query(world: &World, query: &Query) -> Option<QueryResult> {
    match query {
        Query::Louvain => Some(QueryResult::Communities(louvain(world))),
        Query::LouvainWithResolution(resolution) => Some(QueryResult::Communities(
            louvain_with_resolution(world, *resolution),
        )),
        Query::Modularity => Some(QueryResult::Score(modularity_for_louvain(world))),
        _ => None,
    }
}

fn modularity_for_louvain(world: &World) -> f32 {
    let communities = louvain(world);
    modularity(world, &communities)
}
