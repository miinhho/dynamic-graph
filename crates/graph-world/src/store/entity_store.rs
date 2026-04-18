//! Store for emergent entities (Layer 3).
//!
//! Like `RelationshipStore`, this is an indexed container — the
//! emergence logic (in the engine) owns the *decision* of when to mint
//! or update; the store only handles persistence and lookup.
//!
//! Entities are never deleted per `docs/redesign.md` §3.4. A call to
//! mark an entity dormant flips its `status` field; the record remains
//! in the store indefinitely.
//!
//! ## Member index
//!
//! `by_member` maps each `LocusId` to the set of entity IDs whose
//! `current.members` contains that locus.  It is maintained by all
//! mutation paths (`insert`, `update_snapshot`) and enables
//! `candidates_for_members` — an O(k) pre-filter used by the emergence
//! perspective's `all_matches` to avoid scanning every entity when
//! looking for overlap candidates.

mod indexing;
mod queries;

use graph_core::{Entity, EntityId, EntitySnapshot, EntityStatus, LocusId};
use rustc_hash::FxHashMap;

use indexing::{deindex_members, index_members};

#[derive(Debug, Default, Clone)]
pub struct EntityStore {
    by_id: FxHashMap<EntityId, Entity>,
    /// Locus → entities whose `current.members` contains that locus.
    /// Kept in sync by `insert` and `update_snapshot`.
    by_member: FxHashMap<LocusId, Vec<EntityId>>,
    next_id: u64,
    generation: u64,
}

impl EntityStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mint_id(&mut self) -> EntityId {
        let id = EntityId(self.next_id);
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

    /// Insert a freshly born entity. Panics on duplicate id.
    pub fn insert(&mut self, entity: Entity) {
        let id = entity.id;
        index_members(&mut self.by_member, id, &entity.current.members);
        if self.by_id.insert(id, entity).is_some() {
            panic!("EntityStore: duplicate id {id:?}");
        }
        self.generation += 1;
    }

    /// Update an entity's current snapshot and maintain the `by_member` index.
    ///
    /// Removes the entity from index entries for members it no longer has,
    /// then adds it to index entries for the new members.  The layer stack
    /// and other fields are unaffected; push the corresponding `EntityLayer`
    /// via `get_mut` after calling this.
    ///
    /// Does nothing when the entity ID is not found.
    pub fn update_snapshot(&mut self, id: EntityId, snapshot: EntitySnapshot) {
        let Some(entity) = self.by_id.get_mut(&id) else {
            return;
        };
        // Fast path: members unchanged — only update numeric fields, skip index churn.
        // The common case for CoherenceShift transitions is identical member sets.
        if entity.current.members == snapshot.members {
            entity.current = snapshot;
            self.generation += 1;
            return;
        }
        let old_members = std::mem::take(&mut entity.current.members);
        let new_members = snapshot.members.clone();
        entity.current = snapshot;
        let _ = entity;
        deindex_members(&mut self.by_member, id, &old_members);
        index_members(&mut self.by_member, id, &new_members);
        self.generation += 1;
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.by_id.get(&id)
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        if self.by_id.contains_key(&id) {
            self.generation += 1;
        }
        self.by_id.get_mut(&id)
    }

    /// Monotonic generation counter — incremented on every mutation.
    /// Used by `Storage::commit_batch` to skip entity table writes when
    /// no entity changed.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Iterate all entities (active and dormant).
    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.by_id.values()
    }

    /// Mutable iterator over all entities. Used by the weathering pass.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Entity> {
        self.generation += 1;
        self.by_id.values_mut()
    }

    /// Iterate only active entities.
    pub fn active(&self) -> impl Iterator<Item = &Entity> {
        self.by_id
            .values()
            .filter(|e| e.status == EntityStatus::Active)
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn active_count(&self) -> usize {
        self.by_id
            .values()
            .filter(|e| e.status == EntityStatus::Active)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{BatchId, EntitySnapshot, EntityStatus, LayerTransition, LocusId};

    fn born(id: EntityId) -> Entity {
        let snapshot = EntitySnapshot {
            members: vec![LocusId(id.0)],
            member_relationships: vec![],
            coherence: 1.0,
        };
        Entity::born(id, BatchId(0), snapshot)
    }

    #[test]
    fn insert_and_get() {
        let mut store = EntityStore::new();
        let id = store.mint_id();
        store.insert(born(id));
        assert!(store.get(id).is_some());
        assert_eq!(store.len(), 1);
        assert_eq!(store.active_count(), 1);
    }

    #[test]
    #[should_panic(expected = "duplicate id")]
    fn duplicate_insert_panics() {
        let mut store = EntityStore::new();
        let id = store.mint_id();
        store.insert(born(id));
        store.insert(born(id));
    }

    #[test]
    fn layer_at_batch_returns_correct_snapshot() {
        let mut store = EntityStore::new();
        let id = store.mint_id();
        store.insert(born(id));
        // batch 0: Born layer
        // deposit a second layer at batch 5
        let snap2 = EntitySnapshot {
            members: vec![LocusId(id.0), LocusId(99)],
            member_relationships: vec![],
            coherence: 0.5,
        };
        store.get_mut(id).unwrap().deposit(
            BatchId(5),
            snap2,
            LayerTransition::MembershipDelta {
                added: vec![LocusId(99)],
                removed: vec![],
            },
        );
        // query before second layer
        let layer = store.layer_at_batch(id, BatchId(3)).unwrap();
        assert_eq!(layer.batch, BatchId(0));
        // query at or after second layer
        let layer = store.layer_at_batch(id, BatchId(5)).unwrap();
        assert_eq!(layer.batch, BatchId(5));
        // query before entity existed
        assert!(store.layer_at_batch(id, BatchId(0)).is_some());
    }

    #[test]
    fn dormant_entity_excluded_from_active_iterator() {
        let mut store = EntityStore::new();
        let id = store.mint_id();
        store.insert(born(id));
        let entity = store.get_mut(id).unwrap();
        entity.status = EntityStatus::Dormant;
        entity.deposit(
            BatchId(1),
            EntitySnapshot::empty(),
            LayerTransition::BecameDormant,
        );
        assert_eq!(store.len(), 1);
        assert_eq!(store.active_count(), 0);
    }
}
