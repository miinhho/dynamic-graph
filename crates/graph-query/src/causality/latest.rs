use graph_core::{Change, LocusId, RelationshipId};
use graph_world::World;

pub fn last_change_to_locus(world: &World, locus: LocusId) -> Option<&Change> {
    world.changes_to_locus(locus).next()
}

pub fn last_change_to_relationship(world: &World, rel: RelationshipId) -> Option<&Change> {
    world.changes_to_relationship(rel).next()
}
