// redb iterators yield Result on .next(), making `for` loops impossible.
#![allow(clippy::while_let_on_iterator)]

//! graph-storage: redb-backed persistent storage for the substrate.
//!
//! Provides
//! ACID transactions, random-access reads, and automatic compaction
//! courtesy of redb's B-tree engine.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use graph_storage::Storage;
//! use graph_world::World;
//!
//! let storage = Storage::open("/tmp/my_graph.redb").unwrap();
//!
//! // Save entire world
//! let world = World::new();
//! storage.save_world(&world).unwrap();
//!
//! // Load it back
//! let restored = storage.load_world().unwrap();
//! ```
//!
//! ## Design
//!
//! Each domain store maps to a redb table:
//!
//! | Table | Key | Value |
//! |-------|-----|-------|
//! | `loci` | `LocusId` (u64) | postcard `Locus` |
//! | `relationships` | `RelationshipId` (u64) | postcard `Relationship` |
//! | `entities` | `EntityId` (u64) | postcard `Entity` |
//! | `changes` | `ChangeId` (u64) | postcard `Change` |
//! | `changes_by_batch` | `BatchId` (u64) | multimap → `ChangeId`s |
//! | `properties` | `LocusId` (u64) | postcard `Properties` |
//! | `names` | `&str` | `LocusId` (u64) |
//! | `aliases` | `&str` | `LocusId` (u64) |
//! | `meta` | `&str` | u64 counter |
//!
//! Values are serialized with postcard (compact, zero-alloc decode).
//! redb handles crash safety, page management, and compaction internally.

mod error;
mod tables;

pub use error::StorageError;

use std::path::Path;

use graph_core::{
    BatchId, Change, ChangeId, Entity, EntityId, Locus, LocusId, Properties,
    Relationship, RelationshipId,
};
use graph_world::World;
use redb::{Database, ReadableMultimapTable, ReadableTable, ReadableTableMetadata};

use tables::*;

/// redb-backed persistent storage for a `World`.
pub struct Storage {
    db: Database,
}

impl Storage {
    /// Open or create a database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let db = Database::create(path.as_ref())?;

        // Ensure all tables exist by opening a write transaction.
        let txn = db.begin_write()?;
        {
            let _ = txn.open_table(LOCI)?;
            let _ = txn.open_table(RELATIONSHIPS)?;
            let _ = txn.open_table(ENTITIES)?;
            let _ = txn.open_table(CHANGES)?;
            let _ = txn.open_multimap_table(CHANGES_BY_BATCH)?;
            let _ = txn.open_table(PROPERTIES)?;
            let _ = txn.open_table(NAMES)?;
            let _ = txn.open_table(ALIASES)?;
            let _ = txn.open_table(META)?;
        }
        txn.commit()?;

