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

use graph_core::{EndpointKey, Relationship, RelationshipId, RelationshipKindId};
use rustc_hash::FxHashMap;

#[derive(Debug, Default, Clone)]
pub struct RelationshipStore {
    by_id: FxHashMap<RelationshipId, Relationship>,
    by_key: FxHashMap<(EndpointKey, RelationshipKindId), RelationshipId>,
    next_id: u64,
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
        if self.by_id.insert(id, relationship).is_some() {
            panic!("RelationshipStore: duplicate id {id:?}");
        }
        self.by_key.insert(key, id);
    }

    pub fn get(&self, id: RelationshipId) -> Option<&Relationship> {
        self.by_id.get(&id)
    }

    pub fn get_mut(&mut self, id: RelationshipId) -> Option<&mut Relationship> {
        self.by_id.get_mut(&id)
    }

    pub fn lookup(
        &self,
        key: &EndpointKey,
        kind: RelationshipKindId,
    ) -> Option<RelationshipId> {
        self.by_key.get(&(key.clone(), kind)).copied()
    }

    /// Remove a relationship by id. Returns the removed record, or `None`
    /// if the id was not found.
    ///
    /// Both indices (`by_id` and `by_key`) are updated. After removal the
    /// id is dangling — do not re-insert with the same id.
    pub fn remove(&mut self, id: RelationshipId) -> Option<Relationship> {
        let rel = self.by_id.remove(&id)?;
        let key = (rel.endpoints.key(), rel.kind);
        self.by_key.remove(&key);
        Some(rel)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Relationship> {
        self.by_id.values()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Relationship> {
        self.by_id.values_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Endpoints, InfluenceKindId, LocusId, RelationshipLineage, StateVector,
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
                kinds_observed: vec![InfluenceKindId(kind)],
            },
            last_decayed_batch: 0,
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
}
