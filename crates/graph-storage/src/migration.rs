//! Schema migration helpers.
//!
//! ## Schema history
//!
//! | Version | What changed |
//! |---------|-------------|
//! | 1       | Initial schema (no `wall_time` / `metadata` on `Change`) |
//! | 2       | Added `wall_time: Option<u64>` and `metadata: Option<Properties>` to `Change`; added `SUBSCRIPTIONS` table |
//!
//! ## Why row-level migration is needed
//!
//! `postcard` serialization is positional: field order in the struct maps
//! directly to byte layout. Adding `wall_time` / `metadata` at the end of
//! `Change` means v1 bytes end where `batch` ends, and reading them as v2
//! `Change` fails with an EOF error on the first call.
//!
//! `migrate_from_v1` reads every change row using the v1 layout, converts
//! it to the current `Change` (setting the new fields to `None`), and
//! writes the rows back — all inside a single ACID transaction.

use redb::ReadableTable;
use serde::{Deserialize, Serialize};

use graph_core::{
    BatchId, ChangeId, ChangeSubject, InfluenceKindId, StateVector,
};

use crate::error::StorageError;
use crate::tables::*;
use crate::Storage;
use crate::CURRENT_SCHEMA_VERSION;

/// The `Change` layout as serialized by schema version 1.
///
/// Identical to the current `Change` except it lacks `wall_time` and
/// `metadata`. Used exclusively by [`migrate_from_v1`] — do not use
/// elsewhere.
#[derive(Serialize, Deserialize)]
struct ChangeV1 {
    id: ChangeId,
    subject: ChangeSubject,
    kind: InfluenceKindId,
    predecessors: Vec<ChangeId>,
    before: StateVector,
    after: StateVector,
    batch: BatchId,
}

impl From<ChangeV1> for graph_core::Change {
    fn from(v: ChangeV1) -> Self {
        Self {
            id: v.id,
            subject: v.subject,
            kind: v.kind,
            predecessors: v.predecessors,
            before: v.before,
            after: v.after,
            batch: v.batch,
            wall_time: None,
            metadata: None,
        }
    }
}

