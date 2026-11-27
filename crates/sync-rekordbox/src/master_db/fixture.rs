//! Encrypted stand-ins for a Rekordbox `master.db`, for tests only.
//!
//! The tables Overtone touches use the exact DDL of a Rekordbox 7.0.8
//! database. `djmdContent` keeps all of its columns so its rows look real.
//! Compiled for this crate's tests and, through the `fixture` feature, for
//! other crates' tests.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use super::passphrase;
use crate::RekordboxError;

/// `localUpdateCount` in a fresh fixture.
pub const INITIAL_USN: i64 = 1000;

/// A fake account-credentials `agentRegistry` row. Overtone must never read
/// it; tests check it survives unchanged and never shows up in output.
pub const CREDENTIAL_ROW: &str = "fixtureAccountToken";
pub const CREDENTIAL_SECRET: &str = "fixture-secret-never-read";

/// A user tag that [`create_live`] leaves only in the `-wal` file.
pub const WAL_ONLY_TAG: &str = "Only In WAL";

const TIMESTAMP: &str = "2025-01-20 22:13:47.022 +00:00";

const SCHEMA: &str = "
CREATE TABLE `agentRegistry` (`registry_id` VARCHAR(255) PRIMARY KEY, `id_1` VARCHAR(255) DEFAULT NULL, `id_2` VARCHAR(255) DEFAULT NULL, `int_1` BIGINT DEFAULT NULL, `int_2` BIGINT DEFAULT NULL, `str_1` VARCHAR(255) DEFAULT NULL, `str_2` VARCHAR(255) DEFAULT NULL, `date_1` DATETIME DEFAULT NULL, `date_2` DATETIME DEFAULT NULL, `text_1` TEXT DEFAULT NULL, `text_2` TEXT DEFAULT NULL, `created_at` DATETIME NOT NULL, `updated_at` DATETIME NOT NULL);
CREATE TABLE `djmdMyTag` (`ID` VARCHAR(255) PRIMARY KEY, `Seq` INTEGER DEFAULT NULL, `Name` VARCHAR(255) DEFAULT NULL, `Attribute` INTEGER DEFAULT NULL, `ParentID` VARCHAR(255) DEFAULT NULL, `UUID` VARCHAR(255) DEFAULT NULL, `rb_data_status` INTEGER DEFAULT 0, `rb_local_data_status` INTEGER DEFAULT 0, `rb_local_deleted` TINYINT(1) DEFAULT 0, `rb_local_synced` TINYINT(1) DEFAULT 0, `usn` BIGINT DEFAULT NULL, `rb_local_usn` BIGINT DEFAULT NULL, `created_at` DATETIME NOT NULL, `updated_at` DATETIME NOT NULL);
CREATE INDEX `djmd_my_tag__parent_i_d` ON `djmdMyTag` (`ParentID`);
CREATE INDEX `djmd_my_tag__seq` ON `djmdMyTag` (`Seq`);
CREATE INDEX `djmd_my_tag__u_u_i_d` ON `djmdMyTag` (`UUID`);
CREATE INDEX `djmd_my_tag_rb_local_usn__i_d` ON `djmdMyTag` (`rb_local_usn`, `ID`);
CREATE TABLE `djmdSongMyTag` (`ID` VARCHAR(255) PRIMARY KEY, `MyTagID` VARCHAR(255) DEFAULT NULL, `ContentID` VARCHAR(255) DEFAULT NULL, `TrackNo` INTEGER DEFAULT NULL, `UUID` VARCHAR(255) DEFAULT NULL, `rb_data_status` INTEGER DEFAULT 0, `rb_local_data_status` INTEGER DEFAULT 0, `rb_local_deleted` TINYINT(1) DEFAULT 0, `rb_local_synced` TINYINT(1) DEFAULT 0, `usn` BIGINT DEFAULT NULL, `rb_local_usn` BIGINT DEFAULT NULL, `created_at` DATETIME NOT NULL, `updated_at` DATETIME NOT NULL);
CREATE INDEX `djmd_song_my_tag__my_tag_i_d_rb_local_deleted__i_d` ON `djmdSongMyTag` (`MyTagID`, `rb_local_deleted`, `ID`);
CREATE INDEX `djmd_song_my_tag__content_i_d_rb_local_deleted` ON `djmdSongMyTag` (`ContentID`, `rb_local_deleted`);
CREATE TABLE `djmdContent` (`ID` VARCHAR(255) PRIMARY KEY, `FolderPath` VARCHAR(255) DEFAULT NULL, `FileNameL` VARCHAR(255) DEFAULT NULL, `FileNameS` VARCHAR(255) DEFAULT NULL, `Title` VARCHAR(255) DEFAULT NULL, `ArtistID` VARCHAR(255) DEFAULT NULL, `AlbumID` VARCHAR(255) DEFAULT NULL, `GenreID` VARCHAR(255) DEFAULT NULL, `BPM` INTEGER DEFAULT NULL, `Length` INTEGER DEFAULT NULL, `TrackNo` INTEGER DEFAULT NULL, `BitRate` INTEGER DEFAULT NULL, `BitDepth` INTEGER DEFAULT NULL, `Commnt` TEXT DEFAULT NULL, `FileType` INTEGER DEFAULT NULL, `Rating` INTEGER DEFAULT NULL, `ReleaseYear` INTEGER DEFAULT NULL, `RemixerID` VARCHAR(255) DEFAULT NULL, `LabelID` VARCHAR(255) DEFAULT NULL, `OrgArtistID` VARCHAR(255) DEFAULT NULL, `KeyID` VARCHAR(255) DEFAULT NULL, `StockDate` VARCHAR(255) DEFAULT NULL, `ColorID` VARCHAR(255) DEFAULT NULL, `DJPlayCount` INTEGER DEFAULT NULL, `ImagePath` VARCHAR(255) DEFAULT NULL, `MasterDBID` VARCHAR(255) DEFAULT NULL, `MasterSongID` VARCHAR(255) DEFAULT NULL, `AnalysisDataPath` VARCHAR(255) DEFAULT NULL, `SearchStr` VARCHAR(255) DEFAULT NULL, `FileSize` INTEGER DEFAULT NULL, `DiscNo` INTEGER DEFAULT NULL, `ComposerID` VARCHAR(255) DEFAULT NULL, `Subtitle` VARCHAR(255) DEFAULT NULL, `SampleRate` INTEGER DEFAULT NULL, `DisableQuantize` INTEGER DEFAULT NULL, `Analysed` INTEGER DEFAULT NULL, `ReleaseDate` VARCHAR(255) DEFAULT NULL, `DateCreated` VARCHAR(255) DEFAULT NULL, `ContentLink` INTEGER DEFAULT NULL, `Tag` VARCHAR(255) DEFAULT NULL, `ModifiedByRBM` VARCHAR(255) DEFAULT NULL, `HotCueAutoLoad` VARCHAR(255) DEFAULT NULL, `DeliveryControl` VARCHAR(255) DEFAULT NULL, `DeliveryComment` VARCHAR(255) DEFAULT NULL, `CueUpdated` VARCHAR(255) DEFAULT NULL, `AnalysisUpdated` VARCHAR(255) DEFAULT NULL, `TrackInfoUpdated` VARCHAR(255) DEFAULT NULL, `Lyricist` VARCHAR(255) DEFAULT NULL, `ISRC` VARCHAR(255) DEFAULT NULL, `SamplerTrackInfo` INTEGER DEFAULT NULL, `SamplerPlayOffset` INTEGER DEFAULT NULL, `SamplerGain` FLOAT DEFAULT NULL, `VideoAssociate` VARCHAR(255) DEFAULT NULL, `LyricStatus` INTEGER DEFAULT NULL, `ServiceID` INTEGER DEFAULT NULL, `OrgFolderPath` VARCHAR(255) DEFAULT NULL, `Reserved1` TEXT DEFAULT NULL, `Reserved2` TEXT DEFAULT NULL, `Reserved3` TEXT DEFAULT NULL, `Reserved4` TEXT DEFAULT NULL, `ExtInfo` TEXT DEFAULT NULL, `rb_file_id` VARCHAR(255) DEFAULT NULL, `DeviceID` VARCHAR(255) DEFAULT NULL, `rb_LocalFolderPath` VARCHAR(255) DEFAULT NULL, `SrcID` VARCHAR(255) DEFAULT NULL, `SrcTitle` VARCHAR(255) DEFAULT NULL, `SrcArtistName` VARCHAR(255) DEFAULT NULL, `SrcAlbumName` VARCHAR(255) DEFAULT NULL, `SrcLength` INTEGER DEFAULT NULL, `UUID` VARCHAR(255) DEFAULT NULL, `rb_data_status` INTEGER DEFAULT 0, `rb_local_data_status` INTEGER DEFAULT 0, `rb_local_deleted` TINYINT(1) DEFAULT 0, `rb_local_synced` TINYINT(1) DEFAULT 0, `usn` BIGINT DEFAULT NULL, `rb_local_usn` BIGINT DEFAULT NULL, `created_at` DATETIME NOT NULL, `updated_at` DATETIME NOT NULL);
";

