//! Status, tracks, library folders, and scanning.

use std::path::Path;
use std::time::Duration;

use overtone_core::library::{Library, LibraryRoot};
use overtone_core::model::Track;
use overtone_core::scan::{self, ScanOptions, ScanProgress, ScanReport};
use overtone_core::{AppStatus, CoreError};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime, State};

use super::{Throttle, join_error};
use crate::error::CommandError;
use crate::state::AppState;

/// Scan progress is chatty (once per file); the webview gets ~20 per second.
const SCAN_PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

pub fn tracks(state: &AppState) -> Result<Vec<Track>, CommandError> {
    state.with_library(|library| Ok(library.tracks()?))
}

pub fn roots(state: &AppState) -> Result<Vec<LibraryRoot>, CommandError> {
    state.with_library(|library| Ok(library.roots()?))
}

/// Adds `path` as a library folder (canonicalized by core).
pub fn add_library_root(state: &AppState, path: &Path) -> Result<LibraryRoot, CommandError> {
    state.with_library(|library| scan::add_root(library, path))
}

/// Stops indexing a folder. Its tracks stay in the library. Returns the
/// folder that was removed, if the id existed.
pub fn forget_root(state: &AppState, id: i64) -> Result<Option<LibraryRoot>, CommandError> {
    state.with_library(|library| {
        let removed = library.roots()?.into_iter().find(|root| root.id == id);
        library.remove_root(id)?;
        Ok(removed)
    })
}

/// Scans every library folder on its own connection to `library_path`, so
/// the UI can keep reading meanwhile.
pub fn run_scan(
    library_path: &Path,
    refresh: bool,
    on_progress: &(dyn Fn(ScanProgress) + Sync),
) -> Result<ScanReport, CommandError> {
    let mut library = Library::open(library_path).map_err(CoreError::from)?;
    Ok(scan::scan_library_with(
        &mut library,
        ScanOptions { refresh },
        on_progress,
    )?)
}

/// Wraps `send` so at most ~20 events per second go through, always
/// including each phase's first and last event.
fn throttled(send: impl Fn(ScanProgress) + Sync) -> impl Fn(ScanProgress) + Sync {
    let throttle = Throttle::new(SCAN_PROGRESS_INTERVAL);
    move |progress| {
        let (key, is_final) = match progress {
            ScanProgress::Walking { .. } => (0, false),
            ScanProgress::Processing { done, total } => (1, done >= total),
            ScanProgress::Saving { done, total } => (2, done >= total),
        };
        if throttle.allow(key, is_final) {
            send(progress);
        }
    }
}

#[tauri::command]
pub fn app_status(state: State<'_, AppState>) -> Result<AppStatus, CommandError> {
    state.status()
}

#[tauri::command(async)]
pub fn list_tracks(state: State<'_, AppState>) -> Result<Vec<Track>, CommandError> {
    tracks(&state)
}

#[tauri::command]
pub fn list_roots(state: State<'_, AppState>) -> Result<Vec<LibraryRoot>, CommandError> {
    roots(&state)
}

/// Adds a library folder and lets the webview read audio inside it.
#[tauri::command]
pub fn add_root<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    path: String,
) -> Result<LibraryRoot, CommandError> {
    let root = add_library_root(&state, Path::new(&path))?;
    allow_asset_directory(&app, Path::new(&root.path));
    Ok(root)
}

#[tauri::command]
pub fn remove_root(state: State<'_, AppState>, id: i64) -> Result<(), CommandError> {
    // Tauri's asset scope can only grow (a forbidden pattern can't be lifted
    // again if the folder is re-added), so the removed folder stays readable
    // until the app restarts. It is a folder the DJ indexed themselves.
    forget_root(&state, id).map(|_| ())
}

