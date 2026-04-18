use graph_core::{Entity, EntityId, EntityLayer, Locus, Relationship};

use super::super::World;

pub(super) fn entity_of(world: &World, locus: graph_core::LocusId) -> Option<&Entity> {
    world
        .entities
        .active()
        .find(|entity| entity.current.members.contains(&locus))
}

pub(super) fn entity_members(world: &World, id: EntityId) -> impl Iterator<Item = &Locus> {
    world
        .entities
        .get(id)
        .map(|entity| entity.current.members.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|&locus_id| world.loci.get(locus_id))
}

pub(super) fn entity_member_relationships(
    world: &World,
    id: EntityId,
) -> impl Iterator<Item = &Relationship> {
    world
        .entities
        .get(id)
        .map(|entity| entity.current.member_relationships.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|&relationship_id| world.relationships.get(relationship_id))
}

pub(super) fn entities_at_batch(
    world: &World,
    batch: graph_core::BatchId,
) -> Vec<(EntityId, &EntityLayer)> {
    world
        .entities
        .iter()
        .filter_map(|entity| {
            world
                .entities
                .layer_at_batch(entity.id, batch)
                .map(|layer| (entity.id, layer))
        })
        .collect()
}