/// User tags in the fixture: (ID, Seq, Name, ParentID), under the default
/// Genre (1) and Situation (3) categories.
const USER_TAGS: &[(&str, i64, &str, &str)] = &[
    ("1001", 1, "Acid House", "1"),
    ("1002", 2, "Deep House", "1"),
    ("1003", 1, "Warmup", "3"),
];

fn open_new(path: &Path) -> Result<Connection, RekordboxError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(&format!("PRAGMA key = '{}';", passphrase()))?;
    Ok(conn)
}

/// Creates an encrypted database at `path` in WAL mode with the four
/// default My Tag categories, three user tags (Acid House and Deep House
/// under Genre, Warmup under Situation), and one `djmdContent` row per
/// `(ID, FolderPath)` in `tracks`. The first track is tagged Acid House and
/// the first two Warmup. Everything is checkpointed into the main file.
pub fn create(path: &Path, tracks: &[(&str, &str)]) -> Result<(), RekordboxError> {
    if path.exists() {
        return Err(RekordboxError::Unsupported(format!(
            "{} already exists",
            path.display()
        )));
    }
    let conn = open_new(path)?;
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    assert_eq!(mode, "wal");
    conn.execute_batch(SCHEMA)?;
    conn.execute(
        "INSERT INTO agentRegistry (registry_id, int_1, created_at, updated_at) \
         VALUES ('localUpdateCount', ?1, ?2, ?2)",
        params![INITIAL_USN, TIMESTAMP],
    )?;
    conn.execute(
        "INSERT INTO agentRegistry (registry_id, str_1, text_1, created_at, updated_at) \
         VALUES (?1, ?2, ?2, ?3, ?3)",
        params![CREDENTIAL_ROW, CREDENTIAL_SECRET, TIMESTAMP],
    )?;
    let mut usn = 100;
    for (seq, name) in ["Genre", "Components", "Situation", "Untitled Column"]
        .iter()
        .enumerate()
    {
        usn += 1;
        let id = (seq + 1).to_string();
        conn.execute(
            "INSERT INTO djmdMyTag (ID, Seq, Name, Attribute, ParentID, UUID, rb_local_usn, \
             created_at, updated_at) VALUES (?1, ?1, ?2, 1, 'root', ?1, ?3, ?4, ?4)",
            params![id, name, usn, TIMESTAMP],
        )?;
    }
    for (id, seq, name, parent) in USER_TAGS {
        usn += 1;
        conn.execute(
            "INSERT INTO djmdMyTag (ID, Seq, Name, Attribute, ParentID, UUID, rb_local_usn, \
             created_at, updated_at) VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?7)",
            params![id, seq, name, parent, format!("uuid-{id}"), usn, TIMESTAMP],
        )?;
    }
    for (index, (id, folder_path)) in tracks.iter().enumerate() {
        usn += 1;
        let file_name = Path::new(folder_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        conn.execute(
            "INSERT INTO djmdContent (ID, FolderPath, FileNameL, Title, FileType, BPM, UUID, \
             rb_local_usn, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 5, 12800, ?5, ?6, ?7, ?7)",
            params![
                id,
                folder_path,
                file_name,
                format!("Track {index}"),
                format!("content-{id}"),
                usn,
                TIMESTAMP
            ],
        )?;
    }
    let mut song = |id: &str, tag: &str, content: &str, track_no: i64| {
        usn += 1;
        conn.execute(
            "INSERT INTO djmdSongMyTag (ID, MyTagID, ContentID, TrackNo, UUID, rb_local_usn, \
             created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                id,
                tag,
                content,
                track_no,
                format!("uuid-{id}"),
                usn,
                TIMESTAMP
            ],
        )
    };
    if let Some((first, _)) = tracks.first() {
        song("2001", "1001", first, 1)?;
        song("2002", "1003", first, 1)?;
    }
    if let Some((second, _)) = tracks.get(1) {
        song("2003", "1003", second, 2)?;
    }
    conn.close().map_err(|(_, err)| err)?;
    Ok(())
}

