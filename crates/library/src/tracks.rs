use std::collections::HashMap;

use overtone_model::{
    AudioFormat, AudioProperties, QualityReport, Track, TrackId, TrackTags, UnixMillis, VibeId,
};
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Library, LibraryError, now_millis, quality};

/// What a scan read from one file, ready to store.
#[derive(Debug, Clone, PartialEq, Default, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct ScannedTrack {
    pub path: String,
    pub file_size: u64,
    pub modified_at: UnixMillis,
    pub audio_hash: Option<String>,
    pub properties: AudioProperties,
    pub tags: TrackTags,
}

/// What `upsert_scanned` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum UpsertOutcome {
    Inserted,
    Updated,
    /// Size and modification time matched the stored record; nothing changed
    /// except clearing a missing flag.
    Unchanged,
}

const TRACK_COLUMNS: &str = "id, path, file_size, modified_at, audio_hash, format, \
    sample_rate_hz, bit_depth, channels, bitrate_kbps, duration_ms, title, artist, album, \
    album_artist, remixer, label, genre, year, bpm, musical_key, comment, grouping, added_at, \
    missing_since";

impl Library {
    /// Inserts or refreshes the record for a scanned file, matched by exact path.
    pub fn upsert_scanned(
        &mut self,
        scanned: &ScannedTrack,
    ) -> Result<(TrackId, UpsertOutcome), LibraryError> {
        let tx = self.conn.transaction()?;
        let result = upsert_in(&tx, scanned, now_millis())?;
        tx.commit()?;
        Ok(result)
    }

    /// Applies many scan results in one transaction.
    pub fn upsert_scanned_batch(
        &mut self,
        scanned: &[ScannedTrack],
    ) -> Result<Vec<(TrackId, UpsertOutcome)>, LibraryError> {
        let now = now_millis();
        let tx = self.conn.transaction()?;
        let results = scanned
            .iter()
            .map(|track| upsert_in(&tx, track, now))
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit()?;
        Ok(results)
    }

    pub fn track(&self, id: TrackId) -> Result<Track, LibraryError> {
        let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id = ?1");
        let mut track = self
            .conn
            .query_row(&sql, [id.0], track_from_row)
            .optional()?
            .ok_or(LibraryError::NotFound {
                kind: "track",
                id: id.0,
            })??;
        track.vibes = self.vibe_ids_for(id)?;
        track.quality = quality::report_for(&self.conn, id)?;
        Ok(track)
    }

