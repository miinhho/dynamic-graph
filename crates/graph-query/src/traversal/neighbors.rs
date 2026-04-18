use graph_core::{Endpoints, LocusId, RelationshipKindId};
use graph_world::World;

pub(super) fn neighbors(
    world: &World,
    locus: LocusId,
    kind: Option<RelationshipKindId>,
) -> impl Iterator<Item = LocusId> + '_ {
    world
        .relationships_for_locus(locus)
        .filter(move |relationship| kind.is_none_or(|expected| relationship.kind == expected))
        .map(move |relationship| relationship.endpoints.other_than(locus))
}

pub(super) fn successors(
    world: &World,
    locus: LocusId,
    kind: Option<RelationshipKindId>,
) -> impl Iterator<Item = LocusId> + '_ {
    world
        .relationships_for_locus(locus)
        .filter(move |relationship| kind.is_none_or(|expected| relationship.kind == expected))
        .filter_map(move |relationship| match relationship.endpoints {
            Endpoints::Directed { from, to } if from == locus => Some(to),
            Endpoints::Directed { .. } => None,
            Endpoints::Symmetric { .. } => Some(relationship.endpoints.other_than(locus)),
        })
}

pub(super) fn predecessors(
    world: &World,
    locus: LocusId,
    kind: Option<RelationshipKindId>,
) -> impl Iterator<Item = LocusId> + '_ {
    world
        .relationships_for_locus(locus)
        .filter(move |relationship| kind.is_none_or(|expected| relationship.kind == expected))
        .filter_map(move |relationship| match relationship.endpoints {
            Endpoints::Directed { from, to } if to == locus => Some(from),
            Endpoints::Directed { .. } => None,
            Endpoints::Symmetric { .. } => Some(relationship.endpoints.other_than(locus)),
        })
}
