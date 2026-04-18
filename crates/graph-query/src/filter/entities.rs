use super::entity_bridge::{
    entity_member_loci_inner, locus_entities_inner, top_entity_members_inner,
};
use graph_core::{Entity, Locus, LocusId};
use graph_world::World;

pub fn active_entities(world: &World) -> Vec<&Entity> {
    world.entities().active().collect()
}

pub fn entities_with_member(world: &World, locus: LocusId) -> Vec<&Entity> {
    world
        .entities()
        .active()
        .filter(|e| e.current.members.contains(&locus))
        .collect()
}

pub fn entities_with_coherence<F>(world: &World, pred: F) -> Vec<&Entity>
where
    F: Fn(f32) -> bool,
{
    world
        .entities()
        .active()
        .filter(|e| pred(e.current.coherence))
        .collect()
}

pub fn entities_matching<F>(world: &World, pred: F) -> Vec<&Entity>
where
    F: Fn(&Entity) -> bool,
{
    world.entities().active().filter(|e| pred(e)).collect()
}

pub fn entity_member_loci<'w>(world: &'w World, entity: &Entity) -> Vec<&'w Locus> {
    entity_member_loci_inner(world, entity)
}

pub fn locus_entities(world: &World, locus: LocusId) -> Vec<&Entity> {
    locus_entities_inner(world, locus)
}

pub fn top_entity_members(world: &World, n: usize) -> Vec<&Locus> {
    top_entity_members_inner(world, n)
}
