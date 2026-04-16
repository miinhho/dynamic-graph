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

use std::cell::Cell;
use std::path::Path;

use graph_core::{
    BatchId, Change, ChangeId, Entity, EntityId, Locus, LocusId, Properties,
    Relationship, RelationshipId,
};
use graph_world::World;
use redb::{Database, ReadableMultimapTable, ReadableTable, ReadableTableMetadata};

use tables::*;

/// Schema version stamped into every new database.
/// `open()` rejects databases written under a different version.
///
/// ## Development policy
///
/// During development, **do not bump this constant** on every internal struct
/// change. Instead:
///
/// - Use `open_or_reset()` as the entry point — it silently deletes and
///   recreates the file on mismatch, so stale databases are never an obstacle.
/// - `Simulation::with_config` (via `SimulationBuilder::with_storage`) already
///   calls `open_or_reset`, so dev workflows are handled automatically.
///
/// Bump `CURRENT_SCHEMA_VERSION` only when you want to signal an explicit
/// incompatibility to users who call `Storage::open()` directly. Before the
/// v1 library release, reset this to 1 and write proper migration functions
/// for `open_and_migrate` instead of relying on destroy-and-recreate.
pub(crate) const CURRENT_SCHEMA_VERSION: u64 = 1;

/// redb-backed persistent storage for a `World`.
pub struct Storage {
    db: Database,
    /// Generation of the subscription set the last time subscriptions were
    /// written to the `SUBSCRIPTIONS` table. Compared against
    /// `world.subscriptions().generation()` in `commit_batch()` to skip
    /// the clear-and-rewrite when nothing changed. `u64::MAX` means "not
    /// yet initialised" and forces a write on the first `commit_batch()`.
    last_subscription_gen: Cell<u64>,
    /// Generation of the relationship store at the last persist. `u64::MAX`
    /// forces a write on the first `commit_batch()`.
    last_relationship_gen: Cell<u64>,
    /// Generation of the entity store at the last persist. `u64::MAX`
    /// forces a write on the first `commit_batch()`.
    last_entity_gen: Cell<u64>,
}

impl Storage {
    /// Open or create a database at `path`.
    ///
    /// Returns `Err(StorageError::SchemaMismatch)` when the database was
    /// created by a different `CURRENT_SCHEMA_VERSION`. Delete the file and
    /// re-open to start fresh.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let db = Database::create(path.as_ref())?;

