//! WAL segment file management.
//!
//! Segments live in `{data_dir}/wal/` and are named
//! `seg-{first_batch:016x}.wal` where `first_batch` is the `BatchId`
//! of the first record in that segment.
//!
//! A new segment is started when the current segment's batch span
//! reaches `WalConfig::segment_batch_interval`. The segment's first
//! batch is encoded in the filename so the recovery path can order
//! segments and skip those already subsumed by a checkpoint.

use std::fs;
use std::path::{Path, PathBuf};

use graph_core::BatchId;

pub const WAL_SUBDIR: &str = "wal";

/// Return the path for a segment whose first batch is `first_batch`.
pub fn segment_path(data_dir: &Path, first_batch: BatchId) -> PathBuf {
    data_dir
        .join(WAL_SUBDIR)
        .join(format!("seg-{:016x}.wal", first_batch.0))
}

/// Parse the first batch id from a segment filename.
/// Returns `None` if the filename does not match the expected pattern.
pub fn parse_first_batch(path: &Path) -> Option<BatchId> {
    let name = path.file_name()?.to_str()?;
    let hex = name.strip_prefix("seg-")?.strip_suffix(".wal")?;
    let v = u64::from_str_radix(hex, 16).ok()?;
    Some(BatchId(v))
}

/// List all segment files in `data_dir/wal/`, sorted by first_batch ascending.
pub fn list_segments(data_dir: &Path) -> std::io::Result<Vec<(BatchId, PathBuf)>> {
    let wal_dir = data_dir.join(WAL_SUBDIR);
    if !wal_dir.exists() {
        return Ok(Vec::new());
    }
    let mut segments: Vec<(BatchId, PathBuf)> = fs::read_dir(&wal_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let first_batch = parse_first_batch(&path)?;
            Some((first_batch, path))
        })
        .collect();
    segments.sort_by_key(|(b, _)| *b);
    Ok(segments)
}

/// Ensure `data_dir/wal/` exists, creating it if necessary.
pub fn ensure_wal_dir(data_dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(data_dir.join(WAL_SUBDIR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn segment_path_roundtrip() {
        let dir = TempDir::new().unwrap();
        let batch = BatchId(42);
        let path = segment_path(dir.path(), batch);
        let parsed = parse_first_batch(&path).unwrap();
        assert_eq!(parsed, batch);
    }

    #[test]
    fn list_segments_sorted() {
        let dir = TempDir::new().unwrap();
        ensure_wal_dir(dir.path()).unwrap();
        let wal_dir = dir.path().join(WAL_SUBDIR);
        // Create out-of-order segment files.
        fs::write(wal_dir.join("seg-0000000000000100.wal"), b"").unwrap();
        fs::write(wal_dir.join("seg-0000000000000000.wal"), b"").unwrap();
        fs::write(wal_dir.join("seg-0000000000000200.wal"), b"").unwrap();
        let segs = list_segments(dir.path()).unwrap();
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].0, BatchId(0x000));
        assert_eq!(segs[1].0, BatchId(0x100));
        assert_eq!(segs[2].0, BatchId(0x200));
    }
}
