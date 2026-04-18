use rustc_hash::FxHashSet;

use graph_core::RelationshipId;
use graph_schema::SchemaWorld;
use graph_world::World;

use crate::report::BoundaryEdge;

use super::{SignalMode, signal};

pub(super) struct BoundaryMatches {
    pub(super) confirmed: Vec<BoundaryEdge>,
    pub(super) ghost: Vec<BoundaryEdge>,
    pub(super) shadow: Vec<RelationshipId>,
}

pub(super) fn collect_boundary_matches(
    dynamic: &World,
    schema: &SchemaWorld,
    threshold: f32,
    mode: SignalMode,
) -> BoundaryMatches {
    let active_dynamic = collect_active_dynamic(dynamic, threshold, mode);
    let (confirmed, ghost, covered) = collect_declared_matches(dynamic, schema, &active_dynamic);
    let shadow = collect_shadow_relationships(&active_dynamic, &covered);
    BoundaryMatches {
        confirmed,
        ghost,
        shadow,
    }
}

fn collect_active_dynamic(
    dynamic: &World,
    threshold: f32,
    mode: SignalMode,
) -> FxHashSet<RelationshipId> {
    dynamic
        .relationships()
        .iter()
        .filter(|rel| signal(rel, mode) > threshold)
        .map(|rel| rel.id)
        .collect()
}

fn collect_declared_matches(
    dynamic: &World,
    schema: &SchemaWorld,
    active_dynamic: &FxHashSet<RelationshipId>,
) -> (
    Vec<BoundaryEdge>,
    Vec<BoundaryEdge>,
    FxHashSet<RelationshipId>,
) {
    let mut confirmed = Vec::new();
    let mut ghost = Vec::new();
    let mut covered = FxHashSet::default();

    for fact in schema.facts.active_facts() {
        let matching_rel = dynamic
            .relationships_between(fact.subject, fact.object)
            .find(|rel| active_dynamic.contains(&rel.id));

        let edge = BoundaryEdge {
            subject: fact.subject,
            predicate: fact.predicate.clone(),
            object: fact.object,
            dynamic_rel: matching_rel.map(|rel| rel.id),
        };

        if let Some(rel) = matching_rel {
            covered.insert(rel.id);
            confirmed.push(edge);
        } else {
            ghost.push(edge);
        }
    }

    (confirmed, ghost, covered)
}

fn collect_shadow_relationships(
    active_dynamic: &FxHashSet<RelationshipId>,
    covered: &FxHashSet<RelationshipId>,
) -> Vec<RelationshipId> {
    active_dynamic
        .iter()
        .filter(|id| !covered.contains(id))
        .copied()
        .collect()
}
