//! Stored lossless conversions.

use std::collections::HashSet;

use overtone_model::{Conversion, ConversionStatus, TrackId};
use rusqlite::{Row, params};

use crate::{Library, LibraryError, now_millis};

const COLUMNS: &str =
    "id, track_id, output_path, format, status, dropped, serato, frames, created_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<(Conversion, String, String, String)> {
    Ok((
        Conversion {
            id: row.get(0)?,
            track_id: TrackId(row.get(1)?),
            output_path: row.get(2)?,
            format: row.get(3)?,
            status: ConversionStatus::PendingRelocation,
            dropped: Vec::new(),
            serato: Vec::new(),
            frames: u64::try_from(row.get::<_, i64>(7)?).unwrap_or(0),
            created_at: row.get(8)?,
        },
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

fn finish(
    (mut c, status, dropped, serato): (Conversion, String, String, String),
) -> Result<Conversion, LibraryError> {
    c.status = ConversionStatus::parse(&status).ok_or(LibraryError::Corrupt {
        column: "status",
        value: status,
    })?;
    let parse = |column: &'static str, json: String| {
        serde_json::from_str(&json).map_err(|_| LibraryError::Corrupt {
            column,
            value: json,
        })
    };
    c.dropped = parse("dropped", dropped)?;
    c.serato = parse("serato", serato)?;
    Ok(c)
}

impl Library {
    /// Records a verified conversion waiting for relocation.
    pub fn record_conversion(
        &self,
        track: TrackId,
        output_path: &str,
        format: &str,
        dropped: &[String],
        serato: &[String],
        frames: u64,
    ) -> Result<Conversion, LibraryError> {
        self.conn.execute(
            "INSERT INTO conversions (track_id, output_path, format, status, dropped, serato, \
             frames, created_at) VALUES (?1, ?2, ?3, 'pending_relocation', ?4, ?5, ?6, ?7)",
            params![
                track.0,
                output_path,
                format,
                serde_json::to_string(dropped).expect("strings serialize"),
                serde_json::to_string(serato).expect("strings serialize"),
                i64::try_from(frames).unwrap_or(i64::MAX),
                now_millis()
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conversion(id)
    }

    pub fn conversion(&self, id: i64) -> Result<Conversion, LibraryError> {
        let found = self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM conversions WHERE id = ?1"),
                [id],
                from_row,
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => LibraryError::NotFound {
                    kind: "conversion",
                    id,
                },
                other => other.into(),
            })?;
        finish(found)
    }

    /// Conversions with `status` (or all), oldest first.
    pub fn conversions(
        &self,
        status: Option<ConversionStatus>,
    ) -> Result<Vec<Conversion>, LibraryError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM conversions WHERE (?1 IS NULL OR status = ?1) ORDER BY id"
        ))?;
        let rows = stmt
            .query_map([status.map(ConversionStatus::as_str)], from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(finish).collect()
    }

    /// The track's conversion waiting for relocation, if any.
    pub fn pending_conversion(&self, track: TrackId) -> Result<Option<Conversion>, LibraryError> {
        Ok(self
            .conversions(Some(ConversionStatus::PendingRelocation))?
            .into_iter()
            .find(|c| c.track_id == track))
    }

    /// Output paths of conversions waiting for relocation, which scans skip.
    pub fn pending_conversion_outputs(&self) -> Result<HashSet<String>, LibraryError> {
        Ok(self
            .conversions(Some(ConversionStatus::PendingRelocation))?
            .into_iter()
            .map(|c| c.output_path)
            .collect())
    }

    pub fn set_conversion_status(
        &self,
        id: i64,
        status: ConversionStatus,
    ) -> Result<(), LibraryError> {
        let changed = self.conn.execute(
            "UPDATE conversions SET status = ?2 WHERE id = ?1",
            params![id, status.as_str()],
        )?;
        if changed == 0 {
            return Err(LibraryError::NotFound {
                kind: "conversion",
                id,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::tests::scanned;

    #[test]
    fn records_pending_conversions_once_per_track() {
        let mut lib = Library::open_in_memory().expect("open");
        let (id, _) = lib.upsert_scanned(&scanned("/m/a.wav")).expect("track");
        let c = lib
            .record_conversion(
                id,
                "/m/a.flac",
                "flac",
                &["PRIV (x)".into()],
                &["Serato Markers2".into()],
                10,
            )
            .expect("record");
        assert_eq!(c.status, ConversionStatus::PendingRelocation);
        assert_eq!(c.dropped, ["PRIV (x)"]);
        assert!(
            lib.pending_conversion_outputs()
                .expect("outputs")
                .contains("/m/a.flac")
        );
        assert!(
            lib.record_conversion(id, "/m/a2.flac", "flac", &[], &[], 10)
                .is_err(),
            "one pending conversion per track"
        );
        lib.set_conversion_status(c.id, ConversionStatus::Discarded)
            .expect("discard");
        assert!(lib.pending_conversion(id).expect("pending").is_none());
    }
}
