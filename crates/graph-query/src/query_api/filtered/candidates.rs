use graph_core::{EndpointKey, Endpoints, LocusId, RelationshipId};
use graph_world::World;

pub(super) fn relationship_candidates(
    world: &World,
    seed: &Option<crate::planner::SeedKind>,
) -> Vec<RelationshipId> {
    use crate::planner::SeedKind;

    match seed {
        Some(SeedKind::DirectLookup { from, to, kind }) => {
            direct_lookup_candidates(world, *from, *to, *kind)
        }
        Some(SeedKind::Between { a, b }) => between_candidates(world, *a, *b),
        Some(SeedKind::From(locus)) => directional_candidates_from(world, *locus),
        Some(SeedKind::To(locus)) => directional_candidates_to(world, *locus),
        Some(SeedKind::Touching(locus)) => touching_candidates(world, *locus),
        None => world.relationships().iter().map(|r| r.id).collect(),
    }
}

fn direct_lookup_candidates(
    world: &World,
    from: LocusId,
    to: LocusId,
    kind: graph_core::InfluenceKindId,
) -> Vec<RelationshipId> {
    let key = EndpointKey::Directed(from, to);
    world
        .relationships()
        .lookup(&key, kind)
        .map(|id| vec![id])
        .unwrap_or_default()
}

fn between_candidates(world: &World, a: LocusId, b: LocusId) -> Vec<RelationshipId> {
    world.relationships_between(a, b).map(|r| r.id).collect()
}

fn directional_candidates_from(world: &World, locus: LocusId) -> Vec<RelationshipId> {
    world
        .relationships_for_locus(locus)
        .filter(|r| matches!(r.endpoints, Endpoints::Directed { from, .. } if from == locus))
        .map(|r| r.id)
        .collect()
}

fn directional_candidates_to(world: &World, locus: LocusId) -> Vec<RelationshipId> {
    world
        .relationships_for_locus(locus)
        .filter(|r| matches!(r.endpoints, Endpoints::Directed { to, .. } if to == locus))
        .map(|r| r.id)
        .collect()
}

fn touching_candidates(world: &World, locus: LocusId) -> Vec<RelationshipId> {
    world.relationships_for_locus(locus).map(|r| r.id).collect()
}
