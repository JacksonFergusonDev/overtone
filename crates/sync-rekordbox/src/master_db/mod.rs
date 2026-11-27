//! My Tag sync into Rekordbox's `master.db`.
//!
//! Overtone owns one My Tag category named `Overtone` (a `djmdMyTag` row with
//! `ParentID = "root"` and `Attribute = 1`) and writes one tag per vibe in
//! it, named by the vibe's full path such as `Dubs/Deep`. Rows outside that
//! category are never written.
//!
//! The database is SQLCipher-encrypted with a passphrase that is the same for
//! every installation. This module opens whatever file it is given; callers
//! must give it a copy, never the live `master.db`, because opening a WAL
//! database creates `-wal` and `-shm` files next to it. See "Rekordbox
//! master.db: own My Tag writer over rusqlite with SQLCipher, not rbox" in
//! `engineering/decisions/dj-software-sync.md`.
//!
//! The bookkeeping (IDs, UUIDs, timestamps, and the `localUpdateCount` USN
//! counter) is ported from `rbox-masterdb` 0.3.1 by Dylan Jones, GPL-3.0-only
//! like Overtone. References name files in that crate's `src/` folder.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use unicode_normalization::UnicodeNormalization;

use crate::RekordboxError;

mod apply;
#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
mod plan;
#[cfg(test)]
mod tests;

pub use apply::{ApplyOutcome, OutsideRows, apply, verify};
pub use plan::{
    DesiredTag, DesiredTagState, ExistingTag, MyTagPlan, SongTagRemoval, TagAssignment, plan,
};

/// Name of the My Tag category Overtone owns.
pub const CATEGORY_NAME: &str = "Overtone";

/// `ParentID` of top-level My Tag categories.
pub const ROOT_PARENT: &str = "root";

/// `djmdMyTag.Attribute` of a category (rbox `enums.rs`, `MyTagType::Folder`).
pub const ATTRIBUTE_CATEGORY: i64 = 1;

/// `djmdMyTag.Attribute` of a tag (rbox `enums.rs`, `MyTagType::List`).
pub const ATTRIBUTE_TAG: i64 = 0;

/// `agentRegistry.registry_id` of the local USN counter. It is the only
/// `agentRegistry` row Overtone reads or writes: other rows hold Rekordbox
/// account credentials.
const LOCAL_USN_ROW: &str = "localUpdateCount";

/// The SQLCipher passphrase, stored shifted by one byte as in rbox
/// `connection.rs` (`MAGIC`), which decodes it by subtracting 1 from each
/// byte. It is the same for every Rekordbox 6 and 7 installation and is
/// published by pyrekordbox and rbox.
const MAGIC: &str = "513ge593d49928d46ggb9ggc9d8e:4254c85:f8e426eg8b92843b2gg547195:8";

/// How long a statement waits for a lock, as in rbox-db `connection.rs`.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Tables Overtone reads or writes.
const REQUIRED_TABLES: &[&str] = &["djmdMyTag", "djmdSongMyTag", "djmdContent", "agentRegistry"];

fn passphrase() -> String {
    MAGIC.bytes().map(|b| char::from(b - 1)).collect()
}

/// One `djmdMyTag` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyTagRow {
    pub id: String,
    pub seq: i64,
    pub name: String,
    pub attribute: i64,
    pub parent_id: String,
}

/// One `djmdSongMyTag` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongTagRow {
    pub id: String,
    pub my_tag_id: String,
    pub content_id: String,
    pub track_no: i64,
}

/// An open Rekordbox database.
pub struct MasterDb {
    conn: Connection,
    path: PathBuf,
}

