use graph_core::{Endpoints, Locus, LocusId, Relationship};
use graph_world::World;
use rustc_hash::FxHashSet;

use super::LociQuery;

pub(super) fn source_loci_query<'w>(
    world: &'w World,
    rels: Vec<&'w Relationship>,
) -> LociQuery<'w> {
    let mut seen = FxHashSet::default();
    let loci: Vec<&'w Locus> = rels
        .iter()
        .filter_map(|relationship| relationship.endpoints.source())
        .filter(|&id| seen.insert(id))
        .filter_map(|id| world.locus(id))
        .collect();
    LociQuery::from_candidates(world, loci)
}

pub(super) fn target_loci_query<'w>(
    world: &'w World,
    rels: Vec<&'w Relationship>,
) -> LociQuery<'w> {
    let mut seen = FxHashSet::default();
    let loci: Vec<&'w Locus> = rels
        .iter()
        .filter_map(|relationship| relationship.endpoints.target())
        .filter(|&id| seen.insert(id))
        .filter_map(|id| world.locus(id))
        .collect();
    LociQuery::from_candidates(world, loci)
}

pub(super) fn endpoint_loci_query<'w>(
    world: &'w World,
    rels: Vec<&'w Relationship>,
) -> LociQuery<'w> {
    let mut seen = FxHashSet::default();
    let ids: Vec<LocusId> = rels
        .iter()
        .flat_map(|relationship| match relationship.endpoints {
            Endpoints::Directed { from, to } => [from, to],
            Endpoints::Symmetric { a, b } => [a, b],
        })
        .filter(|&id| seen.insert(id))
        .collect();
    LociQuery::from_candidates(
        world,
        ids.into_iter().filter_map(|id| world.locus(id)).collect(),
    )
}
