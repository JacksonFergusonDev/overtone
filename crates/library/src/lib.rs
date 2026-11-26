//! Local SQLite library store for Overtone.
//!
//! Tracks, overlapping vibe groupings, quality reports, tag snapshots, and
//! sync history live here as the single source of truth shared by the
//! desktop app and CLI.

mod error;
mod migrations;
mod quality;
mod roots;
mod settings;
mod tracks;
mod vibes;

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub use error::LibraryError;
pub use migrations::SCHEMA_VERSION;
pub use overtone_model as model;
pub use roots::LibraryRoot;
pub use tracks::{ScannedTrack, UpsertOutcome};

/// File name of the library database inside the app data directory.
pub const LIBRARY_FILE_NAME: &str = "library.sqlite";

/// High-level summary of a library database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibrarySummary {
    pub schema_version: i64,
    pub track_count: u64,
    pub missing_count: u64,
    pub vibe_count: u64,
    /// Tracks whose automated verdict is Warning or Suspect and that the DJ
    /// hasn't reviewed yet.
    pub needs_review_count: u64,
}

/// Handle to an open Overtone SQLite library.
pub struct Library {
    conn: Connection,
}

impl Library {
    /// Opens the library at `path`, creating it and its parent folder if
    /// needed, and applies pending schema migrations.
    pub fn open(path: &Path) -> Result<Self, LibraryError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|source| LibraryError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    /// Opens an ephemeral in-memory library, for tests.
    pub fn open_in_memory() -> Result<Self, LibraryError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, LibraryError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrations::apply(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn summary(&self) -> Result<LibrarySummary, LibraryError> {
        let count = |sql: &str| -> Result<u64, LibraryError> {
            Ok(self.conn.query_row(sql, [], |row| row.get::<_, u64>(0))?)
        };
        Ok(LibrarySummary {
            schema_version: migrations::current_version(&self.conn)?,
            track_count: count("SELECT COUNT(*) FROM tracks WHERE missing_since IS NULL")?,
            missing_count: count("SELECT COUNT(*) FROM tracks WHERE missing_since IS NOT NULL")?,
            vibe_count: count("SELECT COUNT(*) FROM vibes")?,
            needs_review_count: count(
                "SELECT COUNT(*) FROM quality_reports \
                 WHERE verdict IN ('warning', 'suspect') AND review = 'unreviewed'",
            )?,
        })
    }

    /// Copies the whole database to `dest` using SQLite's online backup, for
    /// snapshots before bulk edits.
    pub fn backup_to(&self, dest: &Path) -> Result<(), LibraryError> {
        let mut target = Connection::open(dest)?;
        let backup = rusqlite::backup::Backup::new(&self.conn, &mut target)?;
        backup.run_to_completion(256, std::time::Duration::ZERO, None)?;
        Ok(())
    }
}

/// Current time in Unix milliseconds.
pub(crate) fn now_millis() -> i64 {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since_epoch.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn opens_in_memory_library_with_empty_summary() {
        let lib = Library::open_in_memory().expect("open in-memory library");
        let summary = lib.summary().expect("read summary");
        assert_eq!(summary.schema_version, SCHEMA_VERSION);
        assert_eq!(summary.track_count, 0);
        assert_eq!(summary.vibe_count, 0);
    }

    #[test]
    fn creates_parent_folders_and_reopens() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("nested/app/library.sqlite");
        {
            let lib = Library::open(&path).expect("create");
            lib.create_vibe(&"Warmup".parse().expect("vibe"))
                .expect("create vibe");
        }
        let lib = Library::open(&path).expect("reopen");
        assert_eq!(lib.summary().expect("summary").vibe_count, 1);
    }

    #[test]
    fn backs_up_to_a_new_file() {
        let dir = TempDir::new().expect("tempdir");
        let lib = Library::open_in_memory().expect("open");
        lib.create_vibe(&"Dubs".parse().expect("vibe"))
            .expect("create vibe");
        let dest = dir.path().join("snapshot.sqlite");
        lib.backup_to(&dest).expect("backup");
        let copy = Library::open(&dest).expect("open copy");
        assert_eq!(copy.summary().expect("summary").vibe_count, 1);
    }
}
