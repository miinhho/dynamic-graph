//! graph-wal: Write-Ahead Log and checkpoint persistence for the substrate.
//!
//! ## Overview
//!
//! Provides crash-safe persistence for a `World`. The WAL records every
//! committed batch to disk; a periodic checkpoint captures a full world
//! snapshot to bound recovery time.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use graph_wal::{WalConfig, WalSyncWriter, recover};
//! use graph_core::BatchId;
//!
//! // --- writer side (during normal engine operation) ---
//! # let world = graph_world::World::new();
//! let config = WalConfig::new("/tmp/my_wal");
//! let mut writer = WalSyncWriter::open(&config, BatchId(0)).unwrap();
//! // After each Engine::tick() or Simulation::step():
//! //   writer.write_batch(&world, committed_batch_id)?;
//!
//! // --- recovery side (on restart) ---
//! let recovery = recover(std::path::Path::new("/tmp/my_wal")).unwrap();
//! let world = recovery.world;
//! for warning in &recovery.warnings {
//!     eprintln!("WAL recovery warning: {warning:?}");
//! }
//! ```
//!
//! ## Persistence scope
//!
//! The following are persisted:
//! - All `Change` records (the backbone; everything else derives from these).
//! - All `Locus` registrations (id, kind, initial state).
//! - `Relationship` state (activity, weight, `last_decayed_batch`).
//! - `Entity` sediment layers and lineage.
//! - World counter metadata (`BatchId`, `ChangeId`, etc.).
//!
//! The following are *not* persisted (re-supplied by the caller at startup):
//! - `LocusKindRegistry` (user re-registers programs).
//! - `InfluenceKindRegistry` (user re-registers configs).
//! - `CohereStore` (ephemeral — recomputed on demand).
//!
//! ## Crash safety
//!
//! - Checkpoint writes are atomic (staged via a temp file + rename).
//! - WAL records are framed with a CRC-32. A torn write at crash time
//!   causes recovery to stop at that record and report a
//!   `RecoveryWarning::TornRecord`. The world is consistent up to the
//!   last fully written batch.
//! - Maximum data loss: at most one batch (the batch that was committed
//!   in memory but not yet flushed to the WAL file at crash time).
//!
//! ## Compaction
//!
//! Call `compact()` after `ChangeLog::trim_before_batch()` to write a
//! fresh checkpoint and delete stale WAL segment files.

pub mod checkpoint;
pub mod compaction;
pub mod config;
pub mod error;
pub mod record;
pub mod recovery;
pub mod segment;
pub mod writer;

pub use checkpoint::{load_checkpoint, snapshot_from_world, write_checkpoint};
pub use compaction::compact;
pub use config::WalConfig;
pub use error::WalError;
pub use record::BatchRecord;
pub use recovery::{recover, Recovery, RecoveryWarning};
pub use writer::{WalHandle, WalSyncWriter};
