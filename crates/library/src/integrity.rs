//! Stored ffmpeg integrity checks.

use overtone_model::{IntegrityCheck, TrackId};
use rusqlite::{OptionalExtension, Row, params};

use crate::{Library, LibraryError};

const COLUMNS: &str = "track_id, clean, edge_only, errors, error_count, file_modified_at, \
                       ffmpeg_version, checked_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<(IntegrityCheck, String)> {
    let errors: String = row.get(3)?;
    Ok((
        IntegrityCheck {
            track_id: TrackId(row.get(0)?),
            clean: row.get::<_, i64>(1)? == 1,
            edge_only: row.get::<_, i64>(2)? == 1,
            errors: Vec::new(),
            error_count: row.get(4)?,
            file_modified_at: row.get(5)?,
            ffmpeg_version: row.get(6)?,
            checked_at: row.get(7)?,
        },
        errors,
    ))
}

fn finish((mut check, errors): (IntegrityCheck, String)) -> Result<IntegrityCheck, LibraryError> {
    check.errors = serde_json::from_str(&errors).map_err(|_| LibraryError::Corrupt {
        column: "errors",
        value: errors,
    })?;
    Ok(check)
}

impl Library {
    /// Saves checks, replacing earlier ones for the same tracks.
    pub fn save_integrity_checks(&mut self, checks: &[IntegrityCheck]) -> Result<(), LibraryError> {
        let tx = self.conn.transaction()?;
        for check in checks {
            let errors: Vec<&String> = check
                .errors
                .iter()
                .take(IntegrityCheck::MAX_ERRORS)
                .collect();
            tx.execute(
                &format!(
                    "INSERT OR REPLACE INTO integrity_checks ({COLUMNS}) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
                ),
                params![
                    check.track_id.0,
                    i64::from(check.clean),
                    i64::from(check.edge_only),
                    serde_json::to_string(&errors).expect("strings serialize"),
                    check.error_count,
                    check.file_modified_at,
                    check.ffmpeg_version,
                    check.checked_at
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn integrity_check(&self, track: TrackId) -> Result<Option<IntegrityCheck>, LibraryError> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM integrity_checks WHERE track_id = ?1"),
                [track.0],
                from_row,
            )
            .optional()?
            .map(finish)
            .transpose()
    }

    /// Present tracks never checked, or changed on disk since their check.
    pub fn tracks_needing_integrity_check(&self) -> Result<Vec<TrackId>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id FROM tracks t LEFT JOIN integrity_checks i ON i.track_id = t.id \
             WHERE t.missing_since IS NULL \
             AND (i.track_id IS NULL OR i.file_modified_at != t.modified_at) \
             ORDER BY t.path",
        )?;
        let ids = stmt
            .query_map([], |row| Ok(TrackId(row.get(0)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Tracks whose latest check found errors inside the music (not just
    /// junk at the start or end of the file).
    pub fn tracks_with_integrity_errors(&self) -> Result<Vec<TrackId>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT i.track_id FROM integrity_checks i JOIN tracks t ON t.id = i.track_id \
             WHERE i.clean = 0 AND i.edge_only = 0 ORDER BY t.path",
        )?;
        let ids = stmt
            .query_map([], |row| Ok(TrackId(row.get(0)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::tests::scanned;

    #[test]
    fn saves_and_finds_damaged_and_changed_tracks() {
        let mut lib = Library::open_in_memory().expect("open");
        let (a, _) = lib.upsert_scanned(&scanned("/m/a.flac")).expect("a");
        let (b, _) = lib.upsert_scanned(&scanned("/m/b.flac")).expect("b");
        assert_eq!(
            lib.tracks_needing_integrity_check().expect("pending"),
            [a, b]
        );
        let modified = lib.track(a).expect("a").modified_at;
        let check = |track, clean| IntegrityCheck {
            track_id: track,
            clean,
            edge_only: false,
            errors: if clean {
                Vec::new()
            } else {
                vec!["bad CRC".into()]
            },
            error_count: u32::from(!clean),
            file_modified_at: modified,
            ffmpeg_version: "9.1".into(),
            checked_at: 1,
        };
        lib.save_integrity_checks(&[check(a, true), check(b, false)])
            .expect("save");
        assert_eq!(lib.tracks_with_integrity_errors().expect("damaged"), [b]);
        assert_eq!(
            lib.integrity_check(b).expect("b").expect("stored").errors,
            ["bad CRC"]
        );
        assert!(
            lib.tracks_needing_integrity_check()
                .expect("pending")
                .is_empty()
        );
    }
}
