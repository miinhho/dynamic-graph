use graph_core::{EndpointKey, RelationshipId};
use graph_world::World;
use rustc_hash::FxHashSet;

pub fn reciprocal_of(world: &World, rel_id: RelationshipId) -> Option<RelationshipId> {
    let relationship = world.relationships().get(rel_id)?;
    match &relationship.endpoints {
        graph_core::Endpoints::Directed { from, to } => {
            let reverse_key = EndpointKey::Directed(*to, *from);
            world
                .relationships()
                .lookup(&reverse_key, relationship.kind)
        }
        graph_core::Endpoints::Symmetric { .. } => None,
    }
}

pub fn reciprocal_pairs(world: &World) -> Vec<(RelationshipId, RelationshipId)> {
    let mut seen = FxHashSet::default();
    let mut pairs = Vec::new();
    for relationship in world.relationships().iter() {
        if seen.contains(&relationship.id) {
            continue;
        }
        if let Some(reciprocal_id) = reciprocal_of(world, relationship.id) {
            seen.insert(relationship.id);
            seen.insert(reciprocal_id);
            pairs.push((relationship.id, reciprocal_id));
        }
    }
    pairs
}
