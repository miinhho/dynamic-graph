use super::LocusContext;
use crate::ids::{LocusId, RelationshipKindId};
use crate::relationship::Relationship;

pub(super) fn relationship_between(
    ctx: &(impl LocusContext + ?Sized),
    a: LocusId,
    b: LocusId,
) -> Option<&Relationship> {
    ctx.relationships_for(a).find(|r| r.endpoints.involves(b))
}

pub(super) fn relationships_between<'a>(
    ctx: &'a (impl LocusContext + ?Sized),
    a: LocusId,
    b: LocusId,
) -> Box<dyn Iterator<Item = &'a Relationship> + 'a> {
    Box::new(
        ctx.relationships_for(a)
            .filter(move |r| r.endpoints.involves(b)),
    )
}

pub(super) fn neighbor_ids(ctx: &(impl LocusContext + ?Sized), locus: LocusId) -> Vec<LocusId> {
    ctx.relationships_for(locus)
        .map(|r| r.endpoints.other_than(locus))
        .collect()
}

pub(super) fn neighbor_ids_of_kind(
    ctx: &(impl LocusContext + ?Sized),
    locus: LocusId,
    kind: RelationshipKindId,
) -> Vec<LocusId> {
    ctx.relationships_for(locus)
        .filter(move |r| r.kind == kind)
        .map(|r| r.endpoints.other_than(locus))
        .collect()
}

pub(super) fn relationship_between_kind(
    ctx: &(impl LocusContext + ?Sized),
    a: LocusId,
    b: LocusId,
    kind: RelationshipKindId,
) -> Option<&Relationship> {
    ctx.relationships_for(a)
        .find(|r| r.kind == kind && r.endpoints.involves(b))
}
