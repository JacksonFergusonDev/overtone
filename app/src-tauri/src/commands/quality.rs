//! Quality audit, reviews, waveforms, and spectrogram images.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use overtone_audio::analysis::{ANALYZER_VERSION, SpectrogramOptions, Waveform};
use overtone_core::audit::{self, AuditOptions, AuditProgress, AuditReport};
use overtone_core::library::Library;
use overtone_core::model::{QualityReport, ReviewStatus, Track, TrackId};
use overtone_core::{CoreError, tags};
use tauri::State;
use tauri::ipc::Channel;

use super::join_error;
use crate::error::CommandError;
use crate::ipc::{SpectrogramImage, WaveformPeaks};
use crate::state::AppState;

pub fn mark_review(
    state: &AppState,
    track_id: TrackId,
    review: ReviewStatus,
) -> Result<Track, CommandError> {
    state.with_library(|library| {
        library.set_review(track_id, review)?;
        Ok(library.track(track_id)?)
    })
}

pub fn waveform_peaks(
    state: &AppState,
    track_id: TrackId,
) -> Result<Option<WaveformPeaks>, CommandError> {
    state.with_library(|library| {
        Ok(library
            .waveform(track_id)?
            .and_then(|stored| Waveform::decode(&stored.data))
            .map(|waveform| WaveformPeaks {
                buckets: waveform.buckets,
            }))
    })
}

/// Audits `ids` (or every track needing analysis when `None`) on its own
/// library connection. Finished tracks are saved even if `cancel` is set.
pub fn run_audit(
    library_path: &Path,
    ids: Option<Vec<TrackId>>,
    cancel: Arc<AtomicBool>,
    on_progress: &(dyn Fn(AuditProgress) + Sync),
) -> Result<AuditReport, CommandError> {
    let mut library = Library::open(library_path).map_err(CoreError::from)?;
    let ids = match ids {
        Some(ids) => ids,
        None => library
            .tracks_needing_analysis(ANALYZER_VERSION)
            .map_err(CoreError::from)?,
    };
    let options = AuditOptions {
        cancel: Some(cancel),
        ..AuditOptions::default()
    };
    Ok(audit::audit_tracks_with(
        &mut library,
        &ids,
        options,
        on_progress,
    )?)
}

fn cache_name(track_id: TrackId, log: bool, modified: i64, version: u32) -> String {
    let axis = if log { "log" } else { "lin" };
    format!("{}-{axis}-{modified}-{version}.png", track_id.0)
}

/// Deletes cached spectrograms of `track_id` other than the current
/// linear and logarithmic ones.
fn remove_stale_images(cache_dir: &Path, track_id: TrackId, modified: i64) {
    let keep = [false, true].map(|log| cache_name(track_id, log, modified, ANALYZER_VERSION));
    let prefix = format!("{}-", track_id.0);
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix)
            && Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
            && !keep.contains(&name)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Returns the spectrogram PNG for a track, rendering it unless a cached one
/// matches the file's modification time and the analyzer version. Rendering
/// also refreshes the track's stored report and waveform.
pub fn spectrogram(
    library_path: &Path,
    cache_dir: &Path,
    track_id: TrackId,
    log: bool,
) -> Result<SpectrogramImage, CommandError> {
    let mut library = Library::open(library_path).map_err(CoreError::from)?;
    let track = library.track(track_id).map_err(CoreError::from)?;
    let metadata = std::fs::metadata(&track.path)
        .map_err(|err| CommandError::io(format!("couldn't read {}: {err}", track.path)))?;
    let modified = tags::modified_millis(&metadata);
    std::fs::create_dir_all(cache_dir)
        .map_err(|err| CommandError::io(format!("couldn't create the image cache: {err}")))?;
    let png = cache_dir.join(cache_name(track_id, log, modified, ANALYZER_VERSION));

    let report: QualityReport = if png.exists() {
        match track.quality {
            Some(report) => report,
            None => audit::analyze_one(&mut library, track_id, None)?,
        }
    } else {
        let partial = png.with_extension("partial.png");
        let mut options = SpectrogramOptions::new(partial.clone());
        options.log_frequency = log;
        let report = audit::analyze_one(&mut library, track_id, Some(options))?;
        std::fs::rename(&partial, &png)
            .map_err(|err| CommandError::io(format!("couldn't save the spectrogram: {err}")))?;
        report
    };
    remove_stale_images(cache_dir, track_id, modified);
    Ok(SpectrogramImage {
        path: png.display().to_string(),
        report,
        log_frequency: log,
    })
}

#[tauri::command]
pub fn set_review(
    state: State<'_, AppState>,
    track_id: TrackId,
    review: ReviewStatus,
) -> Result<Track, CommandError> {
    mark_review(&state, track_id, review)
}

#[tauri::command]
pub fn track_waveform(
    state: State<'_, AppState>,
    track_id: TrackId,
) -> Result<Option<WaveformPeaks>, CommandError> {
    waveform_peaks(&state, track_id)
}

#[tauri::command]
pub async fn audit_tracks(
    state: State<'_, AppState>,
    track_ids: Option<Vec<TrackId>>,
    on_progress: Channel<AuditProgress>,
) -> Result<AuditReport, CommandError> {
    let slot = state.start_audit()?;
    let library_path = state.library_path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = slot.cancel_flag();
        // `slot` lives until the audit finishes, then frees the audit slot.
        run_audit(&library_path, track_ids, cancel, &|progress| {
            let _ = on_progress.send(progress);
        })
    })
    .await
    .map_err(|err| join_error(&err))?
}

