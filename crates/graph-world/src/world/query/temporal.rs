use graph_core::{BatchId, ChangeSubject, LocusId, RelationshipId, StateVector};
use rustc_hash::FxHashSet;

use super::super::World;

pub(super) fn locus_state_at(
    world: &World,
    locus: LocusId,
    batch: BatchId,
) -> Option<&StateVector> {
    world
        .log
        .changes_to_locus(locus)
        .find(|change| change.batch.0 <= batch.0)
        .map(|change| &change.after)
}

pub(super) fn relationship_state_at(
    world: &World,
    rel: RelationshipId,
    batch: BatchId,
) -> Option<&StateVector> {
    world
        .log
        .changes_to_relationship(rel)
        .find(|change| change.batch.0 <= batch.0)
        .map(|change| &change.after)
}

pub(super) fn relationships_at_batch(world: &World, batch: BatchId) -> FxHashSet<RelationshipId> {
    let mut seen = FxHashSet::default();

    for change in world.log.iter() {
        if change.batch.0 > batch.0 {
            continue;
        }
        if let ChangeSubject::Relationship(rel_id) = change.subject {
            seen.insert(rel_id);
        }
    }

    for relationship in world.relationships.iter() {
        if relationship.lineage.created_by.is_some() {
            seen.insert(relationship.id);
        }
    }

    seen
}
