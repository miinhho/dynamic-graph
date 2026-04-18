// redb iterators yield Result on .next(), making `for` loops impossible.
#![allow(clippy::while_let_on_iterator)]

//! redb-backed world storage.

mod access;
mod batch;
mod error;
mod snapshot;
mod tables;
mod util;

pub use error::StorageError;

use std::cell::Cell;
use std::path::Path;

#[cfg(test)]
use graph_world::World;
use redb::{Database, ReadableTable};

use tables::*;

pub(crate) const CURRENT_SCHEMA_VERSION: u64 = 1;

pub struct Storage {
    db: Database,
    last_subscription_gen: Cell<u64>,
    last_relationship_gen: Cell<u64>,
    last_entity_gen: Cell<u64>,
}

struct SchemaSetup<'a> {
    txn: &'a redb::WriteTransaction,
}

impl Storage {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let db = Database::create(path.as_ref())?;
        let txn = db.begin_write()?;
        {
            Self::initialize_schema(&txn)?;
        }
        txn.commit()?;

        Ok(Self {
            db,
            last_subscription_gen: Cell::new(u64::MAX),
            last_relationship_gen: Cell::new(u64::MAX),
            last_entity_gen: Cell::new(u64::MAX),
        })
    }

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

    pub fn reset(&self) -> Result<(), StorageError> {
        let txn = self.db.begin_write()?;
        {
            self.clear_world_tables(&txn)?;
            SchemaSetup::new(&txn).reset_meta()?;
        }
        txn.commit()?;
        Ok(())
    }

    fn initialize_schema(txn: &redb::WriteTransaction) -> Result<(), StorageError> {
        SchemaSetup::new(txn).initialize()
    }
}

impl<'a> SchemaSetup<'a> {
    fn new(txn: &'a redb::WriteTransaction) -> Self {
        Self { txn }
    }

    fn initialize(&self) -> Result<(), StorageError> {
        self.open_tables()?;
        self.ensure_schema_version()
    }

    fn open_tables(&self) -> Result<(), StorageError> {
        let _ = self.txn.open_table(LOCI)?;
        let _ = self.txn.open_table(RELATIONSHIPS)?;
        let _ = self.txn.open_table(ENTITIES)?;
        let _ = self.txn.open_table(CHANGES)?;
        let _ = self.txn.open_multimap_table(CHANGES_BY_BATCH)?;
        let _ = self.txn.open_table(PROPERTIES)?;
        let _ = self.txn.open_table(NAMES)?;
        let _ = self.txn.open_table(ALIASES)?;
        let _ = self.txn.open_multimap_table(SUBSCRIPTIONS)?;
        let _ = self.txn.open_multimap_table(REL_BY_LOCUS)?;
        let _ = self.txn.open_table(BCM_THRESHOLDS)?;
        let _ = self.txn.open_table(META)?;
        Ok(())
    }

    fn ensure_schema_version(&self) -> Result<(), StorageError> {
        let mut meta = self.txn.open_table(META)?;
        let stored_version = meta.get(META_SCHEMA_VERSION)?.map(|v| v.value());
        match stored_version {
            None => {
                meta.insert(META_SCHEMA_VERSION, CURRENT_SCHEMA_VERSION)?;
                Ok(())
            }
            Some(v) if v == CURRENT_SCHEMA_VERSION => Ok(()),
            Some(v) => Err(StorageError::SchemaMismatch {
                found: v,
                expected: CURRENT_SCHEMA_VERSION,
            }),
        }
    }

    fn reset_meta(&self) -> Result<(), StorageError> {
        let mut meta = self.txn.open_table(META)?;
        util::pop_all_rows(&mut meta)?;
        meta.insert(META_SCHEMA_VERSION, CURRENT_SCHEMA_VERSION)?;
        Ok(())
    }
}

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
    use graph_core::{Locus, LocusId, LocusKindId, RelationshipId, StateVector, props};
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
        world.insert_locus(Locus::new(
            id1,
            LocusKindId(1),
            StateVector::from_slice(&[0.5, 1.0]),
        ));
        world
            .properties_mut()
            .insert(id0, props! { "name" => "Alpha", "score" => 0.8_f64 });
        world
            .properties_mut()
            .insert(id1, props! { "name" => "Beta" });
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
            restored
                .properties()
                .get(LocusId(0))
                .unwrap()
                .get_str("name"),
            Some("Alpha")
        );
        assert_eq!(
            restored
                .properties()
                .get(LocusId(0))
                .unwrap()
                .get_f64("score"),
            Some(0.8)
        );
        assert_eq!(
            restored
                .properties()
                .get(LocusId(1))
                .unwrap()
                .get_str("name"),
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
        world2.insert_locus(Locus::new(
            LocusId(99),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        storage.save_world(&world2).unwrap();

        let counts = storage.table_counts().unwrap();
        assert_eq!(counts.loci, 1);
        let loaded = storage.load_world().unwrap();
        assert!(loaded.locus(LocusId(99)).is_some());
        assert!(loaded.locus(LocusId(0)).is_none());
    }

    #[test]
    fn change_with_wall_time_and_metadata_round_trips() {
        use graph_core::{
            BatchId, Change, ChangeId, ChangeSubject, InfluenceKindId, StateVector, props,
        };

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
    fn bcm_thresholds_round_trip_via_save_load() {
        let (_f, storage) = temp_db();
        let mut world = sample_world();

        world.bcm_thresholds_mut().insert(LocusId(0), 0.42);
        world.bcm_thresholds_mut().insert(LocusId(1), 1.23);

        storage.save_world(&world).unwrap();

        let restored = storage.load_world().unwrap();
        assert!((restored.bcm_threshold(LocusId(0)) - 0.42).abs() < 1e-6);
        assert!((restored.bcm_threshold(LocusId(1)) - 1.23).abs() < 1e-6);
        // Non-BCM locus returns 0.
        assert_eq!(restored.bcm_threshold(LocusId(99)), 0.0);
    }

    #[test]
    fn bcm_thresholds_survive_snapshot_round_trip() {
        let mut world = sample_world();
        world.bcm_thresholds_mut().insert(LocusId(0), 0.5);

        let snapshot = world.to_snapshot();
        let restored = World::from_snapshot(snapshot);
        assert!((restored.bcm_threshold(LocusId(0)) - 0.5).abs() < 1e-6);
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
