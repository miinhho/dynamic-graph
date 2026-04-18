use std::collections::HashSet;

use graph_core::{BatchId, Change, Entity, LocusId, Relationship};
use graph_world::World;
use redb::{ReadableMultimapTable, ReadableTable, ReadableTableMetadata};

use crate::StorageError;

pub(super) fn insert_rel_by_locus(
    idx: &mut redb::MultimapTable<u64, u64>,
    rel: &Relationship,
) -> Result<(), StorageError> {
    use graph_core::Endpoints;
    match &rel.endpoints {
        Endpoints::Directed { from, to } => {
            idx.insert(from.0, rel.id.0)?;
            idx.insert(to.0, rel.id.0)?;
        }
        Endpoints::Symmetric { a, b } => {
            idx.insert(a.0, rel.id.0)?;
            idx.insert(b.0, rel.id.0)?;
        }
    }
    Ok(())
}

pub(super) fn pop_all_rows<K, V>(table: &mut redb::Table<K, V>) -> Result<(), StorageError>
where
    K: redb::Key + 'static,
    V: redb::Value + 'static,
{
    while let Some(guard) = table.pop_last()? {
        drop(guard);
    }
    Ok(())
}

pub(super) fn remove_all_multimap_rows(
    table: &mut redb::MultimapTable<u64, u64>,
) -> Result<(), StorageError> {
    let keys = collect_multimap_keys(table)?;
    for key in keys {
        table.remove_all(key)?;
    }
    Ok(())
}

fn collect_multimap_keys(table: &redb::MultimapTable<u64, u64>) -> Result<Vec<u64>, StorageError> {
    let mut keys = Vec::new();
    let mut iter = table.iter()?;
    while let Some(entry) = iter.next() {
        keys.push(entry?.0.value());
    }
    Ok(keys)
}

pub(super) fn write_postcard_row<T: serde::Serialize>(
    table: &mut redb::Table<u64, &[u8]>,
    key: u64,
    value: &T,
) -> Result<(), StorageError> {
    let bytes = postcard::to_allocvec(value)?;
    table.insert(key, bytes.as_slice())?;
    Ok(())
}

pub(super) fn read_postcard_row<T: serde::de::DeserializeOwned>(
    table: &redb::ReadOnlyTable<u64, &[u8]>,
    key: u64,
) -> Result<Option<T>, StorageError> {
    table
        .get(key)?
        .map(|value| postcard::from_bytes(value.value()))
        .transpose()
        .map_err(Into::into)
}

pub(super) fn collect_postcard_rows<T: serde::de::DeserializeOwned>(
    table: &redb::ReadOnlyTable<u64, &[u8]>,
) -> Result<Vec<T>, StorageError> {
    let mut rows = Vec::with_capacity(table.len()? as usize);
    for_each_table_value(table, |value| {
        rows.push(postcard::from_bytes(value)?);
        Ok(())
    })?;
    Ok(rows)
}

pub(super) fn for_each_table_value<T, F>(
    table: &redb::ReadOnlyTable<u64, &[u8]>,
    mut f: F,
) -> Result<(), StorageError>
where
    F: FnMut(&[u8]) -> Result<T, StorageError>,
{
    let mut iter = table.iter()?;
    while let Some(entry) = iter.next() {
        let (_, value) = entry?;
        f(value.value())?;
    }
    Ok(())
}

pub(super) fn collect_touched_locus_ids(changes: &[Change]) -> HashSet<LocusId> {
    use graph_core::ChangeSubject;

    changes
        .iter()
        .filter_map(|change| match change.subject {
            ChangeSubject::Locus(id) => Some(id),
            _ => None,
        })
        .collect()
}

pub(super) fn relationship_touched_in_batch(
    world: &World,
    rel: &Relationship,
    committed_batch: BatchId,
) -> bool {
    rel.lineage
        .created_by
        .or(rel.lineage.last_touched_by)
        .and_then(|cid| world.log().get(cid))
        .is_some_and(|change| change.batch == committed_batch)
}

pub(super) fn entity_touched_in_batch(entity: &Entity, committed_batch: BatchId) -> bool {
    entity
        .layers
        .last()
        .is_some_and(|layer| layer.batch == committed_batch)
}
