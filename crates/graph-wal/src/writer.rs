//! WAL writers: synchronous and write-behind (async background thread).
//!
//! ## Synchronous writer (`WalSyncWriter`)
//!
//! Serializes and flushes each batch record on the calling thread.
//! Simple, deterministic, suitable for tests and low-throughput use cases.
//!
//! ## Write-behind writer (`WalHandle`)
//!
//! Serializes the batch record synchronously (fast), then hands the
//! encoded bytes to a background `std::thread` via a bounded channel.
//! The background thread writes to the file and calls `sync_data()`.
//!
//! If the channel is full (backpressure), `write_batch()` returns
//! `Err(WalError::BackpressureFull)` — the caller may flush and retry,
//! or simply log the warning and continue (accepting potential data loss
//! in the most recent batch on crash).
//!
//! On drop, a `Flush` sentinel is sent and the caller's thread blocks
//! until the background thread drains the channel and exits cleanly.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use graph_core::BatchId;
use graph_world::World;

use crate::config::WalConfig;
use crate::error::WalError;
use crate::record::{BatchRecord, frame};
use crate::segment::{ensure_wal_dir, segment_path};

// ── batch record builder ─────────────────────────────────────────────────────

/// Extract the `BatchRecord` for the batch that was just committed.
/// `prev_batch` is the batch *before* `world.current_batch()` (i.e., the
/// one whose changes we are persisting — the engine advances the batch id
/// *after* committing, so `world.current_batch()` is already the next batch
/// at call time).
pub fn build_batch_record(world: &World, committed_batch: BatchId) -> BatchRecord {
    // Collect changes for this batch.
    let changes = world.log().batch(committed_batch).cloned().collect();

    // Collect relationships whose lineage.last_touched_by is a change from
    // this batch, or that were created in this batch.
    let touched_relationships: Vec<_> = world
        .relationships()
        .iter()
        .filter(|r| {
            r.lineage.created_by
                .or(r.lineage.last_touched_by)
                .map(|cid| world.log().get(cid).map(|c| c.batch == committed_batch).unwrap_or(false))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    // Collect entities whose top layer was deposited in this batch.
    let touched_entities: Vec<_> = world
        .entities()
        .iter()
        .filter(|e| e.layers.last().map(|l| l.batch == committed_batch).unwrap_or(false))
        .cloned()
        .collect();

    BatchRecord {
        changes,
        touched_relationships,
        touched_entities,
        meta: world.world_meta(),
    }
}

// ── synchronous writer ───────────────────────────────────────────────────────

/// Synchronous WAL writer. Writes one segment file per `segment_batch_interval`
/// batches, flushing and syncing to disk after every batch record.
pub struct WalSyncWriter {
    data_dir: PathBuf,
    segment_batch_interval: u64,
    /// The `BatchId` at which the current segment file started.
    current_segment_first_batch: BatchId,
    file: BufWriter<File>,
}

impl WalSyncWriter {
    /// Open or create the WAL, starting a fresh segment at `start_batch`.
    pub fn open(config: &WalConfig, start_batch: BatchId) -> Result<Self, WalError> {
        ensure_wal_dir(&config.data_dir)?;
        let path = segment_path(&config.data_dir, start_batch);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            data_dir: config.data_dir.clone(),
            segment_batch_interval: config.segment_batch_interval,
            current_segment_first_batch: start_batch,
            file: BufWriter::new(file),
        })
    }

    /// Write WAL records for all batches committed since `prev_batch`.
    ///
    /// A single `Engine::tick` may commit multiple batches
    /// (`prev_batch..world.current_batch()`). This method writes one
    /// `BatchRecord` per committed batch so recovery can replay each
    /// batch individually.
    ///
    /// Typical call site (after each `sim.step()` or `engine.tick()`):
    /// ```ignore
    /// let prev = world.current_batch(); // captured before tick
    /// sim.step(stimuli);
    /// writer.write_tick(&sim.world, prev)?;
    /// ```
    pub fn write_tick(&mut self, world: &World, prev_batch: BatchId) -> Result<(), WalError> {
        let current = world.current_batch();
        for batch_idx in prev_batch.0..current.0 {
            let committed_batch = BatchId(batch_idx);
            if committed_batch.0 >= self.current_segment_first_batch.0 + self.segment_batch_interval {
                self.roll_segment(committed_batch)?;
            }
            let record = build_batch_record(world, committed_batch);
            frame::write_one(&mut self.file, &record)?;
        }
        self.file.flush()?;
        self.file.get_ref().sync_data()?;
        Ok(())
    }

    fn roll_segment(&mut self, new_first_batch: BatchId) -> Result<(), WalError> {
        self.file.flush()?;
        let path = segment_path(&self.data_dir, new_first_batch);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        self.current_segment_first_batch = new_first_batch;
        self.file = BufWriter::new(file);
        Ok(())
    }

    /// Flush and sync the current segment without rolling.
    pub fn flush(&mut self) -> Result<(), WalError> {
        self.file.flush()?;
        self.file.get_ref().sync_data()?;
        Ok(())
    }
}