#[tauri::command]
pub fn cancel_audit(state: State<'_, AppState>) {
    state.cancel_audit();
}

#[tauri::command]
pub async fn render_spectrogram(
    state: State<'_, AppState>,
    track_id: TrackId,
    log: bool,
) -> Result<SpectrogramImage, CommandError> {
    let library_path = state.library_path().to_path_buf();
    let cache_dir = state.spectrogram_dir().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        spectrogram(&library_path, &cache_dir, track_id, log)
    })
    .await
    .map_err(|err| join_error(&err))?
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;

    use overtone_core::model::QualityVerdict;
    use tempfile::TempDir;

    use super::*;
    use crate::commands::library::{tests::scanned_state, tracks};

    #[test]
    fn set_review_returns_the_updated_track() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 1);
        let id = tracks(&state).expect("tracks")[0].id;
        run_audit(state.library_path(), None, Arc::default(), &|_| {}).expect("audit");
        let track = mark_review(&state, id, ReviewStatus::VerifiedOk).expect("review");
        assert_eq!(
            track.quality.expect("report").review,
            ReviewStatus::VerifiedOk
        );
        let err = mark_review(&state, TrackId(999), ReviewStatus::VerifiedOk).expect_err("unknown");
        assert_eq!(err.code, "not_found");
    }

    #[test]
    fn audit_saves_waveforms_and_reports_progress() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 2);
        let seen = Mutex::new(0_u32);
        let report = run_audit(state.library_path(), None, Arc::default(), &|p| {
            *seen.lock().expect("lock") = p.done;
        })
        .expect("audit");
        assert_eq!(report.analyzed, 2);
        assert_eq!(*seen.lock().expect("lock"), 2);
        let id = tracks(&state).expect("tracks")[0].id;
        let peaks = waveform_peaks(&state, id).expect("peaks").expect("some");
        assert!(!peaks.buckets.is_empty());
        assert!(
            peaks
                .buckets
                .iter()
                .flatten()
                .all(|v| (-1.0..=1.0).contains(v))
        );
        // Nothing is pending afterwards.
        let again = run_audit(state.library_path(), None, Arc::default(), &|_| {}).expect("again");
        assert_eq!(again.analyzed, 0);
    }

    #[test]
    fn waveform_is_none_before_analysis() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 1);
        let id = tracks(&state).expect("tracks")[0].id;
        assert!(waveform_peaks(&state, id).expect("peaks").is_none());
    }

    #[test]
    fn cancelled_audit_reports_skipped_tracks() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 3);
        let slot = state.start_audit().expect("slot");
        state.cancel_audit();
        let report =
            run_audit(state.library_path(), None, slot.cancel_flag(), &|_| {}).expect("audit");
        assert!(report.cancelled);
        assert_eq!((report.analyzed, report.skipped), (0, 3));
        assert!(slot.cancel_flag().load(Ordering::Acquire));
        assert!(tracks(&state).expect("tracks")[0].quality.is_none());
    }

    #[test]
    fn spectrogram_is_cached_and_stale_images_are_removed() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 1);
        let id = tracks(&state).expect("tracks")[0].id;
        let (lib, cache) = (state.library_path(), state.spectrogram_dir());

        let first = spectrogram(lib, cache, id, false).expect("render");
        assert!(Path::new(&first.path).exists());
        assert!(
            first.path.contains(&format!("/{}-lin-", id.0)),
            "named after the track and axis: {}",
            first.path
        );
        assert!(!first.log_frequency);
        assert_eq!(first.report.verdict, QualityVerdict::Pass);
        // The render also stored the report and waveform.
        assert!(waveform_peaks(&state, id).expect("peaks").is_some());

        // A second call reuses the file: its modification time is unchanged.
        let before = std::fs::metadata(&first.path)
            .and_then(|m| m.modified())
            .expect("mtime");
        std::thread::sleep(std::time::Duration::from_millis(20));
        let again = spectrogram(lib, cache, id, false).expect("cached");
        assert_eq!(again.path, first.path);
        let after = std::fs::metadata(&again.path)
            .and_then(|m| m.modified())
            .expect("mtime");
        assert_eq!(before, after);
        assert_eq!(again.report, first.report);

        // The other axis is a separate image and both are kept.
        let log = spectrogram(lib, cache, id, true).expect("log");
        assert_ne!(log.path, first.path);
        assert!(Path::new(&first.path).exists() && Path::new(&log.path).exists());

        // A leftover from an older version of the file is deleted; another
        // track's image is not.
        let stale = cache.join(format!("{}-lin-1-1.png", id.0));
        let other = cache.join(format!("{}-lin-1-1.png", id.0 + 100));
        std::fs::write(&stale, b"old").expect("stale");
        std::fs::write(&other, b"other").expect("other");
        spectrogram(lib, cache, id, false).expect("cached");
        assert!(!stale.exists());
        assert!(other.exists());
    }

    #[test]
    fn spectrogram_of_a_missing_file_is_an_io_error() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 1);
        let track = tracks(&state).expect("tracks").remove(0);
        std::fs::remove_file(&track.path).expect("remove");
        let err = spectrogram(
            state.library_path(),
            state.spectrogram_dir(),
            track.id,
            false,
        )
        .expect_err("missing");
        assert_eq!(err.code, "io");
    }
}
