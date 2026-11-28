//! Library-wide ffmpeg integrity checks: decode each file in full and keep
//! every error ffmpeg reports. Files are only read.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use overtone_ffmpeg::Ffmpeg;
use overtone_library::Library;
use overtone_model::{IntegrityCheck, TrackId, UnixMillis};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::CoreError;
use crate::audit::DEFAULT_AUDIT_THREADS;

const SAVE_BATCH: usize = 32;

/// Progress after each checked track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct IntegrityProgress {
    pub done: u32,
    pub total: u32,
    pub track_id: TrackId,
    pub clean: bool,
}

/// Summary of a finished run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct IntegrityRunReport {
    pub checked: u32,
    pub skipped: u32,
    pub cancelled: bool,
    /// Tracks with errors inside the music, in the order they finished.
    pub damaged: Vec<TrackId>,
    /// Tracks whose only errors are junk at the very start or end.
    pub edge_only: u32,
    pub ffmpeg_version: String,
    pub elapsed_ms: u64,
}

/// Tuning for [`check_tracks`].
#[derive(Debug, Clone)]
pub struct IntegrityOptions {
    pub threads: usize,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for IntegrityOptions {
    fn default() -> Self {
        Self {
            threads: DEFAULT_AUDIT_THREADS,
            cancel: None,
        }
    }
}

fn now_millis() -> UnixMillis {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since_epoch.as_millis()).unwrap_or(i64::MAX)
}

/// Skipped at the start of a file for the edge test.
const EDGE_START: Duration = Duration::from_millis(500);
/// Skipped at the end: trailing junk and tags, plus MP3 seek slop.
const EDGE_END: Duration = Duration::from_millis(2_500);

/// Whether a file with errors decodes cleanly once its first half second
/// and last few seconds are left out.
fn only_at_edges(ffmpeg: &Ffmpeg, path: &std::path::Path, duration_ms: Option<u64>) -> bool {
    let Some(duration) = duration_ms.map(Duration::from_millis) else {
        return false;
    };
    let Some(length) = duration.checked_sub(EDGE_START + EDGE_END) else {
        return false;
    };
    if length < Duration::from_secs(10) {
        return false;
    }
    ffmpeg
        .check_integrity_window(path, EDGE_START, length)
        .is_ok_and(|inner| inner.is_clean())
}

/// Checks `ids` in parallel and saves each result. Needs ffmpeg.
pub fn check_tracks(
    library: &mut Library,
    ids: &[TrackId],
    options: &IntegrityOptions,
    on_progress: &(dyn Fn(IntegrityProgress) + Sync),
) -> Result<IntegrityRunReport, CoreError> {
    let ffmpeg = Ffmpeg::locate()?;
    let started = Instant::now();
    let jobs: Vec<(TrackId, PathBuf, UnixMillis, Option<u64>)> = ids
        .iter()
        .map(|&id| {
            let track = library.track(id)?;
            Ok((
                id,
                PathBuf::from(track.path),
                track.modified_at,
                track.properties.duration_ms,
            ))
        })
        .collect::<Result<_, CoreError>>()?;
    let total = u32::try_from(jobs.len()).unwrap_or(u32::MAX);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.threads.max(1))
        .thread_name(|i| format!("overtone-integrity-{i}"))
        .build()
        .map_err(|err| CoreError::Worker(err.to_string()))?;
    let mut report = IntegrityRunReport {
        checked: 0,
        skipped: 0,
        cancelled: false,
        damaged: Vec::new(),
        edge_only: 0,
        ffmpeg_version: ffmpeg.version().to_string(),
        elapsed_ms: 0,
    };
    let cancel = options.cancel.clone();
    let is_cancelled = move || cancel.as_ref().is_some_and(|f| f.load(Ordering::Relaxed));
    let (sender, receiver) = mpsc::channel::<IntegrityCheck>();
    let ffmpeg = &ffmpeg;
    std::thread::scope(|scope| -> Result<(), CoreError> {
        scope.spawn(move || {
            pool.install(|| {
                jobs.par_iter().for_each_with(
                    sender,
                    |sender, (id, path, modified, duration_ms)| {
                        if is_cancelled() {
                            return;
                        }
                        let (clean, errors) = match ffmpeg.check_integrity(path) {
                            Ok(found) => (found.is_clean(), found.errors),
                            Err(err) => (false, vec![err.to_string()]),
                        };
                        let edge_only = !clean && only_at_edges(ffmpeg, path, *duration_ms);
                        let check = IntegrityCheck {
                            track_id: *id,
                            clean,
                            edge_only,
                            error_count: u32::try_from(errors.len()).unwrap_or(u32::MAX),
                            errors: errors
                                .into_iter()
                                .take(IntegrityCheck::MAX_ERRORS)
                                .collect(),
                            file_modified_at: *modified,
                            ffmpeg_version: ffmpeg.version().to_string(),
                            checked_at: now_millis(),
                        };
                        let _ = sender.send(check);
                    },
                );
            });
        });
        let mut batch = Vec::with_capacity(SAVE_BATCH);
        for check in receiver {
            report.checked += 1;
            if check.is_damaged() {
                report.damaged.push(check.track_id);
            } else if !check.clean {
                report.edge_only += 1;
            }
            on_progress(IntegrityProgress {
                done: report.checked,
                total,
                track_id: check.track_id,
                clean: !check.is_damaged(),
            });
            batch.push(check);
            if batch.len() >= SAVE_BATCH {
                library.save_integrity_checks(&batch)?;
                batch.clear();
            }
        }
        library.save_integrity_checks(&batch)?;
        Ok(())
    })?;
    report.skipped = total.saturating_sub(report.checked);
    report.cancelled = report.skipped > 0;
    report.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use overtone_library::ScannedTrack;
    use overtone_testkit::{Fixture, Signal, TempLibrary};

    use super::*;

    #[test]
    fn finds_a_damaged_file_and_skips_unchanged_ones_next_time() {
        if Ffmpeg::locate().is_err() {
            eprintln!("skipping: needs ffmpeg");
            return;
        }
        let mut temp = TempLibrary::new();
        let good = temp.add_fixture("good", &Signal::cd(16_000.0), Fixture::Flac { bits: 16 });
        let bad = temp.add_fixture("bad", &Signal::cd(16_000.0), Fixture::Flac { bits: 16 });
        let mut bytes = std::fs::read(&bad).expect("read");
        let mid = bytes.len() / 2;
        for b in &mut bytes[mid..mid + 64] {
            *b ^= 0x5A;
        }
        std::fs::write(&bad, bytes).expect("corrupt");
        let mut ids = Vec::new();
        for path in [&good, &bad] {
            let (id, _) = temp
                .library
                .upsert_scanned(&ScannedTrack {
                    path: path.display().to_string(),
                    file_size: 1,
                    modified_at: 1,
                    audio_hash: None,
                    properties: overtone_model::AudioProperties::default(),
                    tags: overtone_model::TrackTags::default(),
                })
                .expect("insert");
            ids.push(id);
        }
        let pending = temp
            .library
            .tracks_needing_integrity_check()
            .expect("pending");
        let report = check_tracks(
            &mut temp.library,
            &pending,
            &IntegrityOptions::default(),
            &|_| {},
        )
        .expect("check");
        assert_eq!(report.checked, 2);
        assert_eq!(report.damaged, [ids[1]]);
        assert!(
            temp.library
                .tracks_needing_integrity_check()
                .expect("pending")
                .is_empty()
        );
        let stored = temp
            .library
            .integrity_check(ids[1])
            .expect("q")
            .expect("stored");
        assert!(!stored.clean && !stored.errors.is_empty());
    }
}
