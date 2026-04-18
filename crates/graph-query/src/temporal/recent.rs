use graph_core::{Change, LocusId, RelationshipId};
use graph_world::World;

/// The last `n` changes committed to `locus`, newest first.
///
/// More ergonomic than `changes_to_locus_in_range` when you only want the most
/// recent N entries and don't know the batch range in advance.
pub fn last_n_changes_to_locus(world: &World, locus: LocusId, n: usize) -> Vec<&Change> {
    world.changes_to_locus(locus).take(n).collect()
}

/// The last `n` changes committed to `rel`, newest first.
///
/// Only `ChangeSubject::Relationship` entries are returned — auto-emerge
/// touches are not recorded as relationship-subject changes.
pub fn last_n_changes_to_relationship(
    world: &World,
    rel: RelationshipId,
    n: usize,
) -> Vec<&Change> {
    world.changes_to_relationship(rel).take(n).collect()
}
