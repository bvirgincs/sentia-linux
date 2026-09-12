use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RetrievalError {
    #[error("invalid retrieval config: {0}")]
    InvalidConfig(String),
    #[error("indexing must run as a non-root identity")]
    RootIndexDenied,
    #[error("query contains no searchable tokens")]
    EmptyQuery,
    #[error("index file has insecure permissions: {0}")]
    InsecureIndexPermissions(String),
    #[error("failed to canonicalize {path}: {source}")]
    Canonicalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("database corruption detected at {path}: {details}")]
    CorruptDatabase { path: PathBuf, details: String },
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

impl RetrievalError {
    pub fn code(&self) -> &'static str {
        match self {
            RetrievalError::InvalidConfig(_) => "invalid_config",
            RetrievalError::RootIndexDenied => "root_index_denied",
            RetrievalError::EmptyQuery => "empty_query",
            RetrievalError::InsecureIndexPermissions(_) => "insecure_index_permissions",
            RetrievalError::Canonicalize { .. } => "canonicalize_failed",
            RetrievalError::Io { .. } => "io_error",
            RetrievalError::CorruptDatabase { .. } => "corrupt_database",
            RetrievalError::Sqlite(_) => "sqlite_error",
        }
    }

    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, RetrievalError>;
