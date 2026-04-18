use std::collections::HashSet;

use graph_core::{BatchId, Change, LocusId};
use graph_world::World;

use crate::tables::{
    BCM_THRESHOLDS, CHANGES, CHANGES_BY_BATCH, ENTITIES, LOCI, NAMES, PROPERTIES, REL_BY_LOCUS,
    RELATIONSHIPS, SUBSCRIPTIONS,
};
use crate::util::{
    collect_touched_locus_ids, entity_touched_in_batch, insert_rel_by_locus,
    relationship_touched_in_batch, remove_all_multimap_rows, write_postcard_row,
};
use crate::{Storage, StorageError};

struct BatchCommit<'a> {
    storage: &'a Storage,
    txn: &'a redb::WriteTransaction,
    world: &'a World,
    committed_batch: BatchId,
    touched_locus_ids: &'a HashSet<LocusId>,
}

impl Storage {
    pub fn commit_batch(
        &self,
        world: &World,
        committed_batch: BatchId,
    ) -> Result<(), StorageError> {
        let changes: Vec<_> = world.log().batch(committed_batch).cloned().collect();
        if changes.is_empty() {
            return Ok(());
        }
        let touched_locus_ids = collect_touched_locus_ids(&changes);

        let txn = self.db.begin_write()?;
        {
            self.write_batch_changes(&txn, world, committed_batch, &changes, &touched_locus_ids)?;
        }
        txn.commit()?;
        Ok(())
    }

    fn write_batch_changes(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
        committed_batch: BatchId,
        changes: &[Change],
        touched_locus_ids: &HashSet<LocusId>,
    ) -> Result<(), StorageError> {
        BatchCommit {
            storage: self,
            txn,
            world,
            committed_batch,
            touched_locus_ids,
        }
        .write(changes)
    }

    fn write_changes_for_batch(
        &self,
        txn: &redb::WriteTransaction,
        changes: &[Change],
    ) -> Result<(), StorageError> {
        let mut changes_t = txn.open_table(CHANGES)?;
        let mut batch_idx = txn.open_multimap_table(CHANGES_BY_BATCH)?;
        for change in changes {
            write_postcard_row(&mut changes_t, change.id.0, change)?;
            batch_idx.insert(change.batch.0, change.id.0)?;
        }
        Ok(())
    }

    fn upsert_touched_loci(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
        touched_locus_ids: &HashSet<LocusId>,
    ) -> Result<(), StorageError> {
        let mut table = txn.open_table(LOCI)?;
        for &id in touched_locus_ids {
            if let Some(locus) = world.locus(id) {
                write_postcard_row(&mut table, id.0, locus)?;
            }
        }
        Ok(())
    }

    fn upsert_batch_relationships(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
        committed_batch: BatchId,
    ) -> Result<(), StorageError> {
        let mut table = txn.open_table(RELATIONSHIPS)?;
        let mut index = txn.open_multimap_table(REL_BY_LOCUS)?;
        for rel in world.relationships().iter() {
            if relationship_touched_in_batch(world, rel, committed_batch) {
                write_postcard_row(&mut table, rel.id.0, rel)?;
                insert_rel_by_locus(&mut index, rel)?;
            }
        }
        self.last_relationship_gen
            .set(world.relationships().generation());
        Ok(())
    }

    fn upsert_batch_entities(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
        committed_batch: BatchId,
    ) -> Result<(), StorageError> {
        let mut table = txn.open_table(ENTITIES)?;
        for entity in world.entities().iter() {
            if entity_touched_in_batch(entity, committed_batch) {
                write_postcard_row(&mut table, entity.id.0, entity)?;
            }
        }
        self.last_entity_gen.set(world.entities().generation());
        Ok(())
    }

    fn upsert_touched_properties(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
        touched_locus_ids: &HashSet<LocusId>,
    ) -> Result<(), StorageError> {
        let mut table = txn.open_table(PROPERTIES)?;
        for &id in touched_locus_ids {
            if let Some(props) = world.properties().get(id) {
                write_postcard_row(&mut table, id.0, props)?;
            }
        }
        Ok(())
    }

    fn upsert_touched_names(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
        touched_locus_ids: &HashSet<LocusId>,
    ) -> Result<(), StorageError> {
        let mut table = txn.open_table(NAMES)?;
        for &id in touched_locus_ids {
            if let Some(name) = world.names().name_of(id) {
                table.insert(name, id.0)?;
            }
        }
        Ok(())
    }

    fn upsert_touched_bcm_thresholds(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
        touched_locus_ids: &HashSet<LocusId>,
    ) -> Result<(), StorageError> {
        if world.bcm_thresholds().is_empty() {
            return Ok(());
        }

        let mut table = txn.open_table(BCM_THRESHOLDS)?;
        for &id in touched_locus_ids {
            if let Some(&theta) = world.bcm_thresholds().get(&id) {
                write_postcard_row(&mut table, id.0, &theta)?;
            }
        }
        Ok(())
    }

    fn rewrite_subscriptions_if_changed(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
    ) -> Result<(), StorageError> {
        let current_gen = world.subscriptions().generation();
        if current_gen == self.last_subscription_gen.get() {
            return Ok(());
        }

        let mut table = txn.open_multimap_table(SUBSCRIPTIONS)?;
        remove_all_multimap_rows(&mut table)?;
        for (rel_id, locus_id) in world.subscriptions().iter() {
            table.insert(locus_id.0, rel_id.0)?;
        }
        self.last_subscription_gen.set(current_gen);
        Ok(())
    }
}

impl<'a> BatchCommit<'a> {
    fn write(self, changes: &[Change]) -> Result<(), StorageError> {
        self.storage.write_changes_for_batch(self.txn, changes)?;
        self.storage
            .upsert_touched_loci(self.txn, self.world, self.touched_locus_ids)?;
        self.storage
            .upsert_batch_relationships(self.txn, self.world, self.committed_batch)?;
        self.storage
            .upsert_batch_entities(self.txn, self.world, self.committed_batch)?;
        self.storage
            .upsert_touched_properties(self.txn, self.world, self.touched_locus_ids)?;
        self.storage
            .upsert_touched_names(self.txn, self.world, self.touched_locus_ids)?;
        self.storage
            .upsert_touched_bcm_thresholds(self.txn, self.world, self.touched_locus_ids)?;
        self.storage
            .rewrite_subscriptions_if_changed(self.txn, self.world)?;
        self.storage.write_meta(self.txn, self.world)?;
        Ok(())
    }
}
