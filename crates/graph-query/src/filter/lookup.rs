use graph_core::{Locus, LocusId, Relationship, RelationshipId};
use graph_world::World;

pub fn lookup_loci<'w>(world: &'w World, ids: &[LocusId]) -> Vec<&'w Locus> {
    ids.iter().filter_map(|&id| world.locus(id)).collect()
}

pub fn lookup_relationships<'w>(world: &'w World, ids: &[RelationshipId]) -> Vec<&'w Relationship> {
    ids.iter()
        .filter_map(|&id| world.relationships().get(id))
        .collect()
}
