use graph_core::{BatchId, Change, ChangeSubject, LocusId, RelationshipId};
use graph_world::World;
use rustc_hash::FxHashSet;

pub fn committed_batches(world: &World) -> Vec<BatchId> {
    world.log().committed_batch_ids()
}

pub fn loci_changed_in_batch(world: &World, batch: BatchId) -> Vec<LocusId> {
    let mut seen = FxHashSet::default();
    world
        .log()
        .batch(batch)
        .filter_map(|change| match change.subject {
            ChangeSubject::Locus(id) if seen.insert(id) => Some(id),
            _ => None,
        })
        .collect()
}

pub fn relationships_changed_in_batch(world: &World, batch: BatchId) -> Vec<RelationshipId> {
    let mut seen = FxHashSet::default();
    world
        .log()
        .batch(batch)
        .filter_map(|change| match change.subject {
            ChangeSubject::Relationship(id) if seen.insert(id) => Some(id),
            _ => None,
        })
        .collect()
}

pub fn changes_to_locus_in_range(
    world: &World,
    locus: LocusId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<&Change> {
    world
        .changes_to_locus(locus)
        .filter(|change| change.batch.0 >= from_batch.0 && change.batch.0 <= to_batch.0)
        .collect()
}

pub fn changes_to_relationship_in_range(
    world: &World,
    rel: RelationshipId,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<&Change> {
    world
        .changes_to_relationship(rel)
        .filter(|change| change.batch.0 >= from_batch.0 && change.batch.0 <= to_batch.0)
        .collect()
}
