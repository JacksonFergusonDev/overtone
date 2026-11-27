//! Library-wide spectral quality audit.
//!
//! Tracks are analyzed in parallel on a bounded rayon pool, read-only.
//! Results flow back to the calling thread, which owns the library
//! connection, and are saved in batched transactions. A file that can't be
//! decoded (or crashes the decoder) is recorded as Unchecked with the
//! reason, and the batch carries on.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Instant;

use overtone_audio::analysis::{
    ANALYZER_VERSION, AnalysisOptions, SpectrogramOptions, analyze, unchecked_report,
};
use overtone_library::{AnalysisRecord, Library};
use overtone_model::{QualityReport, QualityVerdict, TrackId};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::CoreError;

/// Worker threads for an audit. Four matches the M3's performance cores and
/// measured as the best sustained throughput on a fanless MacBook Air; see
/// "Measurements" in `engineering/decisions/audio-quality-analysis.md`.
pub const DEFAULT_AUDIT_THREADS: usize = 4;
/// Results saved per transaction.
const SAVE_BATCH: usize = 32;

/// Tuning for [`audit_tracks_with`].
#[derive(Debug, Clone)]
pub struct AuditOptions {
    pub threads: usize,
    /// Set to `true` to stop the audit. Tracks already being analyzed
    /// finish and are saved; the rest are skipped.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for AuditOptions {
    fn default() -> Self {
        Self {
            threads: DEFAULT_AUDIT_THREADS,
            cancel: None,
        }
    }
}

/// Progress after each analyzed track.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct AuditProgress {
    pub done: u32,
    pub total: u32,
    pub track_id: TrackId,
    pub verdict: QualityVerdict,
}

/// Counts of verdicts from one audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct VerdictCounts {
    pub pass: u32,
    pub warning: u32,
    pub suspect: u32,
    pub inconclusive: u32,
    /// Files that couldn't be analyzed.
    pub unchecked: u32,
}

impl VerdictCounts {
    fn add(&mut self, verdict: QualityVerdict) {
        let slot = match verdict {
            QualityVerdict::Pass => &mut self.pass,
            QualityVerdict::Warning => &mut self.warning,
            QualityVerdict::Suspect => &mut self.suspect,
            QualityVerdict::Inconclusive => &mut self.inconclusive,
            QualityVerdict::Unchecked => &mut self.unchecked,
        };
        *slot += 1;
    }
}

/// Summary of a finished audit.
#[derive(Debug, Clone, PartialEq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct AuditReport {
    pub analyzed: u32,
    /// Tracks left unanalyzed because the audit was cancelled.
    pub skipped: u32,
    /// Whether the audit was stopped before it finished.
    pub cancelled: bool,
    pub verdicts: VerdictCounts,
    /// Tracks now flagged Warning or Suspect, in the order they finished.
    pub flagged: Vec<TrackId>,
    pub analyzer_version: u32,
    pub elapsed_ms: u64,
}

/// Analyzes `ids` with the default thread count. See [`audit_tracks_with`].
pub fn audit_tracks(
    library: &mut Library,
    ids: &[TrackId],
    on_progress: &(dyn Fn(AuditProgress) + Sync),
) -> Result<AuditReport, CoreError> {
    audit_tracks_with(library, ids, AuditOptions::default(), on_progress)
}

/// Analyzes every track with no report, or a report from an older analyzer.
pub fn audit_pending(
    library: &mut Library,
    on_progress: &(dyn Fn(AuditProgress) + Sync),
) -> Result<AuditReport, CoreError> {
    let ids = library.tracks_needing_analysis(ANALYZER_VERSION)?;
    audit_tracks(library, &ids, on_progress)
}

