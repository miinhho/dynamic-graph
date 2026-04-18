use graph_core::{Endpoints, LocusId, Relationship};

use super::RelationshipStore;

impl RelationshipStore {
    pub fn degree(&self, locus: LocusId) -> usize {
        self.by_locus.get(&locus).map(Vec::len).unwrap_or(0)
    }

    pub fn in_degree(&self, locus: LocusId) -> usize {
        self.relationships_to(locus).count()
    }

    pub fn out_degree(&self, locus: LocusId) -> usize {
        self.relationships_from(locus).count()
    }

    pub fn degree_iter(&self) -> impl Iterator<Item = (LocusId, usize)> + '_ {
        self.by_locus.iter().map(|(&locus, ids)| (locus, ids.len()))
    }

    pub fn relationships_for_locus(&self, locus: LocusId) -> impl Iterator<Item = &Relationship> {
        self.by_locus
            .get(&locus)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|id| self.by_id.get(id))
    }

    pub fn relationships_from(&self, locus: LocusId) -> impl Iterator<Item = &Relationship> {
        self.relationships_for_locus(locus).filter(
            move |r| matches!(&r.endpoints, Endpoints::Directed { from, .. } if *from == locus),
        )
    }

    pub fn relationships_to(&self, locus: LocusId) -> impl Iterator<Item = &Relationship> {
        self.relationships_for_locus(locus)
            .filter(move |r| matches!(&r.endpoints, Endpoints::Directed { to, .. } if *to == locus))
    }

    pub fn relationships_between(
        &self,
        a: LocusId,
        b: LocusId,
    ) -> impl Iterator<Item = &Relationship> {
        self.relationships_for_locus(a)
            .filter(move |r| r.endpoints.involves(b))
    }
}
