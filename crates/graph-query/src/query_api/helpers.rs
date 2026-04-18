use graph_core::{Cohere, CohereMembers, Endpoints, Relationship};

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
    coheres
        .iter()
        .map(|c| {
            let (entity_ids, relationship_ids) = match &c.members {
                CohereMembers::Entities(ids) => (ids.clone(), vec![]),
                CohereMembers::Relationships(ids) => (vec![], ids.clone()),
                CohereMembers::Mixed {
                    entities,
                    relationships,
                } => (entities.clone(), relationships.clone()),
            };
            CohereResult {
                id: c.id,
                entity_ids,
                relationship_ids,
                strength: c.strength,
            }
        })
        .collect()
}
