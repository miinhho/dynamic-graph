use graph_core::BatchId;
use graph_core::{Change, Entity, Locus, LocusId, Properties, Relationship, RelationshipId};
use graph_world::{World, WorldMeta};
use redb::{ReadableMultimapTable, ReadableTable};

use crate::tables::{
    ALIASES, BCM_THRESHOLDS, CHANGES, CHANGES_BY_BATCH, ENTITIES, LOCI, META, META_CURRENT_BATCH,
    META_NEXT_CHANGE_ID, META_NEXT_ENTITY_ID, META_NEXT_RELATIONSHIP_ID, NAMES, PROPERTIES,
    REL_BY_LOCUS, RELATIONSHIPS, SUBSCRIPTIONS,
};
use crate::util::{
    for_each_table_value, insert_rel_by_locus, pop_all_rows, remove_all_multimap_rows,
    write_postcard_row,
};
use crate::{Storage, StorageError};

struct SnapshotWriter<'a> {
    storage: &'a Storage,
    txn: &'a redb::WriteTransaction,
    world: &'a World,
}

struct SnapshotLoader<'a> {
    storage: &'a Storage,
    txn: &'a redb::ReadTransaction,
    world: World,
}

impl Storage {
    pub fn save_world(&self, world: &World) -> Result<(), StorageError> {
        let txn = self.db.begin_write()?;
        {
            SnapshotWriter::new(self, &txn, world).write_all()?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn load_world(&self) -> Result<World, StorageError> {
        let txn = self.db.begin_read()?;
        SnapshotLoader::new(self, &txn).load()
    }

    pub(super) fn clear_world_tables(
        &self,
        txn: &redb::WriteTransaction,
    ) -> Result<(), StorageError> {
        pop_all_rows(&mut txn.open_table(LOCI)?)?;
        pop_all_rows(&mut txn.open_table(RELATIONSHIPS)?)?;
        pop_all_rows(&mut txn.open_table(ENTITIES)?)?;
        pop_all_rows(&mut txn.open_table(CHANGES)?)?;
        remove_all_multimap_rows(&mut txn.open_multimap_table(CHANGES_BY_BATCH)?)?;
        pop_all_rows(&mut txn.open_table(PROPERTIES)?)?;
        pop_all_rows(&mut txn.open_table(NAMES)?)?;
        pop_all_rows(&mut txn.open_table(ALIASES)?)?;
        remove_all_multimap_rows(&mut txn.open_multimap_table(SUBSCRIPTIONS)?)?;
        remove_all_multimap_rows(&mut txn.open_multimap_table(REL_BY_LOCUS)?)?;
        pop_all_rows(&mut txn.open_table(BCM_THRESHOLDS)?)?;
        Ok(())
    }

    fn write_all_loci(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
    ) -> Result<(), StorageError> {
        let mut table = txn.open_table(LOCI)?;
        for locus in world.loci().iter() {
            write_postcard_row(&mut table, locus.id.0, locus)?;
        }
        Ok(())
    }

    fn write_all_relationships(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
    ) -> Result<(), StorageError> {
        let mut table = txn.open_table(RELATIONSHIPS)?;
        let mut index = txn.open_multimap_table(REL_BY_LOCUS)?;
        for rel in world.relationships().iter() {
            write_postcard_row(&mut table, rel.id.0, rel)?;
            insert_rel_by_locus(&mut index, rel)?;
        }
        self.last_relationship_gen
            .set(world.relationships().generation());
        Ok(())
    }

    fn write_all_entities(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
    ) -> Result<(), StorageError> {
        let mut table = txn.open_table(ENTITIES)?;
        for entity in world.entities().iter() {
            write_postcard_row(&mut table, entity.id.0, entity)?;
        }
        self.last_entity_gen.set(world.entities().generation());
        Ok(())
    }

    fn write_all_changes(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
    ) -> Result<(), StorageError> {
        let mut changes = txn.open_table(CHANGES)?;
        let mut batch_index = txn.open_multimap_table(CHANGES_BY_BATCH)?;
        for change in world.log().iter() {
            write_postcard_row(&mut changes, change.id.0, change)?;
            batch_index.insert(change.batch.0, change.id.0)?;
        }
        Ok(())
    }

    fn write_all_properties(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
    ) -> Result<(), StorageError> {
        let mut table = txn.open_table(PROPERTIES)?;
        for (id, props) in world.properties().iter() {
            write_postcard_row(&mut table, id.0, props)?;
        }
        Ok(())
    }

    fn write_all_names(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
    ) -> Result<(), StorageError> {
        let mut names = txn.open_table(NAMES)?;
        let mut aliases = txn.open_table(ALIASES)?;
        for (name, id) in world.names().iter() {
            names.insert(name, id.0)?;
        }
        for (alias, id) in world.names().aliases() {
            aliases.insert(alias, id.0)?;
        }
        Ok(())
    }

    fn write_all_subscriptions(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
    ) -> Result<(), StorageError> {
        let mut table = txn.open_multimap_table(SUBSCRIPTIONS)?;
        for (rel_id, locus_id) in world.subscriptions().iter() {
            table.insert(locus_id.0, rel_id.0)?;
        }
        self.last_subscription_gen
            .set(world.subscriptions().generation());
        Ok(())
    }

    fn write_all_bcm_thresholds(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
    ) -> Result<(), StorageError> {
        let mut table = txn.open_table(BCM_THRESHOLDS)?;
        pop_all_rows(&mut table)?;
        for (&id, &theta) in world.bcm_thresholds() {
            write_postcard_row(&mut table, id.0, &theta)?;
        }
        Ok(())
    }

    pub(super) fn write_meta(
        &self,
        txn: &redb::WriteTransaction,
        world: &World,
    ) -> Result<(), StorageError> {
        let meta = world.world_meta();
        let mut table = txn.open_table(META)?;
        table.insert(META_CURRENT_BATCH, meta.current_batch.0)?;
        table.insert(META_NEXT_CHANGE_ID, meta.next_change_id)?;
        table.insert(META_NEXT_RELATIONSHIP_ID, meta.next_relationship_id)?;
        table.insert(META_NEXT_ENTITY_ID, meta.next_entity_id)?;
        Ok(())
    }

    fn load_meta(
        &self,
        txn: &redb::ReadTransaction,
        world: &mut World,
    ) -> Result<(), StorageError> {
        let table = txn.open_table(META)?;
        let batch = table.get(META_CURRENT_BATCH)?.ok_or(StorageError::Empty)?;
        let next_cid = table.get(META_NEXT_CHANGE_ID)?.ok_or(StorageError::Empty)?;
        let next_rid = table
            .get(META_NEXT_RELATIONSHIP_ID)?
            .ok_or(StorageError::Empty)?;
        let next_eid = table.get(META_NEXT_ENTITY_ID)?.ok_or(StorageError::Empty)?;
        let meta = WorldMeta {
            current_batch: BatchId(batch.value()),
            next_change_id: next_cid.value(),
            next_relationship_id: next_rid.value(),
            next_entity_id: next_eid.value(),
        };
        world.restore_meta(&meta);
        Ok(())
    }

    fn load_loci(
        &self,
        txn: &redb::ReadTransaction,
        world: &mut World,
    ) -> Result<(), StorageError> {
        let table = txn.open_table(LOCI)?;
        for_each_table_value(&table, |value| {
            let locus: Locus = postcard::from_bytes(value)?;
            world.insert_locus(locus);
            Ok(())
        })
    }

    fn load_relationships(
        &self,
        txn: &redb::ReadTransaction,
        world: &mut World,
    ) -> Result<(), StorageError> {
        let table = txn.open_table(RELATIONSHIPS)?;
        for_each_table_value(&table, |value| {
            let rel: Relationship = postcard::from_bytes(value)?;
            world.relationships_mut().insert(rel);
            Ok(())
        })
    }

    fn load_entities(
        &self,
        txn: &redb::ReadTransaction,
        world: &mut World,
    ) -> Result<(), StorageError> {
        let table = txn.open_table(ENTITIES)?;
        for_each_table_value(&table, |value| {
            let entity: Entity = postcard::from_bytes(value)?;
            world.entities_mut().insert(entity);
            Ok(())
        })
    }

    fn load_changes(
        &self,
        txn: &redb::ReadTransaction,
        world: &mut World,
    ) -> Result<(), StorageError> {
        let table = txn.open_table(CHANGES)?;
        for_each_table_value(&table, |value| {
            let change: Change = postcard::from_bytes(value)?;
            world.log_mut().append(change);
            Ok(())
        })
    }

    fn load_properties(
        &self,
        txn: &redb::ReadTransaction,
        world: &mut World,
    ) -> Result<(), StorageError> {
        let table = txn.open_table(PROPERTIES)?;
        let mut iter = table.iter()?;
        while let Some(entry) = iter.next() {
            let (key, value) = entry?;
            let props: Properties = postcard::from_bytes(value.value())?;
            world.properties_mut().insert(LocusId(key.value()), props);
        }
        Ok(())
    }

    fn load_names(
        &self,
        txn: &redb::ReadTransaction,
        world: &mut World,
    ) -> Result<(), StorageError> {
        let table = txn.open_table(NAMES)?;
        let mut iter = table.iter()?;
        while let Some(entry) = iter.next() {
            let (key, value) = entry?;
            world
                .names_mut()
                .insert(key.value().to_owned(), LocusId(value.value()));
        }
        Ok(())
    }

    fn load_aliases(
        &self,
        txn: &redb::ReadTransaction,
        world: &mut World,
    ) -> Result<(), StorageError> {
        let table = txn.open_table(ALIASES)?;
        let mut iter = table.iter()?;
        while let Some(entry) = iter.next() {
            let (key, value) = entry?;
            world
                .names_mut()
                .add_alias(key.value().to_owned(), LocusId(value.value()));
        }
        Ok(())
    }

    fn load_subscriptions(
        &self,
        txn: &redb::ReadTransaction,
        world: &mut World,
    ) -> Result<(), StorageError> {
        let table = txn.open_multimap_table(SUBSCRIPTIONS)?;
        let mut iter = table.iter()?;
        while let Some(entry) = iter.next() {
            let (locus_key, mut rel_iter) = entry?;
            let locus_id = LocusId(locus_key.value());
            while let Some(rel_entry) = rel_iter.next() {
                let rel_id = RelationshipId(rel_entry?.value());
                world.subscriptions_mut().subscribe(locus_id, rel_id);
            }
        }
        Ok(())
    }

    fn load_bcm_thresholds(
        &self,
        txn: &redb::ReadTransaction,
        world: &mut World,
    ) -> Result<(), StorageError> {
        let table = txn.open_table(BCM_THRESHOLDS)?;
        let mut iter = table.iter()?;
        while let Some(entry) = iter.next() {
            let (key, value) = entry?;
            let theta: f32 = postcard::from_bytes(value.value())?;
            world
                .bcm_thresholds_mut()
                .insert(LocusId(key.value()), theta);
        }
        Ok(())
    }

    fn sync_loaded_generations(&self, world: &World) {
        self.last_subscription_gen
            .set(world.subscriptions().generation());
        self.last_relationship_gen
            .set(world.relationships().generation());
        self.last_entity_gen.set(world.entities().generation());
    }
}

impl<'a> SnapshotWriter<'a> {
    fn new(storage: &'a Storage, txn: &'a redb::WriteTransaction, world: &'a World) -> Self {
        Self {
            storage,
            txn,
            world,
        }
    }

    fn write_all(self) -> Result<(), StorageError> {
        self.storage.clear_world_tables(self.txn)?;
        self.storage.write_all_loci(self.txn, self.world)?;
        self.storage.write_all_relationships(self.txn, self.world)?;
        self.storage.write_all_entities(self.txn, self.world)?;
        self.storage.write_all_changes(self.txn, self.world)?;
        self.storage.write_all_properties(self.txn, self.world)?;
        self.storage.write_all_names(self.txn, self.world)?;
        self.storage.write_all_subscriptions(self.txn, self.world)?;
        self.storage
            .write_all_bcm_thresholds(self.txn, self.world)?;
        self.storage.write_meta(self.txn, self.world)?;
        Ok(())
    }
}

impl<'a> SnapshotLoader<'a> {
    fn new(storage: &'a Storage, txn: &'a redb::ReadTransaction) -> Self {
        Self {
            storage,
            txn,
            world: World::new(),
        }
    }

    fn load(mut self) -> Result<World, StorageError> {
        self.storage.load_meta(self.txn, &mut self.world)?;
        self.storage.load_loci(self.txn, &mut self.world)?;
        self.storage.load_relationships(self.txn, &mut self.world)?;
        self.storage.load_entities(self.txn, &mut self.world)?;
        self.storage.load_changes(self.txn, &mut self.world)?;
        self.storage.load_properties(self.txn, &mut self.world)?;
        self.storage.load_names(self.txn, &mut self.world)?;
        self.storage.load_aliases(self.txn, &mut self.world)?;
        self.storage.load_subscriptions(self.txn, &mut self.world)?;
        self.storage
            .load_bcm_thresholds(self.txn, &mut self.world)?;
        self.storage.sync_loaded_generations(&self.world);
        Ok(self.world)
    }
}