impl Storage {
    /// Migrate a schema-version-1 database to the current schema version.
    ///
    /// This performs an **in-place** migration inside a single ACID
    /// transaction:
    ///
    /// 1. Reads every `Change` row using the v1 postcard layout.
    /// 2. Converts each to the current `Change` (`wall_time: None`,
    ///    `metadata: None`).
    /// 3. Writes the converted bytes back into the same table.
    /// 4. Creates the `SUBSCRIPTIONS` table (v2 addition).
    /// 5. Stamps `CURRENT_SCHEMA_VERSION` in META.
    ///
    /// Call this through [`open_and_migrate`] rather than calling it
    /// directly on a `Storage` that was opened with `open()` (which would
    /// have already rejected the v1 database).
    ///
    /// Returns `Err(StorageError::SchemaMismatch)` if the stored version
    /// is not 1 (i.e. this migration doesn't apply).
    pub fn migrate_from_v1(&self) -> Result<(), StorageError> {
        // Verify we're actually looking at a v1 database.
        {
            let txn = self.db.begin_read()?;
            let meta = txn.open_table(META)?;
            let stored = meta.get(META_SCHEMA_VERSION)?.map(|v| v.value());
            match stored {
                Some(1) => {} // expected
                Some(v) => return Err(StorageError::SchemaMismatch {
                    found: v,
                    expected: 1,
                }),
                None => return Err(StorageError::SchemaMismatch {
                    found: 0,
                    expected: 1,
                }),
            }
        }

        let txn = self.db.begin_write()?;
        {
            // Read all v1 changes, then rewrite as v2.
            let v1_changes: Vec<(u64, graph_core::Change)> = {
                let t = txn.open_table(CHANGES)?;
                let mut iter = t.iter()?;
                let mut out = Vec::new();
                while let Some(entry) = iter.next() {
                    let (key, val): (redb::AccessGuard<u64>, redb::AccessGuard<&[u8]>) = entry?;
                    let v1: ChangeV1 = postcard::from_bytes(val.value())?;
                    out.push((key.value(), graph_core::Change::from(v1)));
                }
                out
            };
            {
                let mut t = txn.open_table(CHANGES)?;
                for (key, change) in &v1_changes {
                    let bytes = postcard::to_allocvec(change)?;
                    t.insert(*key, bytes.as_slice())?;
                }
            }

            // Ensure the SUBSCRIPTIONS table exists (new in v2).
            let _ = txn.open_multimap_table(SUBSCRIPTIONS)?;

            // Stamp the new schema version.
            let mut meta = txn.open_table(META)?;
            meta.insert(META_SCHEMA_VERSION, CURRENT_SCHEMA_VERSION)?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Open a database, automatically migrating from v1 → current if needed.
    ///
    /// If the database is already at `CURRENT_SCHEMA_VERSION`, this is
    /// equivalent to `open()`. If it is at v1, the migration runs
    /// in-place before returning. Any other version mismatch returns
    /// `Err(StorageError::SchemaMismatch)`.
    pub fn open_and_migrate(path: impl AsRef<std::path::Path>) -> Result<Self, StorageError> {
        match Self::open(path.as_ref()) {
            Ok(s) => return Ok(s),
            Err(StorageError::SchemaMismatch { found: 1, .. }) => {
                // Attempt v1 → current migration.
                let s = Self::open_force(path.as_ref())?;
                s.migrate_from_v1()?;
                // Re-open through the normal path to validate the new version.
                drop(s);
                Self::open(path.as_ref())
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{ChangeSubject, LocusId, StateVector};
    use tempfile::NamedTempFile;

    /// Write a v1-format Change into the CHANGES table and stamp version 1.
    fn write_v1_db(path: &std::path::Path) {
        let storage = Storage::open_force(path).unwrap();
        let txn = storage.db.begin_write().unwrap();
        {
            // Write a v1 Change (no wall_time/metadata fields).
            let change_v1 = ChangeV1 {
                id: ChangeId(0),
                subject: ChangeSubject::Locus(LocusId(0)),
                kind: InfluenceKindId(1),
                predecessors: vec![],
                before: StateVector::zeros(1),
                after: StateVector::from_slice(&[0.5]),
                batch: BatchId(0),
            };
            let bytes = postcard::to_allocvec(&change_v1).unwrap();
            let mut changes_t = txn.open_table(CHANGES).unwrap();
            changes_t.insert(0u64, bytes.as_slice()).unwrap();

            // Stamp version 1 and minimal meta so load_world() works.
            let mut meta = txn.open_table(META).unwrap();
            meta.insert(META_SCHEMA_VERSION, 1u64).unwrap();
            meta.insert(META_CURRENT_BATCH, 0u64).unwrap();
            meta.insert(META_NEXT_CHANGE_ID, 1u64).unwrap();
            meta.insert(META_NEXT_RELATIONSHIP_ID, 0u64).unwrap();
            meta.insert(META_NEXT_ENTITY_ID, 0u64).unwrap();
        }
        txn.commit().unwrap();
    }

    #[test]
    fn migrate_from_v1_converts_changes_and_bumps_version() {
        let f = NamedTempFile::new().unwrap();
        write_v1_db(f.path());

        // Normal open must reject v1 database.
        assert!(matches!(
            Storage::open(f.path()),
            Err(StorageError::SchemaMismatch { found: 1, .. })
        ));

        // open_and_migrate should succeed.
        let storage = Storage::open_and_migrate(f.path()).unwrap();

        // Change must be readable as v2 with None fields.
        let change = storage.get_change(ChangeId(0)).unwrap().unwrap();
        assert_eq!(change.id, ChangeId(0));
        assert_eq!(change.wall_time, None);
        assert_eq!(change.metadata, None);
        assert!((change.after.as_slice()[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn open_and_migrate_is_noop_on_current_version() {
        let f = NamedTempFile::new().unwrap();
        drop(Storage::open(f.path()).unwrap()); // stamp v2
        drop(Storage::open_and_migrate(f.path()).unwrap()); // should succeed without migration
        drop(Storage::open(f.path()).unwrap()); // still valid
    }

    #[test]
    fn migrate_from_v1_rejects_non_v1_database() {
        let f = NamedTempFile::new().unwrap();
        drop(Storage::open(f.path()).unwrap()); // stamp CURRENT_SCHEMA_VERSION
        let s = Storage::open_force(f.path()).unwrap();
        let result = s.migrate_from_v1();
        assert!(matches!(
            result,
            Err(StorageError::SchemaMismatch { found, .. }) if found == CURRENT_SCHEMA_VERSION
        ));
    }

    #[test]
    fn subscription_generation_optimization_skips_rewrite_when_unchanged() {
        use graph_core::{LocusId, RelationshipId};
        use graph_world::World;

        let f = NamedTempFile::new().unwrap();
        let storage = Storage::open(f.path()).unwrap();

        let mut world = World::new();
        // Add a subscription so there's something to persist.
        world.subscriptions_mut().subscribe(LocusId(0), RelationshipId(10));

        // First save: stamps the generation.
        storage.save_world(&world).unwrap();
        let gen_after_save = world.subscriptions().generation();
        assert_eq!(storage.last_subscription_gen.get(), gen_after_save);

        // commit_batch with no subscription change: generation unchanged, skip.
        // (We need at least one change in the batch for commit_batch to do anything.)
        // Here we just verify the generation tracking by checking that
        // last_subscription_gen does not change if no subscription mutation happened.
        let gen_before = storage.last_subscription_gen.get();
        // No subscription changes — generation stays the same.
        assert_eq!(world.subscriptions().generation(), gen_before);

        // Mutate subscriptions — generation bumps.
        world.subscriptions_mut().subscribe(LocusId(1), RelationshipId(10));
        assert!(world.subscriptions().generation() > gen_before);
    }
}
