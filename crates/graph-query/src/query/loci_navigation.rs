use graph_core::{Endpoints, Locus, LocusId};
use graph_world::World;
use rustc_hash::FxHashSet;

use super::RelationshipsQuery;

pub(super) fn outgoing_relationships_query<'w>(
    world: &'w World,
    loci: Vec<&'w Locus>,
) -> RelationshipsQuery<'w> {
    let ids: FxHashSet<LocusId> = loci.iter().map(|locus| locus.id).collect();
    RelationshipsQuery {
        world,
        rels: world
            .relationships()
            .iter()
            .filter(
                |relationship| matches!(relationship.endpoints, Endpoints::Directed { from, .. } if ids.contains(&from)),
            )
            .collect(),
    }
}

pub(super) fn incoming_relationships_query<'w>(
    world: &'w World,
    loci: Vec<&'w Locus>,
) -> RelationshipsQuery<'w> {
    let ids: FxHashSet<LocusId> = loci.iter().map(|locus| locus.id).collect();
    RelationshipsQuery {
        world,
        rels: world
            .relationships()
            .iter()
            .filter(
                |relationship| matches!(relationship.endpoints, Endpoints::Directed { to, .. } if ids.contains(&to)),
            )
            .collect(),
    }
}

pub(super) fn touching_relationships_query<'w>(
    world: &'w World,
    loci: Vec<&'w Locus>,
) -> RelationshipsQuery<'w> {
    let ids: FxHashSet<LocusId> = loci.iter().map(|locus| locus.id).collect();
    RelationshipsQuery {
        world,
        rels: world
            .relationships()
            .iter()
            .filter(|relationship| ids.iter().any(|&id| relationship.endpoints.involves(id)))
            .collect(),
    }
}
