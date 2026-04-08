//! The substrate's mutable container.
//!
//! `World` owns:
//! - the `LocusStore` (Layer 0 state),
//! - the `ChangeLog` (Layer 1 history),
//! - the monotonic `BatchId` clock,
//! - the next `ChangeId` counter the engine uses when minting changes.
//!
//! It does *not* own the kind registries (`LocusKindRegistry`,
//! `InfluenceKindRegistry`); those live next to the engine and are
//! threaded into ticks. Keeping the world free of program references
//! makes snapshots cheap and replay clean.

use graph_core::{BatchId, Change, ChangeId, EntityId, EntityLayer, Locus, LocusId, RelationshipId};

use crate::change_log::ChangeLog;
use crate::cohere_store::CohereStore;
use crate::entity_store::EntityStore;
use crate::locus_store::LocusStore;
use crate::relationship_store::RelationshipStore;

#[derive(Debug, Default, Clone)]
pub struct World {
    loci: LocusStore,
    relationships: RelationshipStore,
    entities: EntityStore,
    coheres: CohereStore,
    log: ChangeLog,
    current_batch: BatchId,
    next_change_id: u64,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_locus(&mut self, locus: Locus) {
        self.loci.insert(locus);
    }

    pub fn locus(&self, id: LocusId) -> Option<&Locus> {
        self.loci.get(id)
    }

    pub fn locus_mut(&mut self, id: LocusId) -> Option<&mut Locus> {
        self.loci.get_mut(id)
    }

    pub fn loci(&self) -> &LocusStore {
        &self.loci
    }

    pub fn relationships(&self) -> &RelationshipStore {
        &self.relationships
    }

    /// Engine-only mutable handle to the relationship store. Used by
    /// the auto-emergence path on commit.
    pub fn relationships_mut(&mut self) -> &mut RelationshipStore {
        &mut self.relationships
    }

    pub fn entities(&self) -> &EntityStore {
        &self.entities
    }

    pub fn entities_mut(&mut self) -> &mut EntityStore {
        &mut self.entities
    }

    pub fn coheres(&self) -> &CohereStore {
        &self.coheres
    }

    pub fn coheres_mut(&mut self) -> &mut CohereStore {
        &mut self.coheres
    }

    pub fn log(&self) -> &ChangeLog {
        &self.log
    }

    /// Mutable access to the change log. Used by the trim pass.
    pub fn log_mut(&mut self) -> &mut ChangeLog {
        &mut self.log
    }

    pub fn current_batch(&self) -> BatchId {
        self.current_batch
    }

    /// Mint the next `ChangeId`. Engine-only — exposed on `World` so the
    /// counter advances atomically with appends.
    pub fn mint_change_id(&mut self) -> ChangeId {
        let id = ChangeId(self.next_change_id);
        self.next_change_id += 1;
        id
    }

    /// Append a change to the log. The engine is expected to have set
    /// `change.batch` to `current_batch` already; this method does not
    /// re-check that.
    pub fn append_change(&mut self, change: Change) -> ChangeId {
        self.log.append(change)
    }

    /// Advance to the next batch. Called once per batch by the engine
    /// after all changes for the current batch have committed.
    pub fn advance_batch(&mut self) -> BatchId {
        self.current_batch = BatchId(self.current_batch.0 + 1);
        self.current_batch
    }

    // ── Query conveniences ────────────────────────────────────────────────

    /// Iterate changes to a locus, newest first. Delegates to
    /// `ChangeLog::changes_to_locus`; O(k) where k is the number of
    /// changes targeting this locus.
    pub fn changes_to_locus(&self, id: LocusId) -> impl Iterator<Item = &Change> {
        self.log.changes_to_locus(id)
    }

    /// Iterate changes to a relationship, newest first.
    pub fn changes_to_relationship(&self, id: RelationshipId) -> impl Iterator<Item = &Change> {
        self.log.changes_to_relationship(id)
    }

    /// Direct predecessor changes of `change_id`. Delegates to
    /// `ChangeLog::predecessors`.
    pub fn predecessors(&self, change_id: ChangeId) -> impl Iterator<Item = &Change> {
        self.log.predecessors(change_id)
    }

    /// All causal ancestors of `change_id` in BFS order. Delegates to
    /// `ChangeLog::causal_ancestors`.
    pub fn causal_ancestors(&self, change_id: ChangeId) -> Vec<&Change> {
        self.log.causal_ancestors(change_id)
    }

    /// Most recent entity layer at or before `batch`. Delegates to
    /// `EntityStore::layer_at_batch`.
    pub fn entity_at_batch(&self, id: EntityId, batch: BatchId) -> Option<&EntityLayer> {
        self.entities.layer_at_batch(id, batch)
    }

    /// Returns `true` if `ancestor` is a causal ancestor of `descendant`.
    /// Delegates to `ChangeLog::is_ancestor_of`.
    pub fn is_ancestor_of(&self, ancestor: ChangeId, descendant: ChangeId) -> bool {
        self.log.is_ancestor_of(ancestor, descendant)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{LocusKindId, StateVector};

    #[test]
    fn change_id_counter_is_monotonic() {
        let mut w = World::new();
        let a = w.mint_change_id();
        let b = w.mint_change_id();
        assert_eq!(a.0, 0);
        assert_eq!(b.0, 1);
    }

    #[test]
    fn advance_batch_increments() {
        let mut w = World::new();
        assert_eq!(w.current_batch(), BatchId(0));
        assert_eq!(w.advance_batch(), BatchId(1));
        assert_eq!(w.current_batch(), BatchId(1));
    }

    #[test]
    fn world_holds_loci() {
        let mut w = World::new();
        w.insert_locus(Locus::new(
            LocusId(42),
            LocusKindId(1),
            StateVector::zeros(2),
        ));
        assert!(w.locus(LocusId(42)).is_some());
    }
}
