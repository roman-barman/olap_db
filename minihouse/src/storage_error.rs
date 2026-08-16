use crate::codec::CodecError;
use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("not found: {0}")]
    NotFound(PathBuf),
    #[error("unsupported format version {found}, expected {expected}")]
    UnsupportedVersion { found: String, expected: u32 },
    #[error("corrupt: {0}")]
    Corrupt(String),
    #[error("column '{0}' not found")]
    ColumnNotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}