        Ok(Self { db })
    }

    // ── full save / load ────────────────────────────────────────────

    /// Persist the entire world in a single ACID transaction.
    ///
    /// This is a **replace-all** operation: existing data is cleared
    /// and the full world state is written. For incremental writes,
    /// use [`commit_batch`].
    pub fn save_world(&self, world: &World) -> Result<(), StorageError> {
        let txn = self.db.begin_write()?;
        {
            // Clear all tables first.
            {
                let mut t = txn.open_table(LOCI)?;
                // redb 2.x: drain via retain or iterate+delete. We use a
                // simpler pattern: drop the table contents by writing fresh.
                while let Some(guard) = t.pop_last()? {
                    drop(guard);
                }
            }
            {
                let mut t = txn.open_table(RELATIONSHIPS)?;
                while let Some(g) = t.pop_last()? { drop(g); }
            }
            {
                let mut t = txn.open_table(ENTITIES)?;
                while let Some(g) = t.pop_last()? { drop(g); }
            }
            {
                let mut t = txn.open_table(CHANGES)?;
                while let Some(g) = t.pop_last()? { drop(g); }
            }
            {
                let mut t = txn.open_multimap_table(CHANGES_BY_BATCH)?;
                // Multimap tables: we need to clear all entries.
                // Collect keys first to avoid borrow issues.
                let keys: Vec<u64> = {
                    let mut keys = Vec::new();
                    let mut iter = t.iter()?;
                    while let Some(entry) = iter.next() {
                        let (key, _) = entry?;
                        keys.push(key.value());
                    }
                    keys
                };
                for key in keys {
                    t.remove_all(key)?;
                }
            }
            {
                let mut t = txn.open_table(PROPERTIES)?;
                while let Some(g) = t.pop_last()? { drop(g); }
            }
            {
                let mut t = txn.open_table(NAMES)?;
                while let Some(g) = t.pop_last()? { drop(g); }
            }
            {
                let mut t = txn.open_table(ALIASES)?;
                while let Some(g) = t.pop_last()? { drop(g); }
            }

            // Write loci.
            {
                let mut t = txn.open_table(LOCI)?;
                for locus in world.loci().iter() {
                    let bytes = postcard::to_allocvec(locus)?;
                    t.insert(locus.id.0, bytes.as_slice())?;
                }
            }

            // Write relationships.
            {
                let mut t = txn.open_table(RELATIONSHIPS)?;
                for rel in world.relationships().iter() {
                    let bytes = postcard::to_allocvec(rel)?;
                    t.insert(rel.id.0, bytes.as_slice())?;
                }
            }

            // Write entities.
            {
                let mut t = txn.open_table(ENTITIES)?;
                for entity in world.entities().iter() {
                    let bytes = postcard::to_allocvec(entity)?;
                    t.insert(entity.id.0, bytes.as_slice())?;
                }
            }

            // Write changes + batch index.
            {
                let mut changes_t = txn.open_table(CHANGES)?;
                let mut batch_idx = txn.open_multimap_table(CHANGES_BY_BATCH)?;
                for change in world.log().iter() {
                    let bytes = postcard::to_allocvec(change)?;
                    changes_t.insert(change.id.0, bytes.as_slice())?;
                    batch_idx.insert(change.batch.0, change.id.0)?;
                }
            }

            // Write properties.
            {
                let mut t = txn.open_table(PROPERTIES)?;
                for (id, props) in world.properties().iter() {
                    let bytes = postcard::to_allocvec(props)?;
                    t.insert(id.0, bytes.as_slice())?;
                }
            }

            // Write names + aliases.
            {
                let mut names_t = txn.open_table(NAMES)?;
                let mut aliases_t = txn.open_table(ALIASES)?;
                for (name, id) in world.names().iter() {
                    names_t.insert(name, id.0)?;
                }
                for (alias, id) in world.names().aliases() {
                    aliases_t.insert(alias, id.0)?;
                }
            }

            // Write meta counters.
            {
                let meta = world.world_meta();
                let mut t = txn.open_table(META)?;
                t.insert(META_CURRENT_BATCH, meta.current_batch.0)?;
                t.insert(META_NEXT_CHANGE_ID, meta.next_change_id)?;
                t.insert(META_NEXT_RELATIONSHIP_ID, meta.next_relationship_id)?;
                t.insert(META_NEXT_ENTITY_ID, meta.next_entity_id)?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    /// Load the full world from the database.
    ///
    /// Returns `Err(StorageError::Empty)` if no world has been saved.
    pub fn load_world(&self) -> Result<World, StorageError> {
        let txn = self.db.begin_read()?;
        let mut world = World::new();

        // Read meta first to check if DB has data.
        {
            let t = txn.open_table(META)?;
            let batch = t.get(META_CURRENT_BATCH)?
                .ok_or(StorageError::Empty)?;
            let next_cid = t.get(META_NEXT_CHANGE_ID)?
                .ok_or(StorageError::Empty)?;
            let next_rid = t.get(META_NEXT_RELATIONSHIP_ID)?
                .ok_or(StorageError::Empty)?;
            let next_eid = t.get(META_NEXT_ENTITY_ID)?
                .ok_or(StorageError::Empty)?;

            let meta = graph_world::WorldMeta {
                current_batch: BatchId(batch.value()),
                next_change_id: next_cid.value(),
                next_relationship_id: next_rid.value(),
                next_entity_id: next_eid.value(),
            };
            world.restore_meta(&meta);
        }

        // Read loci.
        {
            let t = txn.open_table(LOCI)?;
            let mut iter = t.iter()?;
            while let Some(entry) = iter.next() {
                let (_, val) = entry?;
                let locus: Locus = postcard::from_bytes(val.value())?;
                world.insert_locus(locus);
            }
        }

        // Read relationships.
        {
            let t = txn.open_table(RELATIONSHIPS)?;
            let mut iter = t.iter()?;
            while let Some(entry) = iter.next() {
                let (_, val) = entry?;
                let rel: Relationship = postcard::from_bytes(val.value())?;
                world.relationships_mut().insert(rel);
            }
        }

        // Read entities.
        {
            let t = txn.open_table(ENTITIES)?;
            let mut iter = t.iter()?;
            while let Some(entry) = iter.next() {
                let (_, val) = entry?;
                let entity: Entity = postcard::from_bytes(val.value())?;
                world.entities_mut().insert(entity);
            }
        }

        // Read changes (ordered by ChangeId for density invariant).
        {
            let t = txn.open_table(CHANGES)?;
            let mut iter = t.iter()?;
            while let Some(entry) = iter.next() {
                let (_, val) = entry?;
                let change: Change = postcard::from_bytes(val.value())?;
                world.log_mut().append(change);
            }
        }

        // Read properties.
        {
            let t = txn.open_table(PROPERTIES)?;
            let mut iter = t.iter()?;
            while let Some(entry) = iter.next() {
                let (key, val) = entry?;
                let props: Properties = postcard::from_bytes(val.value())?;
                world.properties_mut().insert(LocusId(key.value()), props);
            }
        }

        // Read names + aliases.
        {
            let t = txn.open_table(NAMES)?;
            let mut iter = t.iter()?;
            while let Some(entry) = iter.next() {
                let (key, val) = entry?;
                world.names_mut().insert(key.value().to_owned(), LocusId(val.value()));
            }
        }
        {
            let t = txn.open_table(ALIASES)?;
            let mut iter = t.iter()?;
            while let Some(entry) = iter.next() {
                let (key, val) = entry?;
                world.names_mut().add_alias(key.value().to_owned(), LocusId(val.value()));
            }
        }

        Ok(world)
    }

    // ── incremental commit ──────────────────────────────────────────

    /// Persist only the changes from a single committed batch.
    ///
    /// This is the incremental equivalent of a WAL batch record:
    /// only the touched data is written, not the entire world.
    /// Call this after each `Engine::tick()` or `Simulation::step()`.
    pub fn commit_batch(
        &self,
        world: &World,
        committed_batch: BatchId,
    ) -> Result<(), StorageError> {
        use graph_core::ChangeSubject;

        let changes: Vec<_> = world.log().batch(committed_batch).cloned().collect();
        if changes.is_empty() {
            return Ok(());
        }

        // Collect touched locus IDs from changes.
        let touched_locus_ids: std::collections::HashSet<LocusId> = changes
            .iter()
            .filter_map(|c| match c.subject {
                ChangeSubject::Locus(id) => Some(id),
                _ => None,
            })
            .collect();

        let txn = self.db.begin_write()?;
        {
            // Write changes + batch index.
            {
                let mut changes_t = txn.open_table(CHANGES)?;
                let mut batch_idx = txn.open_multimap_table(CHANGES_BY_BATCH)?;
                for change in &changes {
                    let bytes = postcard::to_allocvec(change)?;
                    changes_t.insert(change.id.0, bytes.as_slice())?;
                    batch_idx.insert(change.batch.0, change.id.0)?;
                }
            }

            // Upsert touched loci.
            {
                let mut t = txn.open_table(LOCI)?;
                for &id in &touched_locus_ids {
                    if let Some(locus) = world.locus(id) {
                        let bytes = postcard::to_allocvec(locus)?;
                        t.insert(id.0, bytes.as_slice())?;
                    }
                }
            }

            // Upsert touched relationships.
            {
                let mut t = txn.open_table(RELATIONSHIPS)?;
                for rel in world.relationships().iter() {
                    let dominated_by_this_batch = rel.lineage.created_by
                        .or(rel.lineage.last_touched_by)
                        .and_then(|cid| world.log().get(cid))
                        .is_some_and(|c| c.batch == committed_batch);
                    if dominated_by_this_batch {
                        let bytes = postcard::to_allocvec(rel)?;
                        t.insert(rel.id.0, bytes.as_slice())?;
                    }
                }
            }

            // Upsert touched entities.
            {
                let mut t = txn.open_table(ENTITIES)?;
                for entity in world.entities().iter() {
                    if entity.layers.last().is_some_and(|l| l.batch == committed_batch) {
                        let bytes = postcard::to_allocvec(entity)?;
                        t.insert(entity.id.0, bytes.as_slice())?;
                    }
                }
            }

            // Upsert properties for touched loci.
            {
                let mut t = txn.open_table(PROPERTIES)?;
                for &id in &touched_locus_ids {
                    if let Some(props) = world.properties().get(id) {
                        let bytes = postcard::to_allocvec(props)?;
                        t.insert(id.0, bytes.as_slice())?;
                    }
                }
            }

            // Upsert name mappings for touched loci.
            {
                let mut t = txn.open_table(NAMES)?;
                for &id in &touched_locus_ids {
                    if let Some(name) = world.names().name_of(id) {
                        t.insert(name, id.0)?;
                    }
                }
            }

            // Update meta counters.
            {
                let meta = world.world_meta();
                let mut t = txn.open_table(META)?;
                t.insert(META_CURRENT_BATCH, meta.current_batch.0)?;
                t.insert(META_NEXT_CHANGE_ID, meta.next_change_id)?;
                t.insert(META_NEXT_RELATIONSHIP_ID, meta.next_relationship_id)?;
                t.insert(META_NEXT_ENTITY_ID, meta.next_entity_id)?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    // ── point queries ───────────────────────────────────────────────

    /// Read a single locus by ID.
    pub fn get_locus(&self, id: LocusId) -> Result<Option<Locus>, StorageError> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(LOCI)?;
        match t.get(id.0)? {
            Some(val) => Ok(Some(postcard::from_bytes(val.value())?)),
            None => Ok(None),
        }
    }

    /// Read a single relationship by ID.
    pub fn get_relationship(&self, id: RelationshipId) -> Result<Option<Relationship>, StorageError> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(RELATIONSHIPS)?;
        match t.get(id.0)? {
            Some(val) => Ok(Some(postcard::from_bytes(val.value())?)),
            None => Ok(None),
        }
    }

    /// Read a single entity by ID.
    pub fn get_entity(&self, id: EntityId) -> Result<Option<Entity>, StorageError> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(ENTITIES)?;
        match t.get(id.0)? {
            Some(val) => Ok(Some(postcard::from_bytes(val.value())?)),
            None => Ok(None),
        }
    }

    /// Read a single change by ID.
    pub fn get_change(&self, id: ChangeId) -> Result<Option<Change>, StorageError> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(CHANGES)?;
        match t.get(id.0)? {
            Some(val) => Ok(Some(postcard::from_bytes(val.value())?)),
            None => Ok(None),
        }
    }

    /// Read all changes for a given batch.
    pub fn changes_for_batch(&self, batch: BatchId) -> Result<Vec<Change>, StorageError> {
        let txn = self.db.begin_read()?;
        let batch_idx = txn.open_multimap_table(CHANGES_BY_BATCH)?;
        let changes_t = txn.open_table(CHANGES)?;

        let mut changes = Vec::new();
        let mut iter = batch_idx.get(batch.0)?;
        while let Some(entry) = iter.next() {
            let change_id = entry?.value();
            if let Some(val) = changes_t.get(change_id)? {
                let change: Change = postcard::from_bytes(val.value())?;
                changes.push(change);
            }
        }
        Ok(changes)
    }

    /// Read properties for a locus.
    pub fn get_properties(&self, id: LocusId) -> Result<Option<Properties>, StorageError> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(PROPERTIES)?;
        match t.get(id.0)? {
            Some(val) => Ok(Some(postcard::from_bytes(val.value())?)),
            None => Ok(None),
        }
    }

    /// Resolve a name (canonical or alias) to a LocusId.
    pub fn resolve_name(&self, name: &str) -> Result<Option<LocusId>, StorageError> {
        let txn = self.db.begin_read()?;
        // Try canonical first.
        let names_t = txn.open_table(NAMES)?;
        if let Some(val) = names_t.get(name)? {
            return Ok(Some(LocusId(val.value())));
        }
        // Then aliases.
        let aliases_t = txn.open_table(ALIASES)?;
        if let Some(val) = aliases_t.get(name)? {
            return Ok(Some(LocusId(val.value())));
        }
        Ok(None)
    }

    // ── statistics ──────────────────────────────────────────────────

    /// Count of records in each table.
    pub fn table_counts(&self) -> Result<StorageCounts, StorageError> {
        let txn = self.db.begin_read()?;
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

/// Record counts per table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageCounts {
    pub loci: u64,
    pub relationships: u64,
    pub entities: u64,
    pub changes: u64,
    pub properties: u64,
    pub names: u64,
    pub aliases: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{LocusKindId, StateVector, props};
    use tempfile::NamedTempFile;

    fn temp_db() -> (NamedTempFile, Storage) {
        let f = NamedTempFile::new().unwrap();
        let storage = Storage::open(f.path()).unwrap();
        (f, storage)
    }

    fn sample_world() -> World {
        let mut world = World::new();
        let id0 = LocusId(0);
        let id1 = LocusId(1);
        world.insert_locus(Locus::new(id0, LocusKindId(1), StateVector::zeros(2)));
        world.insert_locus(Locus::new(id1, LocusKindId(1), StateVector::from_slice(&[0.5, 1.0])));
        world.properties_mut().insert(id0, props! { "name" => "Alpha", "score" => 0.8_f64 });
        world.properties_mut().insert(id1, props! { "name" => "Beta" });
        world.names_mut().insert("Alpha", id0);
        world.names_mut().insert("Beta", id1);
        world.names_mut().add_alias("A", id0);
        world
    }

    #[test]
    fn open_creates_tables() {
        let (_f, storage) = temp_db();
        let counts = storage.table_counts().unwrap();
        assert_eq!(counts.loci, 0);
    }

    #[test]
    fn save_and_load_world_round_trip() {
        let (_f, storage) = temp_db();
        let world = sample_world();
        storage.save_world(&world).unwrap();

        let restored = storage.load_world().unwrap();
        assert_eq!(restored.loci().iter().count(), 2);
        assert_eq!(restored.world_meta(), world.world_meta());

        // Properties.
        assert_eq!(
            restored.properties().get(LocusId(0)).unwrap().get_str("name"),
            Some("Alpha")
        );
        assert_eq!(
            restored.properties().get(LocusId(0)).unwrap().get_f64("score"),
            Some(0.8)
        );
        assert_eq!(
            restored.properties().get(LocusId(1)).unwrap().get_str("name"),
            Some("Beta")
        );

        // Names.
        assert_eq!(restored.names().resolve("Alpha"), Some(LocusId(0)));
        assert_eq!(restored.names().resolve("Beta"), Some(LocusId(1)));
        assert_eq!(restored.names().resolve("A"), Some(LocusId(0)));
    }

    #[test]
    fn load_empty_returns_error() {
        let (_f, storage) = temp_db();
        let result = storage.load_world();
        assert!(matches!(result, Err(StorageError::Empty)));
    }

    #[test]
    fn point_query_locus() {
        let (_f, storage) = temp_db();
        let world = sample_world();
        storage.save_world(&world).unwrap();

        let locus = storage.get_locus(LocusId(0)).unwrap().unwrap();
        assert_eq!(locus.id, LocusId(0));
        assert_eq!(locus.kind, LocusKindId(1));
        assert!(storage.get_locus(LocusId(999)).unwrap().is_none());
    }

    #[test]
    fn point_query_properties() {
        let (_f, storage) = temp_db();
        let world = sample_world();
        storage.save_world(&world).unwrap();

        let props = storage.get_properties(LocusId(0)).unwrap().unwrap();
        assert_eq!(props.get_str("name"), Some("Alpha"));
        assert!(storage.get_properties(LocusId(999)).unwrap().is_none());
    }

    #[test]
    fn resolve_name_canonical_and_alias() {
        let (_f, storage) = temp_db();
        let world = sample_world();
        storage.save_world(&world).unwrap();

        assert_eq!(storage.resolve_name("Alpha").unwrap(), Some(LocusId(0)));
        assert_eq!(storage.resolve_name("A").unwrap(), Some(LocusId(0)));
        assert_eq!(storage.resolve_name("Beta").unwrap(), Some(LocusId(1)));
        assert!(storage.resolve_name("unknown").unwrap().is_none());
    }

    #[test]
    fn table_counts_match_world() {
        let (_f, storage) = temp_db();
        let world = sample_world();
        storage.save_world(&world).unwrap();

        let counts = storage.table_counts().unwrap();
        assert_eq!(counts.loci, 2);
        assert_eq!(counts.properties, 2);
        assert_eq!(counts.names, 2);
        assert_eq!(counts.aliases, 1);
    }

    #[test]
    fn save_world_replaces_previous() {
        let (_f, storage) = temp_db();

        // Save initial.
        let world1 = sample_world();
        storage.save_world(&world1).unwrap();
        assert_eq!(storage.table_counts().unwrap().loci, 2);

        // Save a different world with 1 locus.
        let mut world2 = World::new();
        world2.insert_locus(Locus::new(LocusId(99), LocusKindId(1), StateVector::zeros(1)));
        storage.save_world(&world2).unwrap();

        let counts = storage.table_counts().unwrap();
        assert_eq!(counts.loci, 1);
        let loaded = storage.load_world().unwrap();
        assert!(loaded.locus(LocusId(99)).is_some());
        assert!(loaded.locus(LocusId(0)).is_none());
    }
}
