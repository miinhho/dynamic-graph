use graph_core::{LocusId, RelationshipKindId};
use graph_world::World;

use super::super::neighbors::neighbors;

pub fn hub_loci(world: &World, min_degree: usize) -> Vec<LocusId> {
    world
        .relationships()
        .degree_iter()
        .filter(|(_, degree)| *degree >= min_degree)
        .map(|(locus, _)| locus)
        .collect()
}

pub fn neighbors_of(world: &World, locus: LocusId) -> Vec<LocusId> {
    neighbors(world, locus, None).collect()
}

pub fn neighbors_of_kind(world: &World, locus: LocusId, kind: RelationshipKindId) -> Vec<LocusId> {
    neighbors(world, locus, Some(kind)).collect()
}

pub fn isolated_loci(world: &World) -> Vec<LocusId> {
    world
        .loci()
        .iter()
        .filter(|locus| world.relationships().degree(locus.id) == 0)
        .map(|locus| locus.id)
        .collect()
}

pub fn source_loci(world: &World) -> Vec<LocusId> {
    world
        .loci()
        .iter()
        .filter(|locus| world.in_degree(locus.id) == 0 && world.out_degree(locus.id) > 0)
        .map(|locus| locus.id)
        .collect()
}

pub fn sink_loci(world: &World) -> Vec<LocusId> {
    world
        .loci()
        .iter()
        .filter(|locus| world.out_degree(locus.id) == 0 && world.in_degree(locus.id) > 0)
        .map(|locus| locus.id)
        .collect()
}