// ── write-behind handle ──────────────────────────────────────────────────────

enum BgMsg {
    Write(Vec<u8>, BatchId),
    Flush(mpsc::SyncSender<Result<(), WalError>>),
    Shutdown,
}

/// Write-behind WAL handle. Serializes records on the calling thread
/// (fast), delegates actual I/O to a background thread.
pub struct WalHandle {
    tx: mpsc::SyncSender<BgMsg>,
    thread: Option<thread::JoinHandle<()>>,
}

impl WalHandle {
    /// Spawn the background writer thread and return the handle.
    pub fn open(config: &WalConfig, start_batch: BatchId) -> Result<Self, WalError> {
        ensure_wal_dir(&config.data_dir)?;

        let data_dir = config.data_dir.clone();
        let interval = config.segment_batch_interval;
        let (tx, rx) = mpsc::sync_channel::<BgMsg>(config.channel_capacity);

        let data_dir_bg = data_dir.clone();
        let thread = thread::Builder::new()
            .name("graph-wal-writer".to_string())
            .spawn(move || {
                bg_writer(rx, data_dir_bg, interval, start_batch);
            })
            .map_err(WalError::Io)?;

        Ok(Self {
            tx,
            thread: Some(thread),
        })
    }

    /// Serialize WAL records for all batches committed since `prev_batch`
    /// and enqueue them for background I/O. Returns `Err(BackpressureFull)`
    /// if the channel is at capacity for any record.
    pub fn write_tick(&self, world: &World, prev_batch: BatchId) -> Result<(), WalError> {
        let current = world.current_batch();
        for batch_idx in prev_batch.0..current.0 {
            let committed_batch = BatchId(batch_idx);
            let record = build_batch_record(world, committed_batch);
            let encoded = frame::encode(&record)?;
            self.tx
                .try_send(BgMsg::Write(encoded, committed_batch))
                .map_err(|_| WalError::BackpressureFull)?;
        }
        Ok(())
    }

    /// Block until all previously enqueued writes have been flushed to disk.
    pub fn flush(&self) -> Result<(), WalError> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.tx
            .send(BgMsg::Flush(ack_tx))
            .map_err(|_| WalError::WriterGone)?;
        ack_rx.recv().map_err(|_| WalError::WriterGone)?
    }
}

impl Drop for WalHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(BgMsg::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn bg_writer(
    rx: mpsc::Receiver<BgMsg>,
    data_dir: PathBuf,
    interval: u64,
    start_batch: BatchId,
) {
    let path = segment_path(&data_dir, start_batch);
    let file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(_) => return, // can't open, silently exit (error logged by open caller)
    };
    let mut writer = BufWriter::new(file);
    let mut current_first = start_batch;

    for msg in rx {
        match msg {
            BgMsg::Write(bytes, batch) => {
                // Roll segment if needed.
                if batch.0 >= current_first.0 + interval {
                    let _ = writer.flush();
                    let new_path = segment_path(&data_dir, batch);
                    match OpenOptions::new().create(true).append(true).open(&new_path) {
                        Ok(f) => {
                            writer = BufWriter::new(f);
                            current_first = batch;
                        }
                        Err(_) => continue,
                    }
                }
                let _ = writer.write_all(&bytes);
                let _ = writer.flush();
                let _ = writer.get_ref().sync_data();
            }
            BgMsg::Flush(ack) => {
                let result = writer
                    .flush()
                    .map_err(WalError::Io)
                    .and_then(|_| writer.get_ref().sync_data().map_err(WalError::Io));
                let _ = ack.send(result);
            }
            BgMsg::Shutdown => break,
        }
    }
    let _ = writer.flush();
}
