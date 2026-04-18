use crate::ids::{LocusId, RelationshipKindId};
use crate::relationship::{Endpoints, Relationship};

pub(super) fn relationship_has_kind(rel: &Relationship, kind: RelationshipKindId) -> bool {
    rel.kind == kind
}

pub(super) fn is_incoming_relationship(rel: &Relationship, locus: LocusId) -> bool {
    match rel.endpoints {
        Endpoints::Directed { to, .. } => to == locus,
        Endpoints::Symmetric { .. } => true,
    }
}

pub(super) fn is_outgoing_relationship(rel: &Relationship, locus: LocusId) -> bool {
    match rel.endpoints {
        Endpoints::Directed { from, .. } => from == locus,
        Endpoints::Symmetric { .. } => true,
    }
}
