//! The value a tag field had before Overtone first wrote it, so tag
//! mirroring can be reverted. Only the first original is kept per field.

use overtone_model::{TrackId, UnixMillis};
use rusqlite::{OptionalExtension, params};

use crate::{Library, LibraryError, now_millis};

/// The `tag_snapshots.field` value for the Comment tag.
pub const COMMENT_FIELD: &str = "comment";

/// A tag field's value from before Overtone first changed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSnapshot {
    pub track_id: TrackId,
    pub field: String,
    /// `None` when the field was empty or absent.
    pub original_value: Option<String>,
    pub captured_at: UnixMillis,
}

impl Library {
    /// Records `original` as the value `field` had before Overtone's first
    /// write. Keeps an existing snapshot untouched, so repeated writes never
    /// overwrite the true original. Returns whether a snapshot was added.
    pub fn record_tag_snapshot(
        &self,
        track: TrackId,
        field: &str,
        original: Option<&str>,
    ) -> Result<bool, LibraryError> {
        let inserted = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO tag_snapshots (track_id, field, original_value, captured_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![track.0, field, original, now_millis()],
            )
            .map_err(|err| match &err {
                rusqlite::Error::SqliteFailure(e, _)
                    if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY =>
                {
                    LibraryError::NotFound {
                        kind: "track",
                        id: track.0,
                    }
                }
                _ => err.into(),
            })?;
        Ok(inserted == 1)
    }

    pub fn tag_snapshot(
        &self,
        track: TrackId,
        field: &str,
    ) -> Result<Option<TagSnapshot>, LibraryError> {
        Ok(self
            .conn
            .query_row(
                "SELECT original_value, captured_at FROM tag_snapshots \
                 WHERE track_id = ?1 AND field = ?2",
                params![track.0, field],
                |row| {
                    Ok(TagSnapshot {
                        track_id: track,
                        field: field.to_string(),
                        original_value: row.get(0)?,
                        captured_at: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Tracks with a snapshot of `field`, ordered by id.
    pub fn tracks_with_tag_snapshot(&self, field: &str) -> Result<Vec<TrackId>, LibraryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT track_id FROM tag_snapshots WHERE field = ?1 ORDER BY track_id")?;
        let ids = stmt
            .query_map([field], |row| Ok(TrackId(row.get(0)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Forgets the snapshot, typically after the original was written back.
    /// Returns whether one existed.
    pub fn clear_tag_snapshot(&self, track: TrackId, field: &str) -> Result<bool, LibraryError> {
        let removed = self.conn.execute(
            "DELETE FROM tag_snapshots WHERE track_id = ?1 AND field = ?2",
            params![track.0, field],
        )?;
        Ok(removed == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::tests::scanned;

    #[test]
    fn keeps_only_the_first_original() {
        let mut lib = Library::open_in_memory().expect("open");
        let (track, _) = lib.upsert_scanned(&scanned("/m/a.flac")).expect("track");
        assert!(
            lib.record_tag_snapshot(track, COMMENT_FIELD, Some("8A - Energy 6"))
                .expect("record")
        );
        assert!(
            !lib.record_tag_snapshot(track, COMMENT_FIELD, Some("8A - Energy 6 #warmup"))
                .expect("second record")
        );
        let snapshot = lib
            .tag_snapshot(track, COMMENT_FIELD)
            .expect("get")
            .expect("exists");
        assert_eq!(snapshot.original_value.as_deref(), Some("8A - Energy 6"));
        assert_eq!(
            lib.tracks_with_tag_snapshot(COMMENT_FIELD).expect("list"),
            vec![track]
        );
        assert!(lib.clear_tag_snapshot(track, COMMENT_FIELD).expect("clear"));
        assert!(
            !lib.clear_tag_snapshot(track, COMMENT_FIELD)
                .expect("clear again")
        );
        assert_eq!(lib.tag_snapshot(track, COMMENT_FIELD).expect("get"), None);
    }

    #[test]
    fn records_empty_originals_and_rejects_unknown_tracks() {
        let mut lib = Library::open_in_memory().expect("open");
        let (track, _) = lib.upsert_scanned(&scanned("/m/a.flac")).expect("track");
        lib.record_tag_snapshot(track, COMMENT_FIELD, None)
            .expect("record");
        let snapshot = lib
            .tag_snapshot(track, COMMENT_FIELD)
            .expect("get")
            .expect("exists");
        assert_eq!(snapshot.original_value, None);
        assert!(matches!(
            lib.record_tag_snapshot(TrackId(999), COMMENT_FIELD, None),
            Err(LibraryError::NotFound { kind: "track", .. })
        ));
    }

    #[test]
    fn snapshots_cascade_when_a_track_is_forgotten() {
        let mut lib = Library::open_in_memory().expect("open");
        let (track, _) = lib.upsert_scanned(&scanned("/m/a.flac")).expect("track");
        lib.record_tag_snapshot(track, COMMENT_FIELD, Some("x"))
            .expect("record");
        lib.forget_track(track).expect("forget");
        assert!(
            lib.tracks_with_tag_snapshot(COMMENT_FIELD)
                .expect("list")
                .is_empty()
        );
    }
}
