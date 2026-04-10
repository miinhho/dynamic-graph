//! WAL configuration.

use std::path::PathBuf;

/// Configuration for the WAL writer and recovery.
#[derive(Debug, Clone)]
pub struct WalConfig {
    /// Directory where WAL segments and checkpoints are stored.
    /// The directory is created if it does not exist.
    pub data_dir: PathBuf,

    /// Number of batches between WAL segment rollovers. Each segment
    /// holds `segment_batch_interval` batches. Smaller values mean more
    /// frequent rollovers and smaller individual files; larger values
    /// mean less overhead but larger files to replay on crash.
    /// Default: 256.
    pub segment_batch_interval: u64,

    /// Depth of the write-behind channel (number of serialized batch
    /// records that can be queued before the engine thread blocks).
    /// Increase if the engine runs faster than the I/O thread can drain.
    /// Default: 64.
    pub channel_capacity: usize,

    /// If `true`, all writes are flushed synchronously on the calling
    /// thread (no background I/O thread). Useful for testing and for
    /// workloads where latency predictability matters more than throughput.
    /// Default: false.
    pub sync_writes: bool,
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("wal_data"),
            segment_batch_interval: 256,
            channel_capacity: 64,
            sync_writes: false,
        }
    }
}

impl WalConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            ..Self::default()
        }
    }

    pub fn segment_batch_interval(mut self, n: u64) -> Self {
        self.segment_batch_interval = n;
        self
    }

    pub fn sync_writes(mut self, v: bool) -> Self {
        self.sync_writes = v;
        self
    }

    pub fn channel_capacity(mut self, n: usize) -> Self {
        self.channel_capacity = n;
        self
    }
}
