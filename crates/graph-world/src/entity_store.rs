//! Store for emergent entities (Layer 3).
//!
//! Like `RelationshipStore`, this is an indexed container — the
//! emergence logic (in the engine) owns the *decision* of when to mint
//! or update; the store only handles persistence and lookup.
//!
//! Entities are never deleted per `docs/redesign.md` §3.4. A call to
//! mark an entity dormant flips its `status` field; the record remains
//! in the store indefinitely.

use std::collections::HashMap;

use graph_core::{Entity, EntityId, EntityStatus};

#[derive(Debug, Default, Clone)]
pub struct EntityStore {
    by_id: HashMap<EntityId, Entity>,
    next_id: u64,
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

    /// Insert a freshly born entity. Panics on duplicate id.
    pub fn insert(&mut self, entity: Entity) {
        let id = entity.id;
        if self.by_id.insert(id, entity).is_some() {
            panic!("EntityStore: duplicate id {id:?}");
        }
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.by_id.get(&id)
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.by_id.get_mut(&id)
    }

    /// Iterate all entities (active and dormant).
    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.by_id.values()
    }

    /// Iterate only active entities.
    pub fn active(&self) -> impl Iterator<Item = &Entity> {
        self.by_id.values().filter(|e| e.status == EntityStatus::Active)
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn active_count(&self) -> usize {
        self.by_id.values().filter(|e| e.status == EntityStatus::Active).count()
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