/// Creates `dir/master.db` the way a closed Rekordbox can leave it: with
/// committed changes still in `master.db-wal` (a user tag named
/// [`WAL_ONLY_TAG`] under Components) and a `master.db-shm` beside it.
/// Returns the database path.
pub fn create_live(dir: &Path, tracks: &[(&str, &str)]) -> Result<PathBuf, RekordboxError> {
    let staging = tempfile::TempDir::new()?;
    let source = staging.path().join("master.db");
    create(&source, tracks)?;
    let conn = open_new(&source)?;
    conn.execute_batch("PRAGMA wal_autocheckpoint = 0;")?;
    conn.execute(
        "INSERT INTO djmdMyTag (ID, Seq, Name, Attribute, ParentID, UUID, rb_local_usn, \
         created_at, updated_at) VALUES ('1004', 1, ?1, 0, '2', 'uuid-1004', 150, ?2, ?2)",
        params![WAL_ONLY_TAG, TIMESTAMP],
    )?;
    std::fs::create_dir_all(dir)?;
    let target = dir.join("master.db");
    // Copied while the connection is open and idle, so the WAL still holds
    // the insert and the pair is consistent.
    for suffix in ["", "-wal", "-shm"] {
        std::fs::copy(
            staging.path().join(format!("master.db{suffix}")),
            dir.join(format!("master.db{suffix}")),
        )?;
    }
    conn.close().map_err(|(_, err)| err)?;
    Ok(target)
}

/// Reads the fake credentials row back. Only fixtures have one.
pub fn credential_row(path: &Path) -> Result<Option<String>, RekordboxError> {
    let conn = open_new(path)?;
    let secret = conn.query_row(
        "SELECT str_1 FROM agentRegistry WHERE registry_id = ?1",
        [CREDENTIAL_ROW],
        |row| row.get(0),
    )?;
    conn.close().map_err(|(_, err)| err)?;
    Ok(secret)
}