/// Analyzes `ids` in parallel and saves each report and waveform. Audio
/// files are only read. Unknown track ids are an error before any work
/// starts; per-file failures are recorded as Unchecked.
pub fn audit_tracks_with(
    library: &mut Library,
    ids: &[TrackId],
    options: AuditOptions,
    on_progress: &(dyn Fn(AuditProgress) + Sync),
) -> Result<AuditReport, CoreError> {
    let started = Instant::now();
    let jobs: Vec<(TrackId, PathBuf)> = ids
        .iter()
        .map(|&id| Ok((id, PathBuf::from(library.track(id)?.path))))
        .collect::<Result<_, CoreError>>()?;
    let total = u32::try_from(jobs.len()).unwrap_or(u32::MAX);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.threads.max(1))
        .thread_name(|i| format!("overtone-audit-{i}"))
        .build()
        .map_err(|err| CoreError::Worker(err.to_string()))?;

    let mut report = AuditReport {
        analyzed: 0,
        skipped: 0,
        cancelled: false,
        verdicts: VerdictCounts::default(),
        flagged: Vec::new(),
        analyzer_version: ANALYZER_VERSION,
        elapsed_ms: 0,
    };
    let cancel = options.cancel;
    let is_cancelled = move || {
        cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    };
    let (sender, receiver) = mpsc::channel::<AnalysisRecord>();
    std::thread::scope(|scope| -> Result<(), CoreError> {
        scope.spawn(move || {
            pool.install(|| {
                jobs.par_iter().for_each_with(sender, |sender, (id, path)| {
                    if is_cancelled() {
                        return;
                    }
                    let record = match analyze(path, &AnalysisOptions::default()) {
                        Ok(analysis) => AnalysisRecord {
                            track: *id,
                            report: analysis.report,
                            waveform: Some(analysis.waveform.encode()),
                        },
                        Err(err) => AnalysisRecord {
                            track: *id,
                            report: unchecked_report(err.to_string()),
                            waveform: None,
                        },
                    };
                    // The receiver only hangs up after a save error.
                    let _ = sender.send(record);
                });
            });
        });

        let mut batch = Vec::with_capacity(SAVE_BATCH);
        for record in receiver {
            let verdict = record.report.verdict;
            report.analyzed += 1;
            report.verdicts.add(verdict);
            if verdict.needs_review() {
                report.flagged.push(record.track);
            }
            on_progress(AuditProgress {
                done: report.analyzed,
                total,
                track_id: record.track,
                verdict,
            });
            batch.push(record);
            if batch.len() >= SAVE_BATCH {
                library.save_analysis_batch(&batch)?;
                batch.clear();
            }
        }
        library.save_analysis_batch(&batch)?;
        Ok(())
    })?;
    report.skipped = total.saturating_sub(report.analyzed);
    report.cancelled = report.skipped > 0;
    report.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(report)
}

