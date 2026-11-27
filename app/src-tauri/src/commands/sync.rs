//! Syncing vibes to DJ software.

use std::path::Path;

use overtone_core::fs::ProtectedRoots;
use overtone_core::library::SyncRun;
use overtone_core::sync::{self, SeratoSyncReport};
use overtone_sync_serato::SeratoSyncPlan;
use tauri::State;

use crate::error::CommandError;
use crate::ipc::RunningApps;
use crate::state::AppState;

pub fn running_apps() -> RunningApps {
    RunningApps {
        serato: overtone_sync_serato::serato_running(),
        // TODO: detect Rekordbox and Traktor once their sync lands.
        rekordbox: false,
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
}
