//! Local SQLite library store for Overtone.
//!
//! Tracks, overlapping vibe groupings, quality audit results, and library
//! snapshots live here as the single source of truth.

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors produced by library storage operations.
#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Current SQLite schema version for the Overtone library database.
pub const SCHEMA_VERSION: i64 = 1;

/// High-level summary of a library database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibrarySummary {
    pub schema_version: i64,
    pub track_count: usize,
    pub vibe_count: usize,
}

/// Handle to an open Overtone SQLite library.
pub struct Library {
    conn: Connection,
}

impl Library {
    /// Opens an existing library at `path`, or creates a new one with WAL mode enabled.
    pub fn open(path: &Path) -> Result<Self, LibraryError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self { conn })
    }

    /// Opens an ephemeral in-memory library for unit tests.
    pub fn open_in_memory() -> Result<Self, LibraryError> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self { conn })
    }

    /// Returns a placeholder summary until Phase 1 schema migrations land.
    pub fn summary(&self) -> Result<LibrarySummary, LibraryError> {
        let _ = &self.conn;
        Ok(LibrarySummary {
            schema_version: SCHEMA_VERSION,
            track_count: 0,
            vibe_count: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_in_memory_library() {
        let lib = Library::open_in_memory().expect("open in-memory library");
        let summary = lib.summary().expect("read summary");
        assert_eq!(summary.schema_version, SCHEMA_VERSION);
        assert_eq!(summary.track_count, 0);
    }
}
