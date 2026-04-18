use graph_core::{Entity, Locus, LocusId};
use graph_world::World;
use rustc_hash::FxHashSet;

pub(super) fn entity_member_loci_inner<'w>(world: &'w World, entity: &Entity) -> Vec<&'w Locus> {
    entity
        .current
        .members
        .iter()
        .filter_map(|&id| world.locus(id))
        .collect()
}

pub(super) fn locus_entities_inner(world: &World, locus: LocusId) -> Vec<&Entity> {
    world
        .entities()
        .active()
        .filter(|entity| entity.current.members.contains(&locus))
        .collect()
}

pub(super) fn top_entity_members_inner(world: &World, n: usize) -> Vec<&Locus> {
    let mut entities: Vec<&Entity> = world.entities().active().collect();
    entities.sort_unstable_by(|a, b| {
        b.current
            .coherence
            .partial_cmp(&a.current.coherence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut seen = FxHashSet::default();
    let mut result = Vec::new();
    for entity in entities.into_iter().take(n) {
        for &locus_id in &entity.current.members {
            if seen.insert(locus_id)
                && let Some(locus) = world.locus(locus_id)
            {
                result.push(locus);
            }
        }
    }
    result
}
