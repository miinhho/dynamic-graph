use graph_core::{LocusId, Relationship, RelationshipId, RelationshipKindId};
use rustc_hash::FxHashSet;

use super::super::World;

pub(super) fn relationships_for_locus_of_kind(
    world: &World,
    locus: LocusId,
    kind: RelationshipKindId,
) -> impl Iterator<Item = &Relationship> {
    world
        .relationships
        .relationships_for_locus(locus)
        .filter(move |r| r.kind == kind)
}

pub(super) fn relationships_from_of_kind(
    world: &World,
    locus: LocusId,
    kind: RelationshipKindId,
) -> impl Iterator<Item = &Relationship> {
    world
        .relationships
        .relationships_from(locus)
        .filter(move |r| r.kind == kind)
}

pub(super) fn relationships_to_of_kind(
    world: &World,
    locus: LocusId,
    kind: RelationshipKindId,
) -> impl Iterator<Item = &Relationship> {
    world
        .relationships
        .relationships_to(locus)
        .filter(move |r| r.kind == kind)
}

pub(super) fn relationships_between_of_kind(
    world: &World,
    a: LocusId,
    b: LocusId,
    kind: RelationshipKindId,
) -> impl Iterator<Item = &Relationship> {
    world
        .relationships
        .relationships_between(a, b)
        .filter(move |r| r.kind == kind)
}

pub(super) fn relationships_active_above(
    world: &World,
    threshold: f32,
) -> impl Iterator<Item = &Relationship> {
    world
        .relationships
        .iter()
        // Magnitude comparison — Phase 1 signed activity: a strongly inhibitory
        // edge is "active" in the structural sense even though its sign is negative.
        .filter(move |relationship| relationship.activity().abs() > threshold)
}

pub(super) fn induced_subgraph<'a>(world: &'a World, loci: &[LocusId]) -> Vec<&'a Relationship> {
    let loci_set: FxHashSet<LocusId> = loci.iter().copied().collect();
    let mut seen = FxHashSet::<RelationshipId>::default();
    let mut result = Vec::new();

    for &locus in loci {
        for relationship in world.relationships.relationships_for_locus(locus) {
            if seen.insert(relationship.id) && relationship.endpoints.all_endpoints_in(&loci_set) {
                result.push(relationship);
            }
        }
    }

    result
}
