use graph_world::World;

use super::{Query, QueryResult};

pub(super) fn execute_causal_log(world: &World, query: &Query) -> Option<QueryResult> {
    use crate::*;

    match query {
        Query::CausalAncestors(change_id) => {
            Some(QueryResult::Changes(causal_ancestors(world, *change_id)))
        }
        Query::CausalDescendants(change_id) => {
            Some(QueryResult::Changes(causal_descendants(world, *change_id)))
        }
        Query::CausalDepth(change_id) => Some(QueryResult::Count(causal_depth(world, *change_id))),
        Query::IsAncestorOf {
            ancestor,
            descendant,
        } => Some(QueryResult::Bool(is_ancestor_of(
            world,
            *ancestor,
            *descendant,
        ))),
        Query::RootStimuli(change_id) => {
            Some(QueryResult::Changes(root_stimuli(world, *change_id)))
        }
        Query::ChangesToLocusInRange { locus, from, to } => {
            let changes = changes_to_locus_in_range(world, *locus, *from, *to);
            Some(QueryResult::Changes(
                changes.into_iter().map(|change| change.id).collect(),
            ))
        }
        Query::ChangesToRelationshipInRange {
            relationship,
            from,
            to,
        } => {
            let changes = changes_to_relationship_in_range(world, *relationship, *from, *to);
            Some(QueryResult::Changes(
                changes.into_iter().map(|change| change.id).collect(),
            ))
        }
        Query::LociChangedInBatch(batch) => {
            Some(QueryResult::Loci(loci_changed_in_batch(world, *batch)))
        }
        Query::RelationshipsChangedInBatch(batch) => Some(QueryResult::Relationships(
            relationships_changed_in_batch(world, *batch),
        )),
        _ => None,
    }
}
