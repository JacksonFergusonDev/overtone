use std::path::PathBuf;

use overtone_model::{MetadataField, VibePathError};
use thiserror::Error;

/// Errors produced by library storage operations.
#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "this library was created by a newer version of Overtone (schema {found}, this build supports {supported})"
    )]
    SchemaTooNew { found: i64, supported: i64 },
    #[error(transparent)]
    InvalidVibe(#[from] VibePathError),
    #[error("a vibe named {0:?} already exists")]
    VibeExists(String),
    #[error("{kind} {id} not found")]
    NotFound { kind: &'static str, id: i64 },
    #[error("no {kind} named {name:?}")]
    NotFoundByName { kind: &'static str, name: String },
    #[error("{field} {reason}")]
    FieldNotAllowed {
        field: MetadataField,
        reason: &'static str,
    },
    #[error("suggestion {id} was already {status}")]
    SuggestionDecided { id: i64, status: &'static str },
    #[error(
        "accepting would replace the current {field} {current:?}; choose to replace it explicitly"
    )]
    WouldReplace {
        field: MetadataField,
        current: String,
    },
    #[error("unexpected value {value:?} in column {column}")]
    Corrupt { column: &'static str, value: String },
}
