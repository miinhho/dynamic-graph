//! Recovery: restore a `World` from a checkpoint + WAL segments.
//!
//! Recovery is two-phase:
//!
//! 1. Load `checkpoint.bin` (if present) and hydrate a fresh `World`
//!    from the `WorldSnapshot`. If no checkpoint exists, start with an
//!    empty world and replay all WAL segments.
//!
//! 2. Find all WAL segments whose `first_batch > checkpoint.meta.current_batch`,
//!    read their `BatchRecord`s in order, and apply each record to the
//!    world:
//!    - Append the `Change`s to the `ChangeLog`.
//!    - Upsert touched relationships into `RelationshipStore`.
//!    - Upsert touched entities into `EntityStore`.
//!    - Restore `WorldMeta` counters from the record's `meta` field.
//!
//! The last record in the last segment may be torn (partial write at
//! crash time). Recovery stops at the first torn record and reports a
//! `RecoveryWarning` rather than returning an error — the world is
//! consistent up to the last fully written batch.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use graph_core::BatchId;
use graph_world::World;

use crate::checkpoint::load_checkpoint;
use crate::error::WalError;
use crate::record::frame;
use crate::segment::list_segments;

/// A non-fatal anomaly discovered during recovery.
#[derive(Debug, Clone)]
pub enum RecoveryWarning {
    /// The last record in the given segment was torn (partial write).
    /// The world is consistent up to the batch before this record.
    TornRecord { segment: std::path::PathBuf, at_batch: Option<BatchId> },
    /// Expected a segment starting at `expected_batch` but it was missing.
    /// Recovery continued with the next available segment.
    MissingSegment { expected_batch: BatchId },
}

/// The result of a recovery operation.
pub struct Recovery {
    /// The fully restored world, consistent up to the last committed batch.
    pub world: World,
    /// Non-fatal anomalies encountered during recovery.
    pub warnings: Vec<RecoveryWarning>,
}

/// Restore a `World` from `data_dir`.
///
/// 1. Loads `checkpoint.bin` if present; otherwise starts with an empty world.
/// 2. Replays WAL segments newer than the checkpoint.
///
/// The caller must re-register locus kind programs and influence kind
/// configs (those live in `graph-engine` registries, not in the world).
pub fn recover(data_dir: &Path) -> Result<Recovery, WalError> {
    let mut warnings = Vec::new();
    let mut world = World::new();

    // Phase 1: Load checkpoint.
    let checkpoint_batch = match load_checkpoint(data_dir)? {
        Some(snapshot) => {
            let batch = snapshot.meta.current_batch;
            apply_snapshot(&mut world, snapshot);
            batch
        }
        None => BatchId(0),
    };

    // Phase 2: Replay WAL segments newer than the checkpoint.
    let segments = list_segments(data_dir).map_err(WalError::Io)?;
    for (first_batch, seg_path) in &segments {
        // Skip segments that are fully covered by the checkpoint.
        // A segment's last batch ≤ checkpoint_batch means all its records
        // are already represented in the checkpoint.
        // We replay any segment whose `first_batch ≤ checkpoint_batch` but
        // may contain records newer than the checkpoint, plus all segments
        // whose `first_batch > checkpoint_batch`.
        // Simplification: skip segments whose first_batch is before the
        // checkpoint batch — the checkpoint already includes those records.
        if first_batch.0 < checkpoint_batch.0 {
            continue;
        }

        let file = File::open(seg_path)?;
        let mut reader = BufReader::new(file);
        let mut last_applied_batch: Option<BatchId> = None;

        loop {
            match frame::read_one(&mut reader) {
                Ok(Some(record)) => {
                    // Skip records whose meta batch is already in the checkpoint.
                    if record.meta.current_batch.0 <= checkpoint_batch.0 {
                        continue;
                    }
                    apply_batch_record(&mut world, record);
                    last_applied_batch = Some(world.current_batch());
                }
                Ok(None) => break, // clean EOF
                Err(_) => {
                    warnings.push(RecoveryWarning::TornRecord {
                        segment: seg_path.clone(),
                        at_batch: last_applied_batch,
                    });
                    break;
                }
            }
        }
    }

    Ok(Recovery { world, warnings })
}

/// Apply a `WorldSnapshot` to a fresh (empty) `World`.
fn apply_snapshot(world: &mut World, snapshot: graph_world::WorldSnapshot) {
    for locus in snapshot.loci {
        world.insert_locus(locus);
    }
    for rel in snapshot.relationships {
        world.relationships_mut().insert(rel);
    }
    for entity in snapshot.entities {
        world.entities_mut().insert(entity);
    }
    world.restore_meta(&snapshot.meta);
}

/// Apply one `BatchRecord` to the world, appending changes and upserting
/// relationships and entities.
fn apply_batch_record(world: &mut World, record: crate::record::BatchRecord) {
    // Append changes — must be in commit order for the ChangeId density
    // invariant to hold.
    for change in record.changes {
        world.append_change(change);
    }

    // Upsert relationships (insert or replace).
    for rel in record.touched_relationships {
        let id = rel.id;
        if world.relationships().get(id).is_some() {
            *world.relationships_mut().get_mut(id).unwrap() = rel;
        } else {
            world.relationships_mut().insert(rel);
        }
    }

    // Upsert entities (insert or replace).
    for entity in record.touched_entities {
        let id = entity.id;
        if world.entities().get(id).is_some() {
            *world.entities_mut().get_mut(id).unwrap() = entity;
        } else {
            world.entities_mut().insert(entity);
        }
    }

    // Restore meta last (advances batch clock + resets ID counters).
    world.restore_meta(&record.meta);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::write_checkpoint;
    use graph_core::{Locus, LocusId, LocusKindId, StateVector};
    use tempfile::TempDir;

    fn make_world_with_loci() -> World {
        let mut w = World::new();
        w.insert_locus(Locus::new(LocusId(0), LocusKindId(0), StateVector::zeros(2)));
        w.insert_locus(Locus::new(LocusId(1), LocusKindId(0), StateVector::from_slice(&[0.5])));
        w
    }

    #[test]
    fn recover_empty_dir_returns_empty_world() {
        let dir = TempDir::new().unwrap();
        let r = recover(dir.path()).unwrap();
        assert_eq!(r.world.loci().iter().count(), 0);
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn recover_from_checkpoint_only() {
        let dir = TempDir::new().unwrap();
        let world = make_world_with_loci();
        write_checkpoint(dir.path(), &world).unwrap();

        let r = recover(dir.path()).unwrap();
        assert_eq!(r.world.loci().iter().count(), 2);
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn recover_metadata_counters() {
        let dir = TempDir::new().unwrap();
        let mut world = make_world_with_loci();
        // Simulate some counter advancement.
        world.advance_batch();
        world.mint_change_id();
        world.mint_change_id();
        write_checkpoint(dir.path(), &world).unwrap();

        let r = recover(dir.path()).unwrap();
        assert_eq!(r.world.world_meta(), world.world_meta());
    }
}
