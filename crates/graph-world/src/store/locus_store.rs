//! In-memory store for locus state.
//!
//! Plain `HashMap<LocusId, Locus>` wrapper. The new substrate keeps the
//! locus surface area tiny — there are no channels, no cohort indices,
//! no precomputed adjacency. Adjacency is *derived* later from the
//! change log by the relationship layer (Layer 2). See
//! `docs/redesign.md` §3.3.

use rustc_hash::FxHashMap;

use graph_core::{Locus, LocusId};

#[derive(Debug, Default, Clone)]
pub struct LocusStore {
    loci: FxHashMap<LocusId, Locus>,
    next_id: u64,
}

impl LocusStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint the next `LocusId` from the auto-increment counter.
    /// Used by the ingest API; low-level callers may assign their own IDs.
    pub fn next_id(&self) -> LocusId {
        LocusId(self.next_id)
    }

    /// Insert a locus. Panics on duplicate id — that is a registration
    /// bug, not a runtime situation.
    pub fn insert(&mut self, locus: Locus) {
        let id = locus.id;
        // Keep the auto-increment counter ahead of any manually assigned ID.
        if id.0 >= self.next_id {
            self.next_id = id.0 + 1;
        }
        if self.loci.insert(id, locus).is_some() {
            panic!("LocusStore: duplicate locus {id:?}");
        }
    }

    pub fn get(&self, id: LocusId) -> Option<&Locus> {
        self.loci.get(&id)
    }

    pub fn get_mut(&mut self, id: LocusId) -> Option<&mut Locus> {
        self.loci.get_mut(&id)
    }

    /// Remove a locus by id. Returns the removed `Locus`, or `None` if not found.
    pub fn remove(&mut self, id: LocusId) -> Option<Locus> {
        self.loci.remove(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Locus> {
        self.loci.values()
    }

    pub fn len(&self) -> usize {
        self.loci.len()
    }

    pub fn is_empty(&self) -> bool {
        self.loci.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{LocusKindId, StateVector};

    fn locus(id: u64) -> Locus {
        Locus::new(LocusId(id), LocusKindId(1), StateVector::zeros(2))
    }

    #[test]
    fn insert_and_get() {
        let mut store = LocusStore::new();
        store.insert(locus(1));
        assert!(store.get(LocusId(1)).is_some());
        assert!(store.get(LocusId(2)).is_none());
    }

    #[test]
    #[should_panic(expected = "duplicate locus")]
    fn duplicate_insert_panics() {
        let mut store = LocusStore::new();
        store.insert(locus(1));
        store.insert(locus(1));
    }
}
