//! WAL compaction: triggered when `ChangeLog::trim_before_batch` is called.
//!
//! When the engine trims the change log, old WAL segments become
//! redundant — the checkpoint that covers them is now the authoritative
//! source of that history. Compaction:
//!
//! 1. Writes a fresh checkpoint of the current world state.
//! 2. Deletes WAL segments whose `first_batch` is strictly less than
//!    `retain_from_batch` (those records are now subsumed by the checkpoint).
//!
//! Compaction is called by `WalSyncWriter::compact` or `WalHandle::compact`
//! after the in-memory trim has completed. If compaction fails partway
//! through (e.g., checkpoint write succeeds but a segment deletion fails),
//! the state remains consistent — the checkpoint is valid and the extra
//! segments are harmless on the next recovery.

use std::fs;
use std::path::Path;

use graph_core::BatchId;
use graph_world::World;

use crate::checkpoint::write_checkpoint;
use crate::error::WalError;
use crate::segment::list_segments;

/// Write a new checkpoint of `world` and delete any WAL segment files
/// whose `first_batch < retain_from_batch`.
///
/// This is the compaction step to call after `ChangeLog::trim_before_batch`.
pub fn compact(
    data_dir: &Path,
    world: &World,
    retain_from_batch: BatchId,
) -> Result<(), WalError> {
    // Step 1: Write a fresh checkpoint. This must succeed before we delete
    // anything, so that recovery always has a valid base.
    write_checkpoint(data_dir, world)?;

    // Step 2: Delete stale segments. A segment is stale if its entire
    // batch range is covered by the new checkpoint, i.e.,
    // first_batch < retain_from_batch.
    let segments = list_segments(data_dir).map_err(WalError::Io)?;
    for (first_batch, path) in segments {
        if first_batch.0 < retain_from_batch.0 {
            // Best-effort: log or ignore individual deletion failures.
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::{ensure_wal_dir, segment_path, WAL_SUBDIR};
    use tempfile::TempDir;

    #[test]
    fn compact_removes_stale_segments() {
        let dir = TempDir::new().unwrap();
        ensure_wal_dir(dir.path()).unwrap();
        let wal_dir = dir.path().join(WAL_SUBDIR);

        // Create fake segment files at batch 0, 100, 200.
        fs::write(segment_path(dir.path(), BatchId(0)), b"").unwrap();
        fs::write(segment_path(dir.path(), BatchId(100)), b"").unwrap();
        fs::write(segment_path(dir.path(), BatchId(200)), b"").unwrap();

        let world = World::new();
        compact(dir.path(), &world, BatchId(150)).unwrap();

        let remaining: Vec<_> = fs::read_dir(&wal_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        // batch 0 and batch 100 (< 150) should be removed; batch 200 kept.
        assert_eq!(remaining.len(), 1);
        let name = remaining[0].file_name();
        assert!(name.to_str().unwrap().contains("00000000000000c8")); // 200 = 0xc8
    }

    #[test]
    fn compact_writes_checkpoint() {
        let dir = TempDir::new().unwrap();
        let world = World::new();
        compact(dir.path(), &world, BatchId(0)).unwrap();
        assert!(dir.path().join("checkpoint.bin").exists());
    }
}