impl std::fmt::Debug for MasterDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MasterDb")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl MasterDb {
    /// Opens an existing database file read-write, never creating one.
    ///
    /// Follows rbox-db `connection.rs` `setup_connection`: the key is applied
    /// before anything reads the file, then the busy timeout, then a schema
    /// read proves the key worked, then synchronous `EXTRA`, foreign keys,
    /// and (on macOS) full fsync. rbox sets no `cipher_compatibility`, and
    /// neither does Overtone: SQLCipher 4 defaults open Rekordbox 7 files.
    pub fn open(path: &Path) -> Result<Self, RekordboxError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.execute_batch(&format!(
            "PRAGMA key = '{}';",
            passphrase().replace('\'', "''")
        ))?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|err| {
            RekordboxError::NotMasterDb(format!("{}: couldn't decrypt it ({err})", path.display()))
        })?;
        conn.execute_batch(&format!(
            "PRAGMA synchronous = EXTRA; PRAGMA foreign_keys = 1; PRAGMA fullfsync = {};",
            i32::from(cfg!(target_os = "macos"))
        ))?;
        let journal: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        if !matches!(
            journal.to_ascii_lowercase().as_str(),
            "delete" | "truncate" | "persist" | "wal"
        ) {
            return Err(RekordboxError::NotMasterDb(format!(
                "{}: unexpected journal mode {journal}",
                path.display()
            )));
        }
        let db = Self {
            conn,
            path: path.to_path_buf(),
        };
        for table in REQUIRED_TABLES {
            let exists: bool = db.conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(RekordboxError::NotMasterDb(format!(
                    "{}: no {table} table",
                    path.display()
                )));
            }
        }
        db.local_usn()?;
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Closes the connection, reporting errors that dropping would hide.
    pub fn close(self) -> Result<(), RekordboxError> {
        self.conn.close().map_err(|(_, err)| err.into())
    }

    /// The current `journal_mode`, lowercase.
    pub fn journal_mode(&self) -> Result<String, RekordboxError> {
        let mode: String = self
            .conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        Ok(mode.to_ascii_lowercase())
    }

    /// Moves every WAL frame into the main file and truncates the WAL, so the
    /// file stands on its own once the connection closes. Errors when a
    /// reader or writer blocked part of the checkpoint.
    pub fn checkpoint(&self) -> Result<(), RekordboxError> {
        let (busy, log, done): (i64, i64, i64) =
            self.conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;
        if busy != 0 || (log >= 0 && log != done) {
            return Err(RekordboxError::VerifyFailed(format!(
                "checkpoint incomplete: busy {busy}, {done} of {log} frames"
            )));
        }
        Ok(())
    }

    /// The local USN counter: `agentRegistry.int_1` of the `localUpdateCount`
    /// row (rbox `models/agent_registry.rs`, `local_usn`). No other
    /// `agentRegistry` row is ever read.
    pub fn local_usn(&self) -> Result<i64, RekordboxError> {
        self.conn
            .query_row(
                "SELECT int_1 FROM agentRegistry WHERE registry_id = ?1",
                [LOCAL_USN_ROW],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
            .ok_or_else(|| {
                RekordboxError::NotMasterDb(format!(
                    "{}: no local update counter",
                    self.path.display()
                ))
            })
    }

    /// Every track Rekordbox knows, by NFC-normalized `FolderPath` (the
    /// absolute path including the file name). A file imported more than
    /// once maps to several content IDs. Rows marked deleted are left out.
    pub fn contents_by_path(&self) -> Result<HashMap<String, Vec<String>>, RekordboxError> {
        let mut stmt = self.conn.prepare(
            "SELECT FolderPath, ID FROM djmdContent \
             WHERE FolderPath IS NOT NULL AND COALESCE(rb_local_deleted, 0) = 0 ORDER BY ID",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        for row in rows {
            let (path, id) = row?;
            out.entry(normalize_path(&path)).or_default().push(id);
        }
        Ok(out)
    }

    /// Number of `djmdContent` rows.
    pub fn content_count(&self) -> Result<i64, RekordboxError> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM djmdContent", [], |row| row.get(0))?)
    }

    /// The `Overtone` category, if there is one. Errors when there are
    /// several, rather than guess which one Overtone owns.
    pub fn overtone_category(&self) -> Result<Option<MyTagRow>, RekordboxError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MY_TAG_COLUMNS} FROM djmdMyTag \
             WHERE Name = ?1 AND ParentID = ?2 AND Attribute = ?3 ORDER BY Seq, ID"
        ))?;
        let rows = stmt
            .query_map(
                params![CATEGORY_NAME, ROOT_PARENT, ATTRIBUTE_CATEGORY],
                my_tag_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        match rows.len() {
            0 | 1 => Ok(rows.into_iter().next()),
            count => Err(RekordboxError::AmbiguousCategory(count)),
        }
    }

    /// Tags in the `Overtone` category, in `Seq` order. Empty when there is
    /// no category.
    pub fn overtone_tags(&self) -> Result<Vec<MyTagRow>, RekordboxError> {
        match self.overtone_category()? {
            Some(category) => self.children(&category.id),
            None => Ok(Vec::new()),
        }
    }

    /// Every `djmdMyTag` row named `name`, in any category.
    pub fn my_tags_named(&self, name: &str) -> Result<Vec<MyTagRow>, RekordboxError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MY_TAG_COLUMNS} FROM djmdMyTag WHERE Name = ?1 ORDER BY ParentID, Seq, ID"
        ))?;
        Ok(stmt
            .query_map([name], my_tag_row)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn children(&self, parent_id: &str) -> Result<Vec<MyTagRow>, RekordboxError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MY_TAG_COLUMNS} FROM djmdMyTag WHERE ParentID = ?1 ORDER BY Seq, ID"
        ))?;
        Ok(stmt
            .query_map([parent_id], my_tag_row)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// Song-tag rows of each tag in `tag_ids`, in `TrackNo` order. Every
    /// requested tag gets an entry, empty if it has no tracks.
    pub fn tag_assignments(
        &self,
        tag_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<SongTagRow>>, RekordboxError> {
        let mut stmt = self.conn.prepare(
            "SELECT ID, MyTagID, ContentID, TrackNo FROM djmdSongMyTag \
             WHERE MyTagID = ?1 ORDER BY TrackNo, ID",
        )?;
        let mut out = BTreeMap::new();
        for tag_id in tag_ids {
            let rows = stmt
                .query_map([tag_id], |row| {
                    Ok(SongTagRow {
                        id: row.get(0)?,
                        my_tag_id: row.get(1)?,
                        content_id: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        track_no: row.get::<_, Option<i64>>(3)?.unwrap_or_default(),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            out.insert(tag_id.clone(), rows);
        }
        Ok(out)
    }
}

const MY_TAG_COLUMNS: &str = "ID, Seq, Name, Attribute, ParentID";

fn my_tag_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MyTagRow> {
    Ok(MyTagRow {
        id: row.get(0)?,
        seq: row.get::<_, Option<i64>>(1)?.unwrap_or_default(),
        name: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        attribute: row.get::<_, Option<i64>>(3)?.unwrap_or_default(),
        parent_id: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
    })
}

/// NFC form of a path string. macOS file APIs return decomposed (NFD)
/// names on some volumes, and Rekordbox may store either form.
pub fn normalize_path(path: &str) -> String {
    path.nfc().collect()
}
