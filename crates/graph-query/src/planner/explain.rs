mod find;
mod generic;
mod generic_stages;

use graph_world::World;

use self::find::{explain_find_entities, explain_find_loci, explain_find_relationships};
use self::generic::explain_non_find_query;
use super::{Query, QueryPlan};

pub fn explain(world: &World, query: &Query) -> QueryPlan {
    match query {
        Query::FindRelationships {
            predicates,
            sort_by,
            limit,
        } => explain_find_relationships(world, predicates, sort_by.is_some(), *limit),
        Query::FindLoci {
            predicates,
            sort_by,
            limit,
        } => explain_find_loci(world, predicates, sort_by.is_some(), *limit),
        Query::FindEntities {
            predicates,
            sort_by,
            limit,
        } => explain_find_entities(world, predicates.len(), sort_by.is_some(), *limit),
        _ => explain_non_find_query(world, query),
    }
}
