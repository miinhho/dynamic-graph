use graph_core::{
    BatchId, Change, ChangeId, Entity, EntityId, Locus, LocusId, Properties, Relationship,
    RelationshipId,
};
use redb::{ReadOnlyMultimapTable, ReadOnlyTable, ReadableTableMetadata};

use crate::tables::{
    ALIASES, CHANGES, CHANGES_BY_BATCH, ENTITIES, LOCI, NAMES, PROPERTIES, REL_BY_LOCUS,
    RELATIONSHIPS,
};
use crate::util::{collect_postcard_rows, read_postcard_row};
use crate::{Storage, StorageCounts, StorageError};

pub(super) struct ReadSession<'a> {
    txn: redb::ReadTransaction,
    _storage: &'a Storage,
}

impl<'a> ReadSession<'a> {
    pub(super) fn new(storage: &'a Storage) -> Result<Self, StorageError> {
        Ok(Self {
            txn: storage.db.begin_read()?,
            _storage: storage,
        })
    }

    pub(super) fn read_locus(&self, id: LocusId) -> Result<Option<Locus>, StorageError> {
        read_postcard_row(&self.txn.open_table(LOCI)?, id.0)
    }

    pub(super) fn read_all_relationships(&self) -> Result<Vec<Relationship>, StorageError> {
        collect_postcard_rows(&self.txn.open_table(RELATIONSHIPS)?)
    }

    pub(super) fn read_relationship(
        &self,
        id: RelationshipId,
    ) -> Result<Option<Relationship>, StorageError> {
        read_postcard_row(&self.txn.open_table(RELATIONSHIPS)?, id.0)
    }

    pub(super) fn read_entity(&self, id: EntityId) -> Result<Option<Entity>, StorageError> {
        read_postcard_row(&self.txn.open_table(ENTITIES)?, id.0)
    }

    pub(super) fn read_relationships_for_locus(
        &self,
        locus_id: LocusId,
    ) -> Result<Vec<Relationship>, StorageError> {
        let idx = self.txn.open_multimap_table(REL_BY_LOCUS)?;
        let rels = self.txn.open_table(RELATIONSHIPS)?;
        collect_related_rows(&idx, locus_id.0, &rels)
    }

    pub(super) fn read_change(&self, id: ChangeId) -> Result<Option<Change>, StorageError> {
        read_postcard_row(&self.txn.open_table(CHANGES)?, id.0)
    }

    pub(super) fn read_changes_for_batch(
        &self,
        batch: BatchId,
    ) -> Result<Vec<Change>, StorageError> {
        let batch_idx = self.txn.open_multimap_table(CHANGES_BY_BATCH)?;
        let changes = self.txn.open_table(CHANGES)?;
        collect_related_rows(&batch_idx, batch.0, &changes)
    }

    pub(super) fn read_properties(&self, id: LocusId) -> Result<Option<Properties>, StorageError> {
        read_postcard_row(&self.txn.open_table(PROPERTIES)?, id.0)
    }

    pub(super) fn resolve_name(&self, name: &str) -> Result<Option<LocusId>, StorageError> {
        if let Some(id) = lookup_name(&self.txn.open_table(NAMES)?, name)? {
            return Ok(Some(id));
        }
        lookup_name(&self.txn.open_table(ALIASES)?, name)
    }

    pub(super) fn table_counts(&self) -> Result<StorageCounts, StorageError> {
        let txn = &self.txn;
        Ok(StorageCounts {
            loci: txn.open_table(LOCI)?.len()?,
            relationships: txn.open_table(RELATIONSHIPS)?.len()?,
            entities: txn.open_table(ENTITIES)?.len()?,
            changes: txn.open_table(CHANGES)?.len()?,
            properties: txn.open_table(PROPERTIES)?.len()?,
            names: txn.open_table(NAMES)?.len()?,
            aliases: txn.open_table(ALIASES)?.len()?,
        })
    }
}

fn collect_related_rows<T: for<'de> serde::Deserialize<'de>>(
    index: &ReadOnlyMultimapTable<u64, u64>,
    key: u64,
    data: &ReadOnlyTable<u64, &[u8]>,
) -> Result<Vec<T>, StorageError> {
    let mut result = Vec::new();
    let mut iter = index.get(key)?;
    while let Some(entry) = iter.next() {
        let id = entry?.value();
        if let Some(value) = data.get(id)? {
            result.push(postcard::from_bytes(value.value())?);
        }
    }
    Ok(result)
}

fn lookup_name(
    table: &ReadOnlyTable<&str, u64>,
    name: &str,
) -> Result<Option<LocusId>, StorageError> {
    Ok(table.get(name)?.map(|value| LocusId(value.value())))
}
