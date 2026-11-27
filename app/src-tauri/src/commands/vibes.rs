//! Vibe tree editing and track assignment.

use overtone_core::model::{TrackId, Vibe, VibeId};
use tauri::State;

use super::parse_vibe;
use crate::error::CommandError;
use crate::state::AppState;

pub fn vibes(state: &AppState) -> Result<Vec<Vibe>, CommandError> {
    state.with_library(|library| Ok(library.vibes()?))
}

pub fn create(state: &AppState, path: &str) -> Result<Vibe, CommandError> {
    let path = parse_vibe(path)?;
    state.with_library(|library| Ok(library.create_vibe(&path)?))
}

pub fn rename(state: &AppState, id: VibeId, new_path: &str) -> Result<Vibe, CommandError> {
    let new_path = parse_vibe(new_path)?;
    state.with_library(|library| Ok(library.rename_vibe(id, &new_path)?))
}

pub fn delete(state: &AppState, id: VibeId) -> Result<(), CommandError> {
    state.with_library(|library| Ok(library.delete_vibe(id)?))
}

pub fn assign(state: &AppState, tracks: &[TrackId], id: VibeId) -> Result<(), CommandError> {
    state.with_library(|library| Ok(library.assign_vibe(tracks, id)?))
}

pub fn unassign(state: &AppState, tracks: &[TrackId], id: VibeId) -> Result<(), CommandError> {
    state.with_library(|library| Ok(library.unassign_vibe(tracks, id)?))
}

#[tauri::command]
pub fn list_vibes(state: State<'_, AppState>) -> Result<Vec<Vibe>, CommandError> {
    vibes(&state)
}

#[tauri::command]
pub fn create_vibe(state: State<'_, AppState>, path: String) -> Result<Vibe, CommandError> {
    create(&state, &path)
}

#[tauri::command]
pub fn rename_vibe(
    state: State<'_, AppState>,
    vibe_id: VibeId,
    new_path: String,
) -> Result<Vibe, CommandError> {
    rename(&state, vibe_id, &new_path)
}

#[tauri::command]
pub fn delete_vibe(state: State<'_, AppState>, vibe_id: VibeId) -> Result<(), CommandError> {
    delete(&state, vibe_id)
}

#[tauri::command(async)]
pub fn assign_vibe(
    state: State<'_, AppState>,
    track_ids: Vec<TrackId>,
    vibe_id: VibeId,
) -> Result<(), CommandError> {
    assign(&state, &track_ids, vibe_id)
}

#[tauri::command(async)]
pub fn unassign_vibe(
    state: State<'_, AppState>,
    track_ids: Vec<TrackId>,
    vibe_id: VibeId,
) -> Result<(), CommandError> {
    unassign(&state, &track_ids, vibe_id)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::commands::library::{tests::scanned_state, tracks};

    #[test]
    fn vibe_crud_and_assignment() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 2);
        let ids: Vec<TrackId> = tracks(&state)
            .expect("tracks")
            .iter()
            .map(|t| t.id)
            .collect();

        let deep = create(&state, "Dubs/Deep").expect("create");
        assert_eq!(deep.path.to_string(), "Dubs/Deep");
        assert_eq!(
            create(&state, "Dubs/Deep").expect("again").id,
            deep.id,
            "creating an existing vibe returns it"
        );
        assert_eq!(
            create(&state, "bad%%name").expect_err("bad").code,
            "invalid_vibe"
        );

        assign(&state, &ids, deep.id).expect("assign");
        let all = vibes(&state).expect("vibes");
        assert_eq!(
            all.iter()
                .find(|v| v.id == deep.id)
                .expect("deep")
                .track_count,
            2
        );
        assert!(
            tracks(&state)
                .expect("tracks")
                .iter()
                .all(|t| t.vibes == vec![deep.id])
        );

        unassign(&state, &ids[..1], deep.id).expect("unassign");
        let t = tracks(&state).expect("tracks");
        assert_eq!(t.iter().filter(|t| t.vibes.contains(&deep.id)).count(), 1);

        let renamed = rename(&state, deep.id, "Rollers/Deep").expect("rename");
        assert_eq!(renamed.path.to_string(), "Rollers/Deep");
        assert_eq!(
            rename(&state, VibeId(999), "X").expect_err("unknown").code,
            "not_found"
        );

        delete(&state, renamed.id).expect("delete");
        assert!(
            vibes(&state)
                .expect("vibes")
                .iter()
                .all(|v| v.id != renamed.id)
        );
        assert_eq!(
            tracks(&state).expect("tracks").len(),
            2,
            "tracks are untouched"
        );
    }
}