        // Ensure all tables exist and validate / write schema version.
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
            let _ = txn.open_multimap_table(SUBSCRIPTIONS)?;
            let _ = txn.open_multimap_table(REL_BY_LOCUS)?;
            let mut meta = txn.open_table(META)?;
            let stored_version = meta.get(META_SCHEMA_VERSION)?.map(|v| v.value());
            match stored_version {
                None => {
                    // Fresh database — stamp the current version.
                    meta.insert(META_SCHEMA_VERSION, CURRENT_SCHEMA_VERSION)?;
                }
                Some(v) if v == CURRENT_SCHEMA_VERSION => {}
                Some(v) => {
                    return Err(StorageError::SchemaMismatch {
                        found: v,
                        expected: CURRENT_SCHEMA_VERSION,
                    });
                }
            }
        }
        txn.commit()?;

        Ok(Self {
            db,
            last_subscription_gen: Cell::new(u64::MAX),
            last_relationship_gen: Cell::new(u64::MAX),
            last_entity_gen: Cell::new(u64::MAX),
        })
    }


    /// Open or create a database, dropping and recreating it on schema mismatch.
    ///
    /// Use this during development: schema changes are frequent and migration
    /// functions are not written until the v1 release. On `SchemaMismatch`,
    /// this deletes the database file and opens a fresh one rather than
    /// returning an error.
    ///
    /// **Do not use in production** — all existing data is silently destroyed
    /// on version mismatch.
    pub fn open_or_reset(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        match Self::open(path.as_ref()) {
            Ok(s) => Ok(s),
            Err(StorageError::SchemaMismatch { .. }) => {
                std::fs::remove_file(path.as_ref()).ok();
                Self::open(path.as_ref())
            }
            Err(e) => Err(e),
        }
    }

    /// Drop all domain data and re-stamp the schema version.
    ///
    /// Use this as a last-resort when the serialized bytes are not
    /// recoverable (e.g. structs grew fields that break postcard EOF).
    /// After calling this, the database is as if freshly created.
    pub fn reset(&self) -> Result<(), StorageError> {
        let txn = self.db.begin_write()?;
        {
            { let mut t = txn.open_table(LOCI)?; while let Some(g) = t.pop_last()? { drop(g); } }
            { let mut t = txn.open_table(RELATIONSHIPS)?; while let Some(g) = t.pop_last()? { drop(g); } }
            { let mut t = txn.open_table(ENTITIES)?; while let Some(g) = t.pop_last()? { drop(g); } }
            { let mut t = txn.open_table(CHANGES)?; while let Some(g) = t.pop_last()? { drop(g); } }
            {
                let mut t = txn.open_multimap_table(CHANGES_BY_BATCH)?;
                let keys: Vec<u64> = { let mut k = Vec::new(); let mut it = t.iter()?; while let Some(e) = it.next() { k.push(e?.0.value()); } k };
                for key in keys { t.remove_all(key)?; }
            }
            { let mut t = txn.open_table(PROPERTIES)?; while let Some(g) = t.pop_last()? { drop(g); } }
            { let mut t = txn.open_table(NAMES)?; while let Some(g) = t.pop_last()? { drop(g); } }
            { let mut t = txn.open_table(ALIASES)?; while let Some(g) = t.pop_last()? { drop(g); } }
            {
                let mut t = txn.open_multimap_table(SUBSCRIPTIONS)?;
                let keys: Vec<u64> = { let mut k = Vec::new(); let mut it = t.iter()?; while let Some(e) = it.next() { k.push(e?.0.value()); } k };
                for key in keys { t.remove_all(key)?; }
            }
            {
                let mut t = txn.open_multimap_table(REL_BY_LOCUS)?;
                let keys: Vec<u64> = { let mut k = Vec::new(); let mut it = t.iter()?; while let Some(e) = it.next() { k.push(e?.0.value()); } k };
                for key in keys { t.remove_all(key)?; }
            }
            let mut meta = txn.open_table(META)?;
            // Clear all meta keys.
            while let Some(g) = meta.pop_last()? { drop(g); }
            meta.insert(META_SCHEMA_VERSION, CURRENT_SCHEMA_VERSION)?;
        }
        txn.commit()?;
        Ok(())
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
            {
                let mut t = txn.open_multimap_table(SUBSCRIPTIONS)?;
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
                let mut t = txn.open_multimap_table(REL_BY_LOCUS)?;
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

            // Write loci.
            {
                let mut t = txn.open_table(LOCI)?;
                for locus in world.loci().iter() {
                    let bytes = postcard::to_allocvec(locus)?;
                    t.insert(locus.id.0, bytes.as_slice())?;
                }
            }

            // Write relationships + secondary locus index.
            {
                let mut t = txn.open_table(RELATIONSHIPS)?;
                let mut idx = txn.open_multimap_table(REL_BY_LOCUS)?;
                for rel in world.relationships().iter() {
                    let bytes = postcard::to_allocvec(rel)?;
                    t.insert(rel.id.0, bytes.as_slice())?;
                    insert_rel_by_locus(&mut idx, rel)?;
                }
                self.last_relationship_gen.set(world.relationships().generation());
            }

            // Write entities.
            {
                let mut t = txn.open_table(ENTITIES)?;
                for entity in world.entities().iter() {
                    let bytes = postcard::to_allocvec(entity)?;
                    t.insert(entity.id.0, bytes.as_slice())?;
                }
                self.last_entity_gen.set(world.entities().generation());
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

            // Write subscriptions.
            {
                let mut t = txn.open_multimap_table(SUBSCRIPTIONS)?;
                for (rel_id, locus_id) in world.subscriptions().iter() {
                    t.insert(locus_id.0, rel_id.0)?;
                }
                self.last_subscription_gen.set(world.subscriptions().generation());
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

        // Read subscriptions.
        {
            let t = txn.open_multimap_table(SUBSCRIPTIONS)?;
            let mut iter = t.iter()?;
            while let Some(entry) = iter.next() {
                let (locus_key, mut rel_iter) = entry?;
                let locus_id = LocusId(locus_key.value());
                while let Some(rel_entry) = rel_iter.next() {
                    let rel_id = RelationshipId(rel_entry?.value());
                    world.subscriptions_mut().subscribe(locus_id, rel_id);
                }
            }
        }

        // Sync generations so the first commit_batch() after load skips
        // tables that haven't changed.
        self.last_subscription_gen.set(world.subscriptions().generation());
        self.last_relationship_gen.set(world.relationships().generation());
        self.last_entity_gen.set(world.entities().generation());

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

            // Upsert touched relationships — skip entirely if the store is
            // unchanged since the last persist (no new or updated relationships).
            {
                let current_rel_gen = world.relationships().generation();
                if current_rel_gen != self.last_relationship_gen.get() {
                    let mut t = txn.open_table(RELATIONSHIPS)?;
                    let mut idx = txn.open_multimap_table(REL_BY_LOCUS)?;
                    for rel in world.relationships().iter() {
                        let dominated_by_this_batch = rel.lineage.created_by
                            .or(rel.lineage.last_touched_by)
                            .and_then(|cid| world.log().get(cid))
                            .is_some_and(|c| c.batch == committed_batch);
                        if dominated_by_this_batch {
                            let bytes = postcard::to_allocvec(rel)?;
                            t.insert(rel.id.0, bytes.as_slice())?;
                            insert_rel_by_locus(&mut idx, rel)?;
                        }
                    }
                    self.last_relationship_gen.set(current_rel_gen);
                }
            }

            // Upsert touched entities — skip entirely when entity store
            // generation is unchanged (no entity recognition ran this batch).
            {
                let current_entity_gen = world.entities().generation();
                if current_entity_gen != self.last_entity_gen.get() {
                    let mut t = txn.open_table(ENTITIES)?;
                    for entity in world.entities().iter() {
                        if entity.layers.last().is_some_and(|l| l.batch == committed_batch) {
                            let bytes = postcard::to_allocvec(entity)?;
                            t.insert(entity.id.0, bytes.as_slice())?;
                        }
                    }
                    self.last_entity_gen.set(current_entity_gen);
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

            // Rewrite subscriptions only when the set actually changed.
            // `SubscriptionStore::generation()` is incremented on every
            // subscribe/unsubscribe mutation; we compare against the last
            // generation we persisted to avoid the clear-and-rewrite on
            // batches that contain relationship state changes but no
            // topology changes.
            {
                let current_gen = world.subscriptions().generation();
                if current_gen != self.last_subscription_gen.get() {
                    let mut t = txn.open_multimap_table(SUBSCRIPTIONS)?;
                    // Clear.
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
                    // Rewrite.
                    for (rel_id, locus_id) in world.subscriptions().iter() {
                        t.insert(locus_id.0, rel_id.0)?;
                    }
                    self.last_subscription_gen.set(current_gen);
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
    /// Iterate all stored relationships.
    ///
    /// Reads the full `RELATIONSHIPS` table in one transaction — O(n_stored).
    /// Used by `Simulation::promote_all_cold` to bring every cold relationship
    /// back into hot memory before an entity recognition pass.
    pub fn all_relationships(&self) -> Result<Vec<Relationship>, StorageError> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(RELATIONSHIPS)?;
        let mut out = Vec::new();
        let mut iter = t.iter()?;
        while let Some(entry) = iter.next() {
            let (_, val) = entry?;
            out.push(postcard::from_bytes(val.value())?);
        }
        Ok(out)
    }

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

    /// Read all stored relationships that have `locus_id` as an endpoint.
    ///
    /// Uses the `REL_BY_LOCUS` secondary index — O(k) where k is the number
    /// of relationships touching this locus. Used for cold→hot promotion.
    pub fn relationships_for_locus(
        &self,
        locus_id: LocusId,
    ) -> Result<Vec<Relationship>, StorageError> {
        let txn = self.db.begin_read()?;
        let idx = txn.open_multimap_table(REL_BY_LOCUS)?;
        let rels_t = txn.open_table(RELATIONSHIPS)?;
        let mut result = Vec::new();
        let mut iter = idx.get(locus_id.0)?;
        while let Some(entry) = iter.next() {
            let rel_id = entry?.value();
            if let Some(val) = rels_t.get(rel_id)? {
                let rel: Relationship = postcard::from_bytes(val.value())?;
                result.push(rel);
            }
        }
        Ok(result)
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

/// Insert both endpoints of `rel` into the `REL_BY_LOCUS` multimap index.
///
/// For directed edges: inserts `(from, rel_id)` and `(to, rel_id)`.
/// For symmetric edges: inserts `(a, rel_id)` and `(b, rel_id)`.
/// Inserting a duplicate `(locus, rel_id)` pair is idempotent in redb
/// multimap tables — duplicate values under the same key are deduplicated.
fn insert_rel_by_locus(
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

    #[test]
    fn change_with_wall_time_and_metadata_round_trips() {
        use graph_core::{BatchId, Change, ChangeId, ChangeSubject, InfluenceKindId, StateVector, props};

        let (_f, storage) = temp_db();
        let mut world = sample_world();

        let change = Change {
            id: ChangeId(0),
            subject: ChangeSubject::Locus(LocusId(0)),
            kind: InfluenceKindId(1),
            predecessors: vec![],
            before: StateVector::zeros(2),
            after: StateVector::from_slice(&[0.5, 0.0]),
            batch: BatchId(0),
            wall_time: Some(1_700_000_000_000),
            metadata: Some(props! { "source" => "sensor-A", "confidence" => 0.95_f64 }),
        };
        world.log_mut().append(change);
        storage.save_world(&world).unwrap();

        let restored = storage.load_world().unwrap();
        let c = restored.log().get(ChangeId(0)).unwrap();
        assert_eq!(c.wall_time, Some(1_700_000_000_000));
        let meta = c.metadata.as_ref().unwrap();
        assert_eq!(meta.get_str("source"), Some("sensor-A"));
        assert!((meta.get_f64("confidence").unwrap() - 0.95).abs() < 1e-9);
    }

    #[test]
    fn subscriptions_round_trip_via_save_load() {
        let (_f, storage) = temp_db();
        let mut world = sample_world();

        let rel_a = RelationshipId(10);
        let rel_b = RelationshipId(11);
        world.subscriptions_mut().subscribe(LocusId(0), rel_a);
        world.subscriptions_mut().subscribe(LocusId(0), rel_b);
        world.subscriptions_mut().subscribe(LocusId(1), rel_a);

        storage.save_world(&world).unwrap();

        let restored = storage.load_world().unwrap();
        assert_eq!(restored.subscriptions().subscription_count(), 3);
        assert!(restored.subscriptions().has_subscribers(rel_a));
        assert!(restored.subscriptions().has_subscribers(rel_b));

        let mut subs_a: Vec<LocusId> = restored.subscriptions().subscribers(rel_a).collect();
        subs_a.sort_by_key(|l| l.0);
        assert_eq!(subs_a, vec![LocusId(0), LocusId(1)]);

        let subs_b: Vec<LocusId> = restored.subscriptions().subscribers(rel_b).collect();
        assert_eq!(subs_b, vec![LocusId(0)]);
    }

    #[test]
    fn empty_subscriptions_save_and_load_cleanly() {
        let (_f, storage) = temp_db();
        let world = sample_world();
        storage.save_world(&world).unwrap();

        let restored = storage.load_world().unwrap();
        assert!(restored.subscriptions().is_empty());
        assert_eq!(restored.subscriptions().subscription_count(), 0);
    }

    #[test]
    fn schema_version_stamped_on_open() {
        let f = NamedTempFile::new().unwrap();
        // First open: fresh database, stamps version.
        drop(Storage::open(f.path()).unwrap());
        // Second open: reads the stamped version, must succeed.
        drop(Storage::open(f.path()).unwrap());
        // Third open: same, must also succeed.
        drop(Storage::open(f.path()).unwrap());
    }

    #[test]
    fn reset_clears_data_and_allows_reopen() {
        let (_f, storage) = temp_db();
        let world = sample_world();
        storage.save_world(&world).unwrap();
        assert_eq!(storage.table_counts().unwrap().loci, 2);

        storage.reset().unwrap();
        drop(storage);

        // After reset, open() must succeed (version is current).
        let storage2 = Storage::open(_f.path()).unwrap();
        assert_eq!(storage2.table_counts().unwrap().loci, 0);
        assert!(matches!(storage2.load_world(), Err(StorageError::Empty)));
    }
}
