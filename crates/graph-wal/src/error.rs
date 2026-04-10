//! Error types for graph-wal.

use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum WalError {
    Io(io::Error),
    Serialize(postcard::Error),
    /// Checkpoint file is present but cannot be decoded.
    CorruptCheckpoint { path: PathBuf, reason: String },
    /// A WAL segment file is present but has an invalid header.
    CorruptSegment { path: PathBuf, reason: String },
    /// Write-behind channel is at capacity; caller should flush and retry.
    BackpressureFull,
    /// Background writer thread panicked or was dropped unexpectedly.
    WriterGone,
}

impl std::fmt::Display for WalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalError::Io(e) => write!(f, "WAL I/O error: {e}"),
            WalError::Serialize(e) => write!(f, "WAL serialization error: {e}"),
            WalError::CorruptCheckpoint { path, reason } => {
                write!(f, "corrupt checkpoint at {}: {}", path.display(), reason)
            }
            WalError::CorruptSegment { path, reason } => {
                write!(f, "corrupt WAL segment at {}: {}", path.display(), reason)
            }
            WalError::BackpressureFull => write!(f, "WAL write-behind channel is full"),
            WalError::WriterGone => write!(f, "WAL background writer is no longer running"),
        }
    }
}

impl std::error::Error for WalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WalError::Io(e) => Some(e),
            WalError::Serialize(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for WalError {
    fn from(e: io::Error) -> Self {
        WalError::Io(e)
    }
}

impl From<postcard::Error> for WalError {
    fn from(e: postcard::Error) -> Self {
        WalError::Serialize(e)
    }
}
