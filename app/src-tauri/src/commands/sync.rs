//! Syncing vibes to DJ software.

use std::path::Path;

use overtone_core::fs::ProtectedRoots;
use overtone_core::library::SyncRun;
use overtone_core::sync::{self, RekordboxExportPlan, RekordboxExportReport, SeratoSyncReport};
use overtone_sync_serato::SeratoSyncPlan;
use tauri::State;

use crate::error::CommandError;
use crate::ipc::RunningApps;
use crate::state::AppState;

pub fn running_apps() -> RunningApps {
    RunningApps {
        serato: overtone_sync_serato::serato_running(),
        rekordbox: overtone_sync_rekordbox::rekordbox_running(),
        // TODO: detect Traktor once its sync lands.
        traktor: false,
    }
}

pub fn plan_serato(
    state: &AppState,
    serato_dir: Option<&Path>,
) -> Result<Vec<SeratoSyncPlan>, CommandError> {
    state.with_library(|library| sync::plan_serato(library, serato_dir))
}

/// Re-plans so the write matches the library now, then applies.
pub fn apply_serato(
    state: &AppState,
    serato_dir: Option<&Path>,
    roots: &ProtectedRoots,
) -> Result<SeratoSyncReport, CommandError> {
    state.with_library(|library| {
        let plans = sync::plan_serato(library, serato_dir)?;
        sync::apply_serato(library, &plans, roots)
    })
}

pub fn plan_rekordbox(
    state: &AppState,
    path: Option<&Path>,
) -> Result<RekordboxExportPlan, CommandError> {
    state.with_library(|library| sync::plan_rekordbox_export(library, path))
}

pub fn export_rekordbox(
    state: &AppState,
    path: Option<&Path>,
    roots: &ProtectedRoots,
) -> Result<RekordboxExportReport, CommandError> {
    state.with_library(|library| sync::export_rekordbox_xml(library, path, roots))
}

pub fn history(state: &AppState, limit: u32) -> Result<Vec<SyncRun>, CommandError> {
    state.with_library(|library| Ok(library.sync_runs(limit)?))
}

#[tauri::command(async)]
pub fn dj_apps_running() -> RunningApps {
    running_apps()
}

#[tauri::command(async)]
pub fn plan_serato_sync(
    state: State<'_, AppState>,
    serato_dir: Option<String>,
) -> Result<Vec<SeratoSyncPlan>, CommandError> {
    plan_serato(&state, serato_dir.as_deref().map(Path::new))
}

#[tauri::command(async)]
pub fn apply_serato_sync(
    state: State<'_, AppState>,
    serato_dir: Option<String>,
) -> Result<SeratoSyncReport, CommandError> {
    apply_serato(
        &state,
        serato_dir.as_deref().map(Path::new),
        ProtectedRoots::global(),
    )
}

/// `path` defaults to `rekordbox.xml` in the app data folder.
#[tauri::command(async)]
pub fn plan_rekordbox_export(
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<RekordboxExportPlan, CommandError> {
    plan_rekordbox(&state, path.as_deref().map(Path::new))
}

#[tauri::command(async)]
pub fn export_rekordbox_xml(
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<RekordboxExportReport, CommandError> {
    export_rekordbox(
        &state,
        path.as_deref().map(Path::new),
        ProtectedRoots::global(),
    )
}

#[tauri::command]
pub fn sync_history(state: State<'_, AppState>, limit: u32) -> Result<Vec<SyncRun>, CommandError> {
    history(&state, limit)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::commands::library::{tests::scanned_state, tracks};
    use crate::commands::vibes;

    #[test]
    fn plans_a_serato_sync_without_writing_and_history_starts_empty() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 1);
        let id = tracks(&state).expect("tracks")[0].id;
        let vibe = vibes::create(&state, "Dubs").expect("vibe");
        vibes::assign(&state, &[id], vibe.id).expect("assign");

        let serato_dir = dir.path().join("_Serato_");
        let plans = plan_serato(&state, Some(&serato_dir)).expect("plan");
        assert_eq!(plans.len(), 1);
        assert!(!serato_dir.exists(), "planning writes nothing");
        assert!(history(&state, 10).expect("history").is_empty());
    }

    #[test]
    fn plans_and_exports_rekordbox_xml_to_an_explicit_path() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 1);
        let id = tracks(&state).expect("tracks")[0].id;
        let vibe = vibes::create(&state, "Dubs").expect("vibe");
        vibes::assign(&state, &[id], vibe.id).expect("assign");

        let xml = dir.path().join("rekordbox.xml");
        let plan = plan_rekordbox(&state, Some(&xml)).expect("plan");
        assert_eq!((plan.track_count, plan.playlist_count), (1, 1));
        assert!(plan.changed && !plan.exists);
        assert!(!xml.exists(), "planning writes nothing");

        let report = export_rekordbox(&state, Some(&xml), &ProtectedRoots::none()).expect("export");
        assert!(report.changed && report.backup_path.is_none());
        assert!(xml.exists());
        assert!(!plan_rekordbox(&state, Some(&xml)).expect("again").changed);
        assert_eq!(history(&state, 10).expect("history").len(), 1);
    }
}