    /// Looks up a track by its exact stored path.
    pub fn track_by_path(&self, path: &str) -> Result<Option<Track>, LibraryError> {
        let id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM tracks WHERE path = ?1", [path], |row| {
                row.get(0)
            })
            .optional()?;
        id.map(|id| self.track(TrackId(id))).transpose()
    }

    /// Every track, including missing ones, ordered by path, with vibes and
    /// quality reports attached.
    pub fn tracks(&self) -> Result<Vec<Track>, LibraryError> {
        let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks ORDER BY path");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut tracks = stmt
            .query_map([], track_from_row)?
            .map(|row| row?)
            .collect::<Result<Vec<_>, LibraryError>>()?;

        let mut vibes: HashMap<i64, Vec<VibeId>> = HashMap::new();
        let mut stmt = self
            .conn
            .prepare("SELECT track_id, vibe_id FROM track_vibes ORDER BY track_id, vibe_id")?;
        for row in stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))? {
            let (track, vibe) = row?;
            vibes.entry(track).or_default().push(VibeId(vibe));
        }
        let mut reports = quality::all_reports(&self.conn)?;
        for track in &mut tracks {
            track.vibes = vibes.remove(&track.id.0).unwrap_or_default();
            track.quality = reports.remove(&track.id);
        }
        Ok(tracks)
    }

    /// Paths and ids of tracks whose path lies inside `root` (a folder path
    /// without trailing slash), compared literally.
    pub fn track_paths_under(&self, root: &str) -> Result<Vec<(TrackId, String)>, LibraryError> {
        let prefix = format!("{}/", root.trim_end_matches('/'));
        let mut stmt = self.conn.prepare(
            "SELECT id, path FROM tracks WHERE substr(path, 1, length(?1)) = ?1 ORDER BY path",
        )?;
        let rows = stmt
            .query_map([&prefix], |row| Ok((TrackId(row.get(0)?), row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Marks tracks as missing (or found again with `None`). Records and their
    /// vibes are kept.
    pub fn set_missing(
        &mut self,
        ids: &[TrackId],
        missing_since: Option<UnixMillis>,
    ) -> Result<(), LibraryError> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "UPDATE tracks SET missing_since = ?2, updated_at = ?3 WHERE id = ?1 \
                 AND missing_since IS NOT ?2",
            )?;
            let now = now_millis();
            for id in ids {
                stmt.execute(params![id.0, missing_since, now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Other tracks with the same audio hash as `id`.
    pub fn duplicates_of(&self, id: TrackId) -> Result<Vec<TrackId>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT other.id FROM tracks AS this JOIN tracks AS other \
             ON other.audio_hash = this.audio_hash AND other.id != this.id \
             WHERE this.id = ?1 AND this.audio_hash IS NOT NULL ORDER BY other.id",
        )?;
        let ids = stmt
            .query_map([id.0], |row| Ok(TrackId(row.get(0)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Removes a track's record (never its file). Vibes and reports cascade.
    pub fn forget_track(&mut self, id: TrackId) -> Result<(), LibraryError> {
        let removed = self
            .conn
            .execute("DELETE FROM tracks WHERE id = ?1", [id.0])?;
        if removed == 0 {
            return Err(LibraryError::NotFound {
                kind: "track",
                id: id.0,
            });
        }
        Ok(())
    }
}

fn upsert_in(
    tx: &rusqlite::Transaction<'_>,
    scanned: &ScannedTrack,
    now: UnixMillis,
) -> Result<(TrackId, UpsertOutcome), LibraryError> {
    let existing: Option<(i64, u64, i64)> = tx
        .query_row(
            "SELECT id, file_size, modified_at FROM tracks WHERE path = ?1",
            [&scanned.path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;

    let p = &scanned.properties;
    let t = &scanned.tags;
    let format = p.format.map(AudioFormat::as_str);
    match existing {
        Some((id, size, modified))
            if size == scanned.file_size && modified == scanned.modified_at =>
        {
            tx.execute(
                "UPDATE tracks SET missing_since = NULL WHERE id = ?1 AND missing_since IS NOT NULL",
                [id],
            )?;
            Ok((TrackId(id), UpsertOutcome::Unchanged))
        }
        Some((id, _, _)) => {
            tx.execute(
                "UPDATE tracks SET file_size = ?2, modified_at = ?3, audio_hash = ?4, format = ?5, \
                 sample_rate_hz = ?6, bit_depth = ?7, channels = ?8, bitrate_kbps = ?9, \
                 duration_ms = ?10, title = ?11, artist = ?12, album = ?13, album_artist = ?14, \
                 remixer = ?15, label = ?16, genre = ?17, year = ?18, bpm = ?19, musical_key = ?20, \
                 comment = ?21, grouping = ?22, updated_at = ?23, missing_since = NULL WHERE id = ?1",
                params![
                    id,
                    scanned.file_size,
                    scanned.modified_at,
                    scanned.audio_hash,
                    format,
                    p.sample_rate_hz,
                    p.bit_depth,
                    p.channels,
                    p.bitrate_kbps,
                    p.duration_ms,
                    t.title,
                    t.artist,
                    t.album,
                    t.album_artist,
                    t.remixer,
                    t.label,
                    t.genre,
                    t.year,
                    t.bpm,
                    t.key,
                    t.comment,
                    t.grouping,
                    now,
                ],
            )?;
            Ok((TrackId(id), UpsertOutcome::Updated))
        }
        None => {
            tx.execute(
                "INSERT INTO tracks (path, file_size, modified_at, audio_hash, format, \
                 sample_rate_hz, bit_depth, channels, bitrate_kbps, duration_ms, title, artist, \
                 album, album_artist, remixer, label, genre, year, bpm, musical_key, comment, \
                 grouping, added_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, \
                 ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?23)",
                params![
                    scanned.path,
                    scanned.file_size,
                    scanned.modified_at,
                    scanned.audio_hash,
                    format,
                    p.sample_rate_hz,
                    p.bit_depth,
                    p.channels,
                    p.bitrate_kbps,
                    p.duration_ms,
                    t.title,
                    t.artist,
                    t.album,
                    t.album_artist,
                    t.remixer,
                    t.label,
                    t.genre,
                    t.year,
                    t.bpm,
                    t.key,
                    t.comment,
                    t.grouping,
                    now,
                ],
            )?;
            Ok((TrackId(tx.last_insert_rowid()), UpsertOutcome::Inserted))
        }
    }
}

/// Reads a track row. The outer `rusqlite::Result` reports column access
/// errors; the inner one reports values that don't match the schema.
fn track_from_row(row: &Row<'_>) -> rusqlite::Result<Result<Track, LibraryError>> {
    let format: Option<String> = row.get(5)?;
    let format = match format {
        None => None,
        Some(value) => match AudioFormat::parse(&value) {
            Some(format) => Some(format),
            None => {
                return Ok(Err(LibraryError::Corrupt {
                    column: "tracks.format",
                    value,
                }));
            }
        },
    };
    Ok(Ok(Track {
        id: TrackId(row.get(0)?),
        path: row.get(1)?,
        file_size: row.get(2)?,
        modified_at: row.get(3)?,
        audio_hash: row.get(4)?,
        properties: AudioProperties {
            format,
            sample_rate_hz: row.get(6)?,
            bit_depth: row.get(7)?,
            channels: row.get(8)?,
            bitrate_kbps: row.get(9)?,
            duration_ms: row.get(10)?,
        },
        tags: TrackTags {
            title: row.get(11)?,
            artist: row.get(12)?,
            album: row.get(13)?,
            album_artist: row.get(14)?,
            remixer: row.get(15)?,
            label: row.get(16)?,
            genre: row.get(17)?,
            year: row.get(18)?,
            bpm: row.get(19)?,
            key: row.get(20)?,
            comment: row.get(21)?,
            grouping: row.get(22)?,
        },
        vibes: Vec::new(),
        quality: None::<QualityReport>,
        added_at: row.get(23)?,
        missing_since: row.get(24)?,
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn scanned(path: &str) -> ScannedTrack {
        ScannedTrack {
            path: path.to_string(),
            file_size: 1_000,
            modified_at: 1_700_000_000_000,
            audio_hash: Some(format!("hash-{path}")),
            properties: AudioProperties {
                format: Some(AudioFormat::Flac),
                sample_rate_hz: Some(44_100),
                bit_depth: Some(16),
                channels: Some(2),
                bitrate_kbps: Some(900),
                duration_ms: Some(300_000),
            },
            tags: TrackTags {
                title: Some("Title".into()),
                artist: Some("Artist".into()),
                bpm: Some(174.0),
                key: Some("8A".into()),
                ..TrackTags::default()
            },
        }
    }

    #[test]
    fn inserts_then_skips_unchanged_then_updates() {
        let mut lib = Library::open_in_memory().expect("open");
        let first = scanned("/music/a.flac");
        let (id, outcome) = lib.upsert_scanned(&first).expect("insert");
        assert_eq!(outcome, UpsertOutcome::Inserted);

        let (same, outcome) = lib.upsert_scanned(&first).expect("rescan");
        assert_eq!((same, outcome), (id, UpsertOutcome::Unchanged));

        let mut changed = first.clone();
        changed.modified_at += 1;
        changed.tags.title = Some("New Title".into());
        let (same, outcome) = lib.upsert_scanned(&changed).expect("update");
        assert_eq!((same, outcome), (id, UpsertOutcome::Updated));

        let track = lib.track(id).expect("load");
        assert_eq!(track.tags.title.as_deref(), Some("New Title"));
        assert_eq!(track.properties.format, Some(AudioFormat::Flac));
        assert_eq!(track.tags.bpm, Some(174.0));
    }

    #[test]
    fn paths_with_sql_wildcards_are_compared_literally() {
        let mut lib = Library::open_in_memory().expect("open");
        lib.upsert_scanned(&scanned("/music/a_b/one.flac"))
            .expect("a");
        lib.upsert_scanned(&scanned("/music/aXb/two.flac"))
            .expect("b");
        lib.upsert_scanned(&scanned("/music/a_b2/three.flac"))
            .expect("c");
        let under = lib.track_paths_under("/music/a_b").expect("query");
        assert_eq!(under.len(), 1);
        assert_eq!(under[0].1, "/music/a_b/one.flac");
        assert!(
            lib.track_by_path("/music/a%b/one.flac")
                .expect("query")
                .is_none()
        );
    }

    #[test]
    fn missing_tracks_keep_their_record_and_clear_on_rescan() {
        let mut lib = Library::open_in_memory().expect("open");
        let file = scanned("/music/a.flac");
        let (id, _) = lib.upsert_scanned(&file).expect("insert");
        lib.set_missing(&[id], Some(5)).expect("mark missing");
        assert_eq!(lib.track(id).expect("load").missing_since, Some(5));
        assert_eq!(lib.summary().expect("summary").missing_count, 1);

        lib.upsert_scanned(&file).expect("found again");
        assert_eq!(lib.track(id).expect("load").missing_since, None);
    }

    #[test]
    fn finds_duplicates_by_audio_hash() {
        let mut lib = Library::open_in_memory().expect("open");
        let mut a = scanned("/music/a.flac");
        let mut b = scanned("/music/b.flac");
        a.audio_hash = Some("same".into());
        b.audio_hash = Some("same".into());
        let results = lib
            .upsert_scanned_batch(&[a, b, scanned("/music/c.flac")])
            .expect("batch");
        assert_eq!(
            lib.duplicates_of(results[0].0).expect("dups"),
            vec![results[1].0]
        );
        assert!(lib.duplicates_of(results[2].0).expect("dups").is_empty());
    }

    #[test]
    fn forgetting_a_track_cascades() {
        let mut lib = Library::open_in_memory().expect("open");
        let (id, _) = lib
            .upsert_scanned(&scanned("/music/a.flac"))
            .expect("insert");
        let vibe = lib
            .create_vibe(&"Dubs".parse().expect("vibe"))
            .expect("vibe");
        lib.assign_vibe(&[id], vibe.id).expect("assign");
        lib.forget_track(id).expect("forget");
        assert!(lib.tracks().expect("list").is_empty());
        assert_eq!(lib.vibes().expect("vibes")[0].track_count, 0);
        assert!(matches!(
            lib.forget_track(id),
            Err(LibraryError::NotFound { .. })
        ));
    }
}
