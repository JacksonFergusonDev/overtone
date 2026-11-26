use std::collections::HashMap;

use overtone_model::{QualityReport, QualityVerdict, ReviewStatus, TrackId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::{Library, LibraryError, now_millis};

const REPORT_COLUMNS: &str = "track_id, verdict, cutoff_hz, confidence, reason, analyzer_version, \
    analyzed_at, review, reviewed_at";

impl Library {
    /// Stores a fresh analysis result. An existing human review is kept,
    /// because re-running analysis must never erase the DJ's decision.
    pub fn save_quality_report(
        &self,
        track: TrackId,
        report: &QualityReport,
    ) -> Result<(), LibraryError> {
        self.conn.execute(
            "INSERT INTO quality_reports (track_id, verdict, cutoff_hz, confidence, reason, \
             analyzer_version, analyzed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT (track_id) DO UPDATE SET verdict = excluded.verdict, \
             cutoff_hz = excluded.cutoff_hz, confidence = excluded.confidence, \
             reason = excluded.reason, analyzer_version = excluded.analyzer_version, \
             analyzed_at = excluded.analyzed_at",
            params![
                track.0,
                report.verdict.as_str(),
                report.cutoff_hz,
                report.confidence,
                report.reason,
                report.analyzer_version,
                report.analyzed_at,
            ],
        )?;
        Ok(())
    }

    /// Records the DJ's review of a track's quality report.
    pub fn set_review(&self, track: TrackId, review: ReviewStatus) -> Result<(), LibraryError> {
        let reviewed_at = (review != ReviewStatus::Unreviewed).then(now_millis);
        let updated = self.conn.execute(
            "UPDATE quality_reports SET review = ?2, reviewed_at = ?3 WHERE track_id = ?1",
            params![track.0, review.as_str(), reviewed_at],
        )?;
        if updated == 0 {
            return Err(LibraryError::NotFound {
                kind: "quality report for track",
                id: track.0,
            });
        }
        Ok(())
    }

    /// Tracks flagged Warning or Suspect that haven't been reviewed, worst first.
    pub fn tracks_needing_review(&self) -> Result<Vec<TrackId>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT q.track_id FROM quality_reports q JOIN tracks t ON t.id = q.track_id \
             WHERE q.verdict IN ('warning', 'suspect') AND q.review = 'unreviewed' \
             ORDER BY q.verdict = 'suspect' DESC, t.path",
        )?;
        let ids = stmt
            .query_map([], |row| Ok(TrackId(row.get(0)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Tracks with no report, or a report from an analyzer older than `version`.
    pub fn tracks_needing_analysis(&self, version: u32) -> Result<Vec<TrackId>, LibraryError> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id FROM tracks t LEFT JOIN quality_reports q ON q.track_id = t.id \
             WHERE t.missing_since IS NULL AND (q.track_id IS NULL OR q.analyzer_version < ?1) \
             ORDER BY t.path",
        )?;
        let ids = stmt
            .query_map([version], |row| Ok(TrackId(row.get(0)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }
}

pub(crate) fn report_for(
    conn: &Connection,
    track: TrackId,
) -> Result<Option<QualityReport>, LibraryError> {
    let sql = format!("SELECT {REPORT_COLUMNS} FROM quality_reports WHERE track_id = ?1");
    conn.query_row(&sql, [track.0], report_from_row)
        .optional()?
        .map(|(_, report)| report)
        .transpose()
}

pub(crate) fn all_reports(
    conn: &Connection,
) -> Result<HashMap<TrackId, QualityReport>, LibraryError> {
    let sql = format!("SELECT {REPORT_COLUMNS} FROM quality_reports");
    let mut stmt = conn.prepare(&sql)?;
    let mut reports = HashMap::new();
    for row in stmt.query_map([], report_from_row)? {
        let (id, report) = row?;
        reports.insert(id, report?);
    }
    Ok(reports)
}

type ReportRow = (TrackId, Result<QualityReport, LibraryError>);

fn report_from_row(row: &Row<'_>) -> rusqlite::Result<ReportRow> {
    let id = TrackId(row.get(0)?);
    let verdict: String = row.get(1)?;
    let review: String = row.get(7)?;
    let Some(verdict) = QualityVerdict::parse(&verdict) else {
        return Ok((
            id,
            Err(LibraryError::Corrupt {
                column: "quality_reports.verdict",
                value: verdict,
            }),
        ));
    };
    let Some(review) = ReviewStatus::parse(&review) else {
        return Ok((
            id,
            Err(LibraryError::Corrupt {
                column: "quality_reports.review",
                value: review,
            }),
        ));
    };
    Ok((
        id,
        Ok(QualityReport {
            verdict,
            cutoff_hz: row.get(2)?,
            confidence: row.get(3)?,
            reason: row.get(4)?,
            analyzer_version: row.get(5)?,
            analyzed_at: row.get(6)?,
            review,
            reviewed_at: row.get(8)?,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::tests::scanned;

    fn report(verdict: QualityVerdict, version: u32) -> QualityReport {
        QualityReport {
            verdict,
            cutoff_hz: Some(16_000.0),
            confidence: Some(0.9),
            reason: Some("16 kHz shelf".into()),
            analyzer_version: version,
            analyzed_at: 1,
            review: ReviewStatus::Unreviewed,
            reviewed_at: None,
        }
    }

    #[test]
    fn reanalysis_keeps_human_review() {
        let mut lib = Library::open_in_memory().expect("open");
        let (id, _) = lib.upsert_scanned(&scanned("/m/a.flac")).expect("track");
        lib.save_quality_report(id, &report(QualityVerdict::Suspect, 1))
            .expect("save");
        assert_eq!(lib.tracks_needing_review().expect("review"), vec![id]);
        assert_eq!(lib.summary().expect("summary").needs_review_count, 1);

        lib.set_review(id, ReviewStatus::VerifiedOk)
            .expect("review");
        lib.save_quality_report(id, &report(QualityVerdict::Warning, 2))
            .expect("reanalyze");
        let stored = lib.track(id).expect("load").quality.expect("report");
        assert_eq!(stored.verdict, QualityVerdict::Warning);
        assert_eq!(stored.review, ReviewStatus::VerifiedOk);
        assert!(stored.reviewed_at.is_some());
        assert!(lib.tracks_needing_review().expect("review").is_empty());
    }

    #[test]
    fn finds_tracks_needing_analysis() {
        let mut lib = Library::open_in_memory().expect("open");
        let (a, _) = lib.upsert_scanned(&scanned("/m/a.flac")).expect("a");
        let (b, _) = lib.upsert_scanned(&scanned("/m/b.flac")).expect("b");
        lib.save_quality_report(a, &report(QualityVerdict::Pass, 1))
            .expect("save");
        assert_eq!(lib.tracks_needing_analysis(1).expect("query"), vec![b]);
        assert_eq!(lib.tracks_needing_analysis(2).expect("query"), vec![a, b]);
    }

    #[test]
    fn reviewing_an_unanalyzed_track_fails() {
        let mut lib = Library::open_in_memory().expect("open");
        let (id, _) = lib.upsert_scanned(&scanned("/m/a.flac")).expect("track");
        assert!(matches!(
            lib.set_review(id, ReviewStatus::VerifiedOk),
            Err(LibraryError::NotFound { .. })
        ));
    }
}
