//! Checkpoint: full world snapshot written to `checkpoint.bin`.
//!
//! A checkpoint captures the entire mutable world state at a given
//! `BatchId`. Recovery loads the checkpoint first, then replays only
//! the WAL segments that are newer than the checkpoint's batch.
//!
//! File format: same framing as WAL records (4-byte len + 4-byte CRC32 +
//! postcard payload), but the payload is a `WorldSnapshot` instead of a
//! `BatchRecord`. Using the same framing means the checkpoint is verifiable
//! with the same CRC logic.
//!
//! The file is written atomically: the payload is written to a temp file
//! in the same directory, then renamed over the existing checkpoint.

use std::fs;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use graph_world::{World, WorldSnapshot};

use crate::error::WalError;

pub const CHECKPOINT_FILE: &str = "checkpoint.bin";

fn checkpoint_path(data_dir: &Path) -> PathBuf {
    data_dir.join(CHECKPOINT_FILE)
}

fn tmp_checkpoint_path(data_dir: &Path) -> PathBuf {
    data_dir.join("checkpoint.bin.tmp")
}

/// Serialize `snapshot` to a framed postcard blob.
fn encode_snapshot(snapshot: &WorldSnapshot) -> Result<Vec<u8>, WalError> {
    let payload = postcard::to_allocvec(snapshot)?;
    let crc = crc32fast::hash(&payload);
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&crc.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Build a `WorldSnapshot` from the world's current stores.
pub fn snapshot_from_world(world: &World) -> WorldSnapshot {
    WorldSnapshot {
        loci: world.loci().iter().cloned().collect(),
        relationships: world.relationships().iter().cloned().collect(),
        entities: world.entities().iter().cloned().collect(),
        meta: world.world_meta(),
    }
}

/// Write a checkpoint of `world` to `data_dir/checkpoint.bin`.
///
/// The write is atomic: the payload is staged to a temp file and then
/// renamed over the existing checkpoint (or created fresh).
pub fn write_checkpoint(data_dir: &Path, world: &World) -> Result<(), WalError> {
    let snapshot = snapshot_from_world(world);
    let frame = encode_snapshot(&snapshot)?;

    let tmp = tmp_checkpoint_path(data_dir);
    {
        let file = fs::File::create(&tmp)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(&frame)?;
        writer.flush()?;
        writer.into_inner().map_err(|e| e.into_error())?.sync_data()?;
    }
    fs::rename(&tmp, checkpoint_path(data_dir))?;
    Ok(())
}

/// Load and decode a checkpoint from `data_dir/checkpoint.bin`.
///
/// Returns `None` if no checkpoint file exists (fresh start).
/// Returns `Err` if the file exists but is corrupt or unreadable.
pub fn load_checkpoint(data_dir: &Path) -> Result<Option<WorldSnapshot>, WalError> {
    let path = checkpoint_path(data_dir);
    if !path.exists() {
        return Ok(None);
    }

    let file = fs::File::open(&path)?;
    let mut reader = BufReader::new(file);

    let mut header = [0u8; 8];
    std::io::Read::read_exact(&mut reader, &mut header).map_err(|_| {
        WalError::CorruptCheckpoint {
            path: path.clone(),
            reason: "file too short to contain a valid header".to_string(),
        }
    })?;

    let payload_len = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
    let stored_crc = u32::from_le_bytes(header[4..8].try_into().unwrap());

    let mut payload = vec![0u8; payload_len];
    std::io::Read::read_exact(&mut reader, &mut payload).map_err(|_| {
        WalError::CorruptCheckpoint {
            path: path.clone(),
            reason: "payload shorter than declared length".to_string(),
        }
    })?;

    let actual_crc = crc32fast::hash(&payload);
    if actual_crc != stored_crc {
        return Err(WalError::CorruptCheckpoint {
            path,
            reason: format!(
                "CRC mismatch: stored={stored_crc:#010x} actual={actual_crc:#010x}"
            ),
        });
    }

    let snapshot: WorldSnapshot = postcard::from_bytes(&payload).map_err(|e| {
        WalError::CorruptCheckpoint {
            path,
            reason: e.to_string(),
        }
    })?;

    Ok(Some(snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{LocusId, LocusKindId, StateVector};
    use graph_core::Locus;
    use tempfile::TempDir;

    #[test]
    fn roundtrip_empty_world() {
        let dir = TempDir::new().unwrap();
        let world = World::new();
        write_checkpoint(dir.path(), &world).unwrap();
        let snap = load_checkpoint(dir.path()).unwrap().unwrap();
        assert!(snap.loci.is_empty());
        assert!(snap.relationships.is_empty());
        assert!(snap.entities.is_empty());
    }

    #[test]
    fn roundtrip_with_loci() {
        let dir = TempDir::new().unwrap();
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(1), LocusKindId(0), StateVector::zeros(2)));
        world.insert_locus(Locus::new(LocusId(2), LocusKindId(0), StateVector::from_slice(&[0.5, 1.0])));
        write_checkpoint(dir.path(), &world).unwrap();
        let snap = load_checkpoint(dir.path()).unwrap().unwrap();
        assert_eq!(snap.loci.len(), 2);
    }

    #[test]
    fn missing_checkpoint_returns_none() {
        let dir = TempDir::new().unwrap();
        let result = load_checkpoint(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn corrupt_checkpoint_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(CHECKPOINT_FILE);
        fs::write(&path, b"not a valid checkpoint").unwrap();
        let result = load_checkpoint(dir.path());
        assert!(result.is_err());
    }
}
