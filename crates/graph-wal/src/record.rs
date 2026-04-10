//! WAL record types — the unit of WAL I/O.
//!
//! Each `BatchRecord` captures everything needed to reconstruct one
//! committed batch without re-running locus programs:
//! - all `Change`s committed in the batch (the canonical change log),
//! - all `Relationship` records touched (inserted or updated) during the
//!   batch (full records — simpler than deltas for the initial impl),
//! - any `Entity` records that had new layers deposited,
//! - the `WorldMeta` counters after the batch commits.
//!
//! Frame format on disk (per record):
//! ```text
//! [payload_len: u32 LE] [crc32: u32 LE] [postcard payload: payload_len bytes]
//! ```
//! The CRC covers the payload bytes only. A torn write (partial payload,
//! failed CRC, or truncated frame header) causes recovery to stop at
//! that record and report a `RecoveryWarning::TornRecord`.

use graph_core::{Change, Entity, Relationship};
use graph_world::WorldMeta;
use serde::{Deserialize, Serialize};

/// A single WAL record written after each committed batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRecord {
    /// All changes committed in this batch, in commit order.
    pub changes: Vec<Change>,
    /// Full state of every relationship touched (created or updated)
    /// during this batch. Recovery uses these to rebuild `RelationshipStore`
    /// without replaying program logic.
    pub touched_relationships: Vec<Relationship>,
    /// Full state of every entity whose layer stack changed during this
    /// batch (new layer deposited, status changed, lineage updated).
    pub touched_entities: Vec<Entity>,
    /// World counter snapshot *after* this batch fully commits.
    pub meta: WorldMeta,
}

/// Framed wire encoding helpers.
///
/// On-disk frame layout (little-endian):
/// ```text
/// [4 bytes: payload length as u32 LE]
/// [4 bytes: CRC-32 of payload bytes]
/// [N bytes: postcard-encoded BatchRecord]
/// ```
pub mod frame {
    use super::*;
    use crate::error::WalError;
    use std::io::{Read, Write};

    const HEADER_LEN: usize = 8; // 4 (len) + 4 (crc)

    pub fn encode(record: &BatchRecord) -> Result<Vec<u8>, WalError> {
        let payload = postcard::to_allocvec(record)?;
        let crc = crc32fast::hash(&payload);
        let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(&crc.to_le_bytes());
        frame.extend_from_slice(&payload);
        Ok(frame)
    }

    /// Read one frame from `reader`. Returns `None` on clean EOF (no
    /// bytes read). Returns an error on partial reads or CRC mismatch.
    pub fn read_one<R: Read>(reader: &mut R) -> Result<Option<BatchRecord>, WalError> {
        let mut header = [0u8; HEADER_LEN];
        match reader.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Check if we read zero bytes (clean EOF) vs partial header
                // (torn write). `read_exact` returns UnexpectedEof for both.
                // We treat it as a torn record — the caller decides severity.
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        }
        let payload_len = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
        let stored_crc = u32::from_le_bytes(header[4..8].try_into().unwrap());

        let mut payload = vec![0u8; payload_len];
        reader.read_exact(&mut payload).map_err(|_| {
            WalError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "torn WAL frame: payload shorter than declared length",
            ))
        })?;

        let actual_crc = crc32fast::hash(&payload);
        if actual_crc != stored_crc {
            return Err(WalError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("CRC mismatch: stored={stored_crc:#010x} actual={actual_crc:#010x}"),
            )));
        }

        let record: BatchRecord = postcard::from_bytes(&payload)?;
        Ok(Some(record))
    }

    pub fn write_one<W: Write>(writer: &mut W, record: &BatchRecord) -> Result<(), WalError> {
        let frame = encode(record)?;
        writer.write_all(&frame)?;
        Ok(())
    }
}
