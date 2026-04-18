//! Indexed store of emergent relationships.
//!
//! Two indices coexist:
//! - `by_id`: `RelationshipId -> Relationship` (the canonical record).
//! - `by_key`: `(EndpointKey, RelationshipKindId) -> RelationshipId` so
//!   the auto-emergence path can dedupe a hit in `O(1)` instead of
//!   walking the whole store.
//!
//! Relationships are minted lazily by the engine when cross-locus
//! causal flow is observed for the first time. The store does not
//! enforce who is allowed to insert; that policy lives in the engine.

mod indexing;
mod queries;

use graph_core::{EndpointKey, LocusId, Relationship, RelationshipId, RelationshipKindId};
use rustc_hash::FxHashMap;

use indexing::{deindex_relationship_loci, index_relationship_loci};

#[derive(Debug, Default, Clone)]
pub struct RelationshipStore {
    by_id: FxHashMap<RelationshipId, Relationship>,
    by_key: FxHashMap<(EndpointKey, RelationshipKindId), RelationshipId>,
    /// All relationship ids that involve a given locus (any endpoint position).
    by_locus: FxHashMap<LocusId, Vec<RelationshipId>>,
    next_id: u64,
    generation: u64,
}

impl RelationshipStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Pre-allocate capacity for `additional` more relationships.
    ///
    /// Call this before bulk inserts (e.g. the auto-emerge pass when many
    /// new relationships are expected) to avoid repeated HashMap rehashing.
    pub fn reserve(&mut self, additional: usize) {
        self.by_id.reserve(additional);
        self.by_key.reserve(additional);
        // by_locus gets at most 2*additional new entries (one per endpoint).
        self.by_locus
            .reserve(additional.saturating_mul(2).min(additional + 1024));
    }

    /// Mint a fresh `RelationshipId`. The engine assigns id and stores
    /// the relationship via `insert`; the two-step shape lets the
    /// caller fill in lineage data that depends on the new id.
    pub fn mint_id(&mut self) -> RelationshipId {
        let id = RelationshipId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Restore the counter after recovery. Must be called before any
    /// `mint_id()` calls to prevent ID collisions with persisted records.
    pub fn set_next_id(&mut self, next: u64) {
        self.next_id = next;
    }

    /// Insert a freshly minted relationship. Panics on duplicate id —
    /// duplicate insertion is a programming error, not a runtime case.
    pub fn insert(&mut self, relationship: Relationship) {
        let id = relationship.id;
        let key = (relationship.endpoints.key(), relationship.kind);
        index_relationship_loci(&mut self.by_locus, id, &relationship.endpoints);
        if self.by_id.insert(id, relationship).is_some() {
            panic!("RelationshipStore: duplicate id {id:?}");
        }
        self.by_key.insert(key, id);
        self.generation += 1;
    }

    pub fn get(&self, id: RelationshipId) -> Option<&Relationship> {
        self.by_id.get(&id)
    }

    pub fn get_mut(&mut self, id: RelationshipId) -> Option<&mut Relationship> {
        if self.by_id.contains_key(&id) {
            self.generation += 1;
        }
        self.by_id.get_mut(&id)
    }

    pub fn lookup(&self, key: &EndpointKey, kind: RelationshipKindId) -> Option<RelationshipId> {
        self.by_key.get(&(key.clone(), kind)).copied()
    }

    /// Remove a relationship by id. Returns the removed record, or `None`
    /// if the id was not found.
    ///
    /// All three indices (`by_id`, `by_key`, `by_locus`) are updated.
    /// After removal the id is dangling — do not re-insert with the same id.
    pub fn remove(&mut self, id: RelationshipId) -> Option<Relationship> {
        let rel = self.by_id.remove(&id)?;
        let key = (rel.endpoints.key(), rel.kind);
        self.by_key.remove(&key);
        deindex_relationship_loci(&mut self.by_locus, id, &rel.endpoints);
        self.generation += 1;
        Some(rel)
    }

    /// Monotonic generation counter — incremented on every mutation
    /// (insert, remove, or `get_mut` path). Used by `Storage::commit_batch`
    /// to skip the relationship table write when nothing changed.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn iter(&self) -> impl Iterator<Item = &Relationship> {
        self.by_id.values()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Relationship> {
        self.generation += 1;
        self.by_id.values_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Endpoints, InfluenceKindId, KindObservation, LocusId, RelationshipLineage, StateVector,
    };

    fn rel(id: RelationshipId, from: u64, to: u64, kind: u64) -> Relationship {
        Relationship {
            id,
            kind: InfluenceKindId(kind),
            endpoints: Endpoints::Directed {
                from: LocusId(from),
                to: LocusId(to),
            },
            state: StateVector::from_slice(&[0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 0,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(InfluenceKindId(
                    kind
                ))],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        }
    }

    #[test]
    fn insert_and_lookup_by_endpoint_kind() {
        let mut store = RelationshipStore::new();
        let id = store.mint_id();
        store.insert(rel(id, 1, 2, 7));
        let key = Endpoints::Directed {
            from: LocusId(1),
            to: LocusId(2),
        }
        .key();
        assert_eq!(store.lookup(&key, InfluenceKindId(7)), Some(id));
        assert_eq!(store.lookup(&key, InfluenceKindId(8)), None);
    }

    #[test]
    fn directed_endpoints_distinguish_direction() {
        // (1->2) and (2->1) of the same kind are *different* relationships.
        let mut store = RelationshipStore::new();
        let id_a = store.mint_id();
        store.insert(rel(id_a, 1, 2, 1));
        let id_b = store.mint_id();
        store.insert(rel(id_b, 2, 1, 1));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn symmetric_endpoints_dedupe_by_unordered_pair() {
        // The store *itself* doesn't dedupe — that's the engine's job.
        // But the EndpointKey for Symmetric{a,b} must equal the key for
        // Symmetric{b,a}, so two lookups land on the same record.
        let key_ab = Endpoints::Symmetric {
            a: LocusId(1),
            b: LocusId(2),
        }
        .key();
        let key_ba = Endpoints::Symmetric {
            a: LocusId(2),
            b: LocusId(1),
        }
        .key();
        assert_eq!(key_ab, key_ba);
    }

    #[test]
    fn relationships_for_locus_finds_both_directions() {
        let mut store = RelationshipStore::new();
        let id_ab = store.mint_id();
        store.insert(rel(id_ab, 1, 2, 1)); // 1→2
        let id_bc = store.mint_id();
        store.insert(rel(id_bc, 2, 3, 1)); // 2→3
        let id_cd = store.mint_id();
        store.insert(rel(id_cd, 3, 4, 1)); // 3→4

        let at_2: Vec<_> = store
            .relationships_for_locus(LocusId(2))
            .map(|r| r.id)
            .collect();
        assert_eq!(at_2.len(), 2);
        assert!(at_2.contains(&id_ab));
        assert!(at_2.contains(&id_bc));
    }

    #[test]
    fn relationships_from_and_to_are_directional() {
        let mut store = RelationshipStore::new();
        let id_ab = store.mint_id();
        store.insert(rel(id_ab, 1, 2, 1)); // 1→2
        let id_ba = store.mint_id();
        store.insert(rel(id_ba, 2, 1, 1)); // 2→1

        let from_1: Vec<_> = store.relationships_from(LocusId(1)).map(|r| r.id).collect();
        assert_eq!(from_1, vec![id_ab]);

        let to_1: Vec<_> = store.relationships_to(LocusId(1)).map(|r| r.id).collect();
        assert_eq!(to_1, vec![id_ba]);
    }

    #[test]
    fn relationships_between_matches_both_endpoint_orders() {
        let mut store = RelationshipStore::new();
        let id_ab = store.mint_id();
        store.insert(rel(id_ab, 1, 2, 1)); // 1→2
        let id_ba = store.mint_id();
        store.insert(rel(id_ba, 2, 1, 2)); // 2→1 (different kind)
        let id_ac = store.mint_id();
        store.insert(rel(id_ac, 1, 3, 1)); // 1→3 (unrelated)

        let between: Vec<_> = store
            .relationships_between(LocusId(1), LocusId(2))
            .map(|r| r.id)
            .collect();
        assert_eq!(between.len(), 2);
        assert!(between.contains(&id_ab));
        assert!(between.contains(&id_ba));
        assert!(!between.contains(&id_ac));
    }

    #[test]
    fn remove_cleans_locus_index() {
        let mut store = RelationshipStore::new();
        let id = store.mint_id();
        store.insert(rel(id, 1, 2, 1));
        store.remove(id);

        assert_eq!(store.relationships_for_locus(LocusId(1)).count(), 0);
        assert_eq!(store.relationships_for_locus(LocusId(2)).count(), 0);
    }
}
