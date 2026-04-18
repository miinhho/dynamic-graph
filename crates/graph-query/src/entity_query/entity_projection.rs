use graph_core::{Entity, Locus, Relationship};
use graph_world::World;
use rustc_hash::FxHashSet;

use crate::query::{LociQuery, RelationshipsQuery};

pub(super) fn strongest_entity(candidates: Vec<&Entity>) -> Option<&Entity> {
    candidates.into_iter().max_by(|a, b| {
        a.current
            .coherence
            .partial_cmp(&b.current.coherence)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

pub(super) fn member_loci_query<'w>(
    world: &'w World,
    candidates: Vec<&'w Entity>,
) -> LociQuery<'w> {
    let mut seen = FxHashSet::default();
    let loci: Vec<&'w Locus> = candidates
        .into_iter()
        .flat_map(|entity| entity.current.members.iter().copied())
        .filter(|&id| seen.insert(id))
        .filter_map(|id| world.locus(id))
        .collect();
    LociQuery::from_candidates(world, loci)
}

pub(super) fn member_relationships_query<'w>(
    world: &'w World,
    candidates: Vec<&'w Entity>,
) -> RelationshipsQuery<'w> {
    let mut seen = FxHashSet::default();
    let relationships: Vec<&'w Relationship> = candidates
        .into_iter()
        .flat_map(|entity| entity.current.member_relationships.iter().copied())
        .filter(|&id| seen.insert(id))
        .filter_map(|id| world.relationships().get(id))
        .collect();
    RelationshipsQuery::from_candidates(world, relationships)
}