/// Analyzes one track, optionally writing a spectrogram PNG, and saves its
/// report and waveform like an audit does. Returns the stored report, which
/// keeps any review the DJ already made. Unlike an audit, a file that can't
/// be decoded is an error, since the caller asked for this track.
pub fn analyze_one(
    library: &mut Library,
    id: TrackId,
    spectrogram: Option<SpectrogramOptions>,
) -> Result<QualityReport, CoreError> {
    let track = library.track(id)?;
    let analysis = analyze(Path::new(&track.path), &AnalysisOptions { spectrogram })?;
    library.save_analysis_batch(&[AnalysisRecord {
        track: id,
        report: analysis.report.clone(),
        waveform: Some(analysis.waveform.encode()),
    }])?;
    Ok(library.track(id)?.quality.unwrap_or(analysis.report))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use overtone_audio::analysis::Waveform;
    use overtone_library::ScannedTrack;
    use overtone_testkit::{Fixture, Signal, TempLibrary};

    use super::*;

    fn add(temp: &mut TempLibrary, name: &str, signal: &Signal) -> TrackId {
        let path = temp.add_fixture(name, signal, Fixture::Flac { bits: 16 });
        let size = std::fs::metadata(&path).expect("stat").len();
        temp.library
            .upsert_scanned(&ScannedTrack {
                path: path.display().to_string(),
                file_size: size,
                ..ScannedTrack::default()
            })
            .expect("insert")
            .0
    }

    #[test]
    fn audits_pending_tracks_and_records_failures() {
        let mut temp = TempLibrary::new();
        let good = add(&mut temp, "full", &Signal::noise(22_050.0));
        let fake = add(&mut temp, "fake", &Signal::noise(16_000.0));
        let broken_path = temp.music_dir().join("broken.flac");
        std::fs::write(&broken_path, b"fLaC not really").expect("write broken");
        let broken = temp
            .library
            .upsert_scanned(&ScannedTrack {
                path: broken_path.display().to_string(),
                file_size: 15,
                ..ScannedTrack::default()
            })
            .expect("insert")
            .0;
        let before = std::fs::read(&broken_path).expect("read");

        let seen = Mutex::new(Vec::new());
        let report = audit_pending(&mut temp.library, &|p| {
            seen.lock().expect("lock").push(p.done);
        })
        .expect("audit");

        assert_eq!(report.analyzed, 3);
        assert_eq!(report.verdicts.pass, 1);
        assert_eq!(report.verdicts.suspect, 1);
        assert_eq!(report.verdicts.unchecked, 1);
        assert_eq!(report.flagged, vec![fake]);
        let mut done = seen.into_inner().expect("lock");
        done.sort_unstable();
        assert_eq!(done, vec![1, 2, 3]);

        let stored = temp
            .library
            .track(broken)
            .expect("broken")
            .quality
            .expect("report");
        assert_eq!(stored.verdict, QualityVerdict::Unchecked);
        assert!(stored.reason.is_some());
        assert_eq!(std::fs::read(&broken_path).expect("read"), before);

        let peaks = temp.library.waveform(good).expect("query").expect("peaks");
        let waveform = Waveform::decode(&peaks.data).expect("decode");
        assert!(!waveform.buckets.is_empty());
        assert!(temp.library.waveform(broken).expect("query").is_none());

        // Nothing is pending once every track has a current report.
        let again = audit_pending(&mut temp.library, &|_| {}).expect("re-audit");
        assert_eq!(again.analyzed, 0);
    }

    #[test]
    fn cancelled_audit_skips_remaining_tracks() {
        let mut temp = TempLibrary::new();
        let ids: Vec<TrackId> = (0..3)
            .map(|i| add(&mut temp, &format!("t{i}"), &Signal::noise(22_050.0)))
            .collect();
        let cancel = Arc::new(AtomicBool::new(true));
        let options = AuditOptions {
            threads: 2,
            cancel: Some(Arc::clone(&cancel)),
        };
        let report = audit_tracks_with(&mut temp.library, &ids, options, &|_| {}).expect("audit");
        assert!(report.cancelled);
        assert_eq!(report.analyzed, 0);
        assert_eq!(report.skipped, 3);
        assert!(temp.library.track(ids[0]).expect("track").quality.is_none());

        cancel.store(false, Ordering::Relaxed);
        let options = AuditOptions {
            threads: 2,
            cancel: Some(cancel),
        };
        let report = audit_tracks_with(&mut temp.library, &ids, options, &|_| {}).expect("audit");
        assert!(!report.cancelled);
        assert_eq!((report.analyzed, report.skipped), (3, 0));
    }

    #[test]
    fn analyze_one_saves_report_and_waveform_and_writes_the_png() {
        let mut temp = TempLibrary::new();
        let id = add(&mut temp, "one", &Signal::noise(16_000.0));
        let png = temp.dir.path().join("one.png");
        let report = analyze_one(
            &mut temp.library,
            id,
            Some(SpectrogramOptions::new(png.clone())),
        )
        .expect("analyze");
        assert_eq!(report.verdict, QualityVerdict::Suspect);
        assert!(png.exists());
        assert!(temp.library.waveform(id).expect("query").is_some());
        assert_eq!(temp.library.track(id).expect("track").quality, Some(report));
    }
}