#[tauri::command]
pub async fn scan_library(
    state: State<'_, AppState>,
    refresh: bool,
    on_progress: Channel<ScanProgress>,
) -> Result<ScanReport, CommandError> {
    let library_path = state.library_path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let send = throttled(|progress| {
            let _ = on_progress.send(progress);
        });
        run_scan(&library_path, refresh, &send)
    })
    .await
    .map_err(|err| join_error(&err))?
}

/// Allows the webview to read files under `dir` through the asset protocol.
pub fn allow_asset_directory<R: Runtime>(app: &AppHandle<R>, dir: &Path) {
    if let Err(err) = app.asset_protocol_scope().allow_directory(dir, true) {
        eprintln!("couldn't allow {} in the asset scope: {err}", dir.display());
    }
}

/// Allows every library folder and the spectrogram cache; called at startup.
pub fn allow_startup_directories<R: Runtime>(app: &AppHandle<R>, state: &AppState) {
    if let Err(err) = std::fs::create_dir_all(state.spectrogram_dir()) {
        eprintln!("couldn't create the spectrogram cache: {err}");
    }
    allow_asset_directory(app, state.spectrogram_dir());
    if let Ok(roots) = roots(state) {
        for root in roots {
            allow_asset_directory(app, Path::new(&root.path));
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::sync::Mutex;

    use overtone_testkit::{Fixture, Signal, write_fixture};
    use tempfile::TempDir;

    use super::*;
    use crate::state::tests::state_in;

    /// A temp library with `count` fixture tracks scanned in.
    pub fn scanned_state(dir: &TempDir, count: usize) -> AppState {
        let music = dir.path().join("music");
        for i in 0..count {
            let path = music.join(format!("track{i}.flac"));
            std::fs::create_dir_all(&music).expect("music dir");
            write_fixture(
                &path,
                &Signal::noise(22_050.0 - 1000.0 * i as f32),
                Fixture::Flac { bits: 16 },
            )
            .expect("fixture");
        }
        let state = state_in(dir);
        add_library_root(&state, &music).expect("add root");
        let report = run_scan(state.library_path(), false, &|_| {}).expect("scan");
        assert_eq!(report.added, u64::try_from(count).expect("count"));
        state
    }

    #[test]
    fn scan_then_list_tracks_and_roots() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 2);
        let tracks = tracks(&state).expect("tracks");
        assert_eq!(tracks.len(), 2);
        assert!(tracks.iter().all(|t| t.missing_since.is_none()));
        assert_eq!(roots(&state).expect("roots").len(), 1);
        assert_eq!(state.status().expect("status").library.track_count, 2);
    }

    #[test]
    fn scan_streams_progress_ending_with_the_last_event() {
        let dir = TempDir::new().expect("tempdir");
        let state = state_in(&dir);
        let music = dir.path().join("music");
        std::fs::create_dir_all(&music).expect("music dir");
        for i in 0..3 {
            write_fixture(
                &music.join(format!("t{i}.wav")),
                &Signal::cd(16_000.0),
                Fixture::Wav { bits: 16 },
            )
            .expect("fixture");
        }
        add_library_root(&state, &music).expect("add");
        let seen = Mutex::new(Vec::new());
        {
            let send = throttled(|p| seen.lock().expect("lock").push(p));
            run_scan(state.library_path(), false, &send).expect("scan");
        }
        let seen = seen.into_inner().expect("lock");
        assert!(matches!(
            seen.last(),
            Some(ScanProgress::Saving { done: 3, total: 3 })
        ));
    }

    #[test]
    fn invalid_roots_are_rejected_and_removed_roots_keep_their_tracks() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 1);
        let err = add_library_root(&state, &dir.path().join("nope")).expect_err("missing folder");
        assert_eq!(err.code, "invalid_root");
        let id = roots(&state).expect("roots")[0].id;
        let removed = forget_root(&state, id).expect("remove").expect("existed");
        assert_eq!(removed.id, id);
        assert!(roots(&state).expect("roots").is_empty());
        assert_eq!(tracks(&state).expect("tracks").len(), 1);
    }
}
