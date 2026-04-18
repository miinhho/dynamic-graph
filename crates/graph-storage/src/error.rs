//! Error types for graph-storage.

use std::fmt;

#[derive(Debug)]
pub enum StorageError {
    /// redb database error.
    Db(Box<redb::DatabaseError>),
    /// redb table error.
    Table(Box<redb::TableError>),
    /// redb transaction error.
    Transaction(Box<redb::TransactionError>),
    /// redb commit error.
    Commit(Box<redb::CommitError>),
    /// redb storage error.
    Storage(Box<redb::StorageError>),
    /// Serialization / deserialization failure (postcard).
    Serde(postcard::Error),
    /// The database is empty (no world has been saved yet).
    Empty,
    /// The database was written by a different schema version.
    SchemaMismatch { found: u64, expected: u64 },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Db(e) => write!(f, "storage database error: {e}"),
            Self::Table(e) => write!(f, "storage table error: {e}"),
            Self::Transaction(e) => write!(f, "storage transaction error: {e}"),
            Self::Commit(e) => write!(f, "storage commit error: {e}"),
            Self::Storage(e) => write!(f, "storage error: {e}"),
            Self::Serde(e) => write!(f, "storage serialization error: {e}"),
            Self::Empty => write!(f, "storage is empty — no world saved yet"),
            Self::SchemaMismatch { found, expected } => write!(
                f,
                "schema mismatch: database version {found} is incompatible with code version {expected}"
            ),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Db(e) => Some(e.as_ref()),
            Self::Table(e) => Some(e.as_ref()),
            Self::Transaction(e) => Some(e.as_ref()),
            Self::Commit(e) => Some(e.as_ref()),
            Self::Storage(e) => Some(e.as_ref()),
            Self::Serde(e) => Some(e),
            Self::Empty => None,
            Self::SchemaMismatch { .. } => None,
        }
    }
}

impl From<redb::DatabaseError> for StorageError {
    fn from(e: redb::DatabaseError) -> Self {
        Self::Db(Box::new(e))
    }
}
impl From<redb::TableError> for StorageError {
    fn from(e: redb::TableError) -> Self {
        Self::Table(Box::new(e))
    }
}
impl From<redb::TransactionError> for StorageError {
    fn from(e: redb::TransactionError) -> Self {
        Self::Transaction(Box::new(e))
    }
}
impl From<redb::CommitError> for StorageError {
    fn from(e: redb::CommitError) -> Self {
        Self::Commit(Box::new(e))
    }
}
impl From<redb::StorageError> for StorageError {
    fn from(e: redb::StorageError) -> Self {
        Self::Storage(Box::new(e))
    }
}
impl From<postcard::Error> for StorageError {
    fn from(e: postcard::Error) -> Self {
        Self::Serde(e)
    }
}
