//! Cached waveform peaks, and batched saving of analysis results.

use overtone_model::{QualityReport, TrackId, UnixMillis};
use rusqlite::{OptionalExtension, params};

use crate::{Library, LibraryError, now_millis};

/// Waveform peaks stored for a track. `data` is opaque here; overtone-audio
/// encodes and decodes it (`Waveform::encode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredWaveform {
    pub analyzer_version: u32,
    pub data: Vec<u8>,
    pub created_at: UnixMillis,
}

/// One track's analysis output, for [`Library::save_analysis_batch`].
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisRecord {
    pub track: TrackId,
    pub report: QualityReport,
    /// Encoded waveform peaks; `None` when analysis failed.
    pub waveform: Option<Vec<u8>>,
}

impl Library {
    /// Stores or replaces the waveform peaks for `track`.
    pub fn save_waveform(
        &self,
        track: TrackId,
        analyzer_version: u32,
        data: &[u8],
    ) -> Result<(), LibraryError> {
        self.conn.execute(
            "INSERT INTO waveforms (track_id, analyzer_version, data, created_at) \
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT (track_id) DO UPDATE SET \
             analyzer_version = excluded.analyzer_version, data = excluded.data, \
             created_at = excluded.created_at",
            params![track.0, analyzer_version, data, now_millis()],
        )?;
        Ok(())
    }

    /// The stored waveform peaks for `track`, if any.
    pub fn waveform(&self, track: TrackId) -> Result<Option<StoredWaveform>, LibraryError> {
        Ok(self
            .conn
            .query_row(
                "SELECT analyzer_version, data, created_at FROM waveforms WHERE track_id = ?1",
                [track.0],
                |row| {
                    Ok(StoredWaveform {
                        analyzer_version: row.get(0)?,
                        data: row.get(1)?,
                        created_at: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// Saves quality reports and waveforms for many tracks in one
    /// transaction. Existing human reviews are kept, as with
    /// [`Library::save_quality_report`].
    pub fn save_analysis_batch(&mut self, records: &[AnalysisRecord]) -> Result<(), LibraryError> {
        let tx = self.conn.unchecked_transaction()?;
        for record in records {
            self.save_quality_report(record.track, &record.report)?;
            if let Some(data) = &record.waveform {
                self.save_waveform(record.track, record.report.analyzer_version, data)?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use overtone_model::{QualityVerdict, ReviewStatus};
    use rusqlite::Connection;
    use tempfile::TempDir;

    use super::*;
    use crate::tracks::tests::scanned;

    fn report() -> QualityReport {
        QualityReport {
            verdict: QualityVerdict::Pass,
            cutoff_hz: Some(21_900.0),
            confidence: Some(0.9),
            reason: Some("full band".into()),
            analyzer_version: 1,
            analyzed_at: 1,
            review: ReviewStatus::Unreviewed,
            reviewed_at: None,
        }
    }

    #[test]
    fn saves_replaces_and_cascades_waveforms() {
        let mut lib = Library::open_in_memory().expect("open");
        let (id, _) = lib.upsert_scanned(&scanned("/m/a.flac")).expect("track");
        assert_eq!(lib.waveform(id).expect("query"), None);
        lib.save_waveform(id, 1, &[1, 2, 3]).expect("save");
        lib.save_waveform(id, 2, &[1, 9]).expect("replace");
        let stored = lib.waveform(id).expect("query").expect("stored");
        assert_eq!(stored.analyzer_version, 2);
        assert_eq!(stored.data, vec![1, 9]);
        lib.forget_track(id).expect("forget");
        let count: i64 = lib
            .conn
            .query_row("SELECT COUNT(*) FROM waveforms", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 0);
    }

    #[test]
    fn batch_saves_reports_and_waveforms_together() {
        let mut lib = Library::open_in_memory().expect("open");
        let (a, _) = lib.upsert_scanned(&scanned("/m/a.flac")).expect("a");
        let (b, _) = lib.upsert_scanned(&scanned("/m/b.flac")).expect("b");
        lib.save_analysis_batch(&[
            AnalysisRecord {
                track: a,
                report: report(),
                waveform: Some(vec![1, 0, 0]),
            },
            AnalysisRecord {
                track: b,
                report: QualityReport {
                    verdict: QualityVerdict::Unchecked,
                    ..report()
                },
                waveform: None,
            },
        ])
        .expect("batch");
        assert!(lib.tracks_needing_analysis(1).expect("pending").is_empty());
        assert!(lib.waveform(a).expect("a").is_some());
        assert!(lib.waveform(b).expect("b").is_none());
    }

    #[test]
    fn migrates_a_version_one_library() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("library.sqlite");
        {
            let conn = Connection::open(&path).expect("create v1");
            conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
                .expect("v1 schema");
            conn.pragma_update(None, "user_version", 1)
                .expect("version");
            conn.execute(
                "INSERT INTO tracks (path, file_size, modified_at, added_at, updated_at) \
                 VALUES ('/m/old.flac', 1, 1, 1, 1)",
                [],
            )
            .expect("v1 track");
        }
        let lib = Library::open(&path).expect("migrate");
        assert_eq!(lib.summary().expect("summary").schema_version, 2);
        let track = lib
            .track_by_path("/m/old.flac")
            .expect("query")
            .expect("track kept");
        lib.save_waveform(track.id, 1, &[1])
            .expect("new table works");
    }
}
