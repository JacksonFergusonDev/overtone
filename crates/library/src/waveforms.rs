//! Cached waveform peaks, and batched saving of analysis results.

use overtone_model::{LoudnessReport, QualityReport, TrackId, UnixMillis};
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
    /// Loudness measurements; `None` when analysis failed.
    pub loudness: Option<LoudnessReport>,
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

    /// Stores or replaces the loudness measurements for `track`.
    pub fn save_loudness(
        &self,
        track: TrackId,
        loudness: &LoudnessReport,
    ) -> Result<(), LibraryError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO loudness_reports (track_id, integrated_lufs, \
             loudness_range_lu, true_peak_dbtp, sample_peak_dbfs, clipped_runs, \
             leading_silence_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                track.0,
                loudness.integrated_lufs,
                loudness.loudness_range_lu,
                loudness.true_peak_dbtp,
                loudness.sample_peak_dbfs,
                i64::try_from(loudness.clipped_runs).unwrap_or(i64::MAX),
                i64::try_from(loudness.leading_silence_ms).unwrap_or(i64::MAX),
            ],
        )?;
        Ok(())
    }

    /// The stored loudness measurements for `track`, if any.
    pub fn loudness(&self, track: TrackId) -> Result<Option<LoudnessReport>, LibraryError> {
        Ok(self
            .conn
            .query_row(
                "SELECT integrated_lufs, loudness_range_lu, true_peak_dbtp, sample_peak_dbfs, \
                 clipped_runs, leading_silence_ms FROM loudness_reports WHERE track_id = ?1",
                [track.0],
                |row| {
                    Ok(LoudnessReport {
                        integrated_lufs: row.get(0)?,
                        loudness_range_lu: row.get(1)?,
                        true_peak_dbtp: row.get(2)?,
                        sample_peak_dbfs: row.get(3)?,
                        clipped_runs: u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                        leading_silence_ms: u64::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
                    })
                },
            )
            .optional()?)
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
            if let Some(loudness) = &record.loudness {
                self.save_loudness(record.track, loudness)?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::SCHEMA_VERSION;
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
                loudness: Some(LoudnessReport {
                    integrated_lufs: Some(-8.5),
                    loudness_range_lu: Some(4.0),
                    true_peak_dbtp: Some(0.4),
                    sample_peak_dbfs: Some(-0.1),
                    clipped_runs: 3,
                    leading_silence_ms: 120,
                }),
            },
            AnalysisRecord {
                track: b,
                report: QualityReport {
                    verdict: QualityVerdict::Unchecked,
                    ..report()
                },
                waveform: None,
                loudness: None,
            },
        ])
        .expect("batch");
        assert!(lib.tracks_needing_analysis(1).expect("pending").is_empty());
        assert_eq!(
            lib.loudness(a).expect("a").expect("stored").integrated_lufs,
            Some(-8.5)
        );
        assert!(lib.loudness(b).expect("b").is_none());
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
        assert_eq!(
            lib.summary().expect("summary").schema_version,
            SCHEMA_VERSION
        );
        let track = lib
            .track_by_path("/m/old.flac")
            .expect("query")
            .expect("track kept");
        lib.save_waveform(track.id, 1, &[1])
            .expect("new table works");
    }
}
