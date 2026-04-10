//! Store for emergent entities (Layer 3).
//!
//! Like `RelationshipStore`, this is an indexed container — the
//! emergence logic (in the engine) owns the *decision* of when to mint
//! or update; the store only handles persistence and lookup.
//!
//! Entities are never deleted per `docs/redesign.md` §3.4. A call to
//! mark an entity dormant flips its `status` field; the record remains
//! in the store indefinitely.

use graph_core::{BatchId, Entity, EntityId, EntityLayer, EntityStatus};
use rustc_hash::FxHashMap;

#[derive(Debug, Default, Clone)]
pub struct EntityStore {
    by_id: FxHashMap<EntityId, Entity>,
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

    /// Mutable iterator over all entities. Used by the weathering pass.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Entity> {
        self.by_id.values_mut()
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

    /// Return the most recent layer deposited at or before `batch`, or `None`
    /// if the entity does not exist or had no layers by that point.
    pub fn layer_at_batch(&self, id: EntityId, batch: BatchId) -> Option<&EntityLayer> {
        let entity = self.get(id)?;
        // Layers are stored oldest-first (monotonically increasing batch).
        // partition_point gives the index of the first layer with batch > target,
        // so the layer immediately before that is the most recent one at or before target.
        let pos = entity.layers.partition_point(|l| l.batch <= batch);
        entity.layers.get(pos.wrapping_sub(1))
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
            LayerTransition::MembershipDelta { added: vec![LocusId(99)], removed: vec![] },
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
