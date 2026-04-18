use graph_core::{Endpoints, LocusId, RelationshipId};
use rustc_hash::FxHashMap;

pub(super) fn index_relationship_loci(
    by_locus: &mut FxHashMap<LocusId, Vec<RelationshipId>>,
    id: RelationshipId,
    endpoints: &Endpoints,
) {
    locus_ids_of(endpoints, |locus| {
        by_locus.entry(locus).or_default().push(id);
    });
}

pub(super) fn deindex_relationship_loci(
    by_locus: &mut FxHashMap<LocusId, Vec<RelationshipId>>,
    id: RelationshipId,
    endpoints: &Endpoints,
) {
    locus_ids_of(endpoints, |locus| {
        if let Some(ids) = by_locus.get_mut(&locus) {
            ids.retain(|&rid| rid != id);
        }
    });
}

fn locus_ids_of(endpoints: &Endpoints, mut f: impl FnMut(LocusId)) {
    match endpoints {
        Endpoints::Directed { from, to } => {
            f(*from);
            if from != to {
                f(*to);
            }
        }
        Endpoints::Symmetric { a, b } => {
            f(*a);
            if a != b {
                f(*b);
            }
        }
    }
}
