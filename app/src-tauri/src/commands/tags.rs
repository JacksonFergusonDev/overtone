//! Mirroring vibes into file Comment tags, and restoring original Comments.

use overtone_core::CoreError;
use overtone_core::fs::ProtectedRoots;
use overtone_core::library::{COMMENT_FIELD, Library, LibraryError};
use overtone_core::model::TrackId;
use overtone_core::tags::{self, CommentChange, MirrorReport};
use tauri::State;

use super::parse_vibe_core;
use crate::error::CommandError;
use crate::state::AppState;

/// Present tracks, optionally only those in `vibe`.
fn mirror_selection(library: &Library, vibe: Option<&str>) -> Result<Vec<TrackId>, CoreError> {
    let mut ids: Vec<TrackId> = library
        .tracks()?
        .into_iter()
        .filter(|track| track.missing_since.is_none())
        .map(|track| track.id)
        .collect();
    if let Some(name) = vibe {
        let path = parse_vibe_core(name)?;
        let found = library.vibe_by_path(&path)?.ok_or_else(|| {
            CoreError::Library(LibraryError::NotFoundByName {
                kind: "vibe",
                name: path.to_string(),
            })
        })?;
        let members = library.tracks_in_vibe(found.id)?;
        ids.retain(|id| members.contains(id));
    }
    Ok(ids)
}

pub fn plan(state: &AppState, vibe: Option<&str>) -> Result<Vec<CommentChange>, CommandError> {
    state.with_library(|library| {
        let ids = mirror_selection(library, vibe)?;
        tags::plan_vibe_mirroring(library, &ids)
    })
}

/// Re-plans so the write matches the files now, then writes.
pub fn apply(
    state: &AppState,
    vibe: Option<&str>,
    roots: &ProtectedRoots,
) -> Result<MirrorReport, CommandError> {
    state.with_library(|library| {
        let ids = mirror_selection(library, vibe)?;
        let changes = tags::plan_vibe_mirroring(library, &ids)?;
        Ok(tags::apply_vibe_mirroring(library, &changes, roots))
    })
}

pub fn restore(state: &AppState, roots: &ProtectedRoots) -> Result<MirrorReport, CommandError> {
    state.with_library(|library| {
        let ids = library.tracks_with_tag_snapshot(COMMENT_FIELD)?;
        Ok(tags::restore_original_comments(library, &ids, roots))
    })
}

#[tauri::command(async)]
pub fn plan_tag_mirror(
    state: State<'_, AppState>,
    vibe: Option<String>,
) -> Result<Vec<CommentChange>, CommandError> {
    plan(&state, vibe.as_deref())
}

#[tauri::command(async)]
pub fn apply_tag_mirror(
    state: State<'_, AppState>,
    vibe: Option<String>,
) -> Result<MirrorReport, CommandError> {
    apply(&state, vibe.as_deref(), ProtectedRoots::global())
}

#[tauri::command(async)]
pub fn restore_comments(state: State<'_, AppState>) -> Result<MirrorReport, CommandError> {
    restore(&state, ProtectedRoots::global())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::commands::library::{tests::scanned_state, tracks};
    use crate::commands::vibes;

    #[test]
    fn mirror_plan_apply_and_restore_on_fixture_files() {
        let dir = TempDir::new().expect("tempdir");
        let state = scanned_state(&dir, 2);
        let ids: Vec<TrackId> = tracks(&state)
            .expect("tracks")
            .iter()
            .map(|t| t.id)
            .collect();
        let deep = vibes::create(&state, "Dubs/Deep").expect("vibe");
        vibes::assign(&state, &ids[..1], deep.id).expect("assign");

        let none = ProtectedRoots::none();
        let planned = plan(&state, None).expect("plan");
        assert_eq!(planned.len(), 1, "only the tagged track changes");
        assert_eq!(planned[0].track_id, ids[0]);
        assert!(
            planned[0]
                .after
                .as_deref()
                .expect("after")
                .contains("#dubs/deep")
        );
        assert_eq!(plan(&state, Some("Dubs/Deep")).expect("scoped").len(), 1);
        assert_eq!(
            plan(&state, Some("Nope")).expect_err("no vibe").code,
            "not_found"
        );
        assert_eq!(
            plan(&state, Some("a%%b")).expect_err("bad").code,
            "invalid_vibe"
        );

        let report = apply(&state, None, &none).expect("apply");
        assert_eq!(report.written, vec![ids[0]]);
        assert!(report.failed.is_empty());
        assert!(plan(&state, None).expect("replan").is_empty());

        let restored = restore(&state, &none).expect("restore");
        assert_eq!(restored.written, vec![ids[0]]);
        assert_eq!(plan(&state, None).expect("replan").len(), 1);
    }
}
