//! Storage persistence and recovery lifecycle methods.

use graph_world::WorldSnapshot;

use super::Simulation;
use super::config::SimulationConfig;
use crate::registry::{InfluenceKindRegistry, LocusKindRegistry};

impl Simulation {
    /// Restore a `Simulation` from a `WorldSnapshot` (e.g. loaded via
    /// [`World::load`]). The caller must supply the same locus-kind programs
    /// and influence configs that were active when the snapshot was taken.
    pub fn from_snapshot(
        snapshot: WorldSnapshot,
        loci: LocusKindRegistry,
        influences: InfluenceKindRegistry,
        config: SimulationConfig,
    ) -> Self {
        Self::with_config(graph_world::World::from_snapshot(snapshot), loci, influences, config)
    }

    /// Load a `World` from a redb-backed storage file and create a
    /// `Simulation` from it. The caller must re-supply registries.
    #[cfg(feature = "storage")]
    pub fn from_storage(
        path: impl AsRef<std::path::Path>,
        loci: LocusKindRegistry,
        influences: InfluenceKindRegistry,
        config: SimulationConfig,
    ) -> Result<Self, graph_storage::StorageError> {
        let storage = graph_storage::Storage::open(path.as_ref())?;
        let world = storage.load_world()?;
        Ok(Self::with_config(world, loci, influences, config))
    }

    /// Get a reference to the underlying redb storage, if configured.
    #[cfg(feature = "storage")]
    pub fn storage(&self) -> Option<&graph_storage::Storage> {
        self.storage.as_ref()
    }

    /// The most recent storage write error, if any. `None` if storage is
    /// not configured, if no write has failed, or if the last write
    /// succeeded (success clears the stored error).
    #[cfg(feature = "storage")]
    pub fn last_storage_error(&self) -> Option<&graph_storage::StorageError> {
        self.last_storage_error.as_ref()
    }

    /// Persist the full world state to storage in a single ACID transaction.
    /// Useful for creating explicit checkpoints. No-op if storage is not
    /// configured.
    #[cfg(feature = "storage")]
    pub fn save_world(&self) -> Result<(), graph_storage::StorageError> {
        match self.storage {
            Some(ref s) => s.save_world(&self.world),
            None => Ok(()),
        }
    }

    /// Flush all unflushed batches to persistent storage.
    ///
    /// When `auto_commit` is `false`, committed batches accumulate in
    /// memory until this method is called. Calling `flush()` when
    /// `auto_commit` is `true` is a no-op (all batches are already written).
    /// No-op if storage is not configured.
    ///
    /// On error, storage is left in a partially-written state — the
    /// successfully written batches are persisted, but subsequent batches
    /// may not be. Call `flush()` again to retry.
    #[cfg(feature = "storage")]
    pub fn flush(&mut self) -> Result<(), graph_storage::StorageError> {
        let Some(ref storage) = self.storage else {
            return Ok(());
        };
        let current_batch = self.world.current_batch();
        for batch_idx in self.last_flushed_batch.0..current_batch.0 {
            storage.commit_batch(&self.world, graph_core::BatchId(batch_idx))?;
        }
        self.last_flushed_batch = current_batch;
        self.last_storage_error = None;
        Ok(())
    }

    /// Whether batches are automatically persisted after each `step()`.
    ///
    /// Returns `false` when lazy flushing is active (caller must call
    /// `flush()` explicitly). Always returns `true` when storage is not
    /// configured.
    #[cfg(feature = "storage")]
    pub fn auto_commit(&self) -> bool {
        self.auto_commit
    }

    /// Change the auto-commit setting at runtime.
    ///
    /// Switching from `false` → `true` does **not** automatically flush
    /// accumulated unflushed batches — call `flush()` first if needed.
    #[cfg(feature = "storage")]
    pub fn set_auto_commit(&mut self, enabled: bool) {
        self.auto_commit = enabled;
    }
}
