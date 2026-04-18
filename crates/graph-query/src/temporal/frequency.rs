use graph_core::{BatchId, ChangeSubject, LocusId, RelationshipId};
use graph_world::World;
use rustc_hash::FxHashMap;

pub fn changed_since(world: &World, since_batch: BatchId) -> Vec<(LocusId, usize)> {
    let mut counts: FxHashMap<LocusId, usize> = FxHashMap::default();
    for c in world.log().iter() {
        if c.batch >= since_batch
            && let ChangeSubject::Locus(id) = c.subject
        {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    sort_counts(counts)
}

pub fn loci_by_change_frequency(
    world: &World,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<(LocusId, usize)> {
    let mut counts: FxHashMap<LocusId, usize> = FxHashMap::default();
    for c in world.log().iter() {
        if c.batch >= from_batch
            && c.batch <= to_batch
            && let ChangeSubject::Locus(id) = c.subject
        {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    sort_counts(counts)
}

pub fn relationships_by_change_frequency(
    world: &World,
    from_batch: BatchId,
    to_batch: BatchId,
) -> Vec<(RelationshipId, usize)> {
    let mut counts: FxHashMap<RelationshipId, usize> = FxHashMap::default();
    for c in world.log().iter() {
        if c.batch >= from_batch
            && c.batch <= to_batch
            && let ChangeSubject::Relationship(id) = c.subject
        {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    sort_counts(counts)
}

fn sort_counts<T: Copy + Eq + std::hash::Hash>(counts: FxHashMap<T, usize>) -> Vec<(T, usize)> {
    let mut result: Vec<_> = counts.into_iter().collect();
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result
}
