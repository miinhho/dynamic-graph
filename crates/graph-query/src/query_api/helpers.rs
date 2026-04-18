use graph_core::{Cohere, CohereMembers, Endpoints, EntityId, Relationship, RelationshipId};

use super::{CohereResult, RelationshipSummary};

pub(crate) fn rel_to_summary(r: &Relationship) -> RelationshipSummary {
    let (from, to, directed) = match r.endpoints {
        Endpoints::Directed { from, to } => (from, to, true),
        Endpoints::Symmetric { a, b } => (a, b, false),
    };
    RelationshipSummary {
        id: r.id,
        kind: r.kind,
        from,
        to,
        directed,
        activity: r.activity(),
        weight: r.weight(),
        change_count: r.lineage.change_count,
        created_batch: r.created_batch,
    }
}

pub(crate) fn coheres_to_results(coheres: &[Cohere]) -> Vec<CohereResult> {
    coheres.iter().map(cohere_to_result).collect()
}

fn cohere_to_result(cohere: &Cohere) -> CohereResult {
    let (entity_ids, relationship_ids) = cohere_member_ids(&cohere.members);
    CohereResult {
        id: cohere.id,
        entity_ids,
        relationship_ids,
        strength: cohere.strength,
    }
}

fn cohere_member_ids(members: &CohereMembers) -> (Vec<EntityId>, Vec<RelationshipId>) {
    match members {
        CohereMembers::Entities(ids) => (ids.clone(), vec![]),
        CohereMembers::Relationships(ids) => (vec![], ids.clone()),
        CohereMembers::Mixed {
            entities,
            relationships,
        } => (entities.clone(), relationships.clone()),
    }
}
