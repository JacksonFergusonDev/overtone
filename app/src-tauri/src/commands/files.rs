//! Integrity checks, export copies, and lossless conversion. All run in
//! the background job slot (one at a time; `cancel_audit` stops them).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use overtone_core::CoreError;
use overtone_core::convert::{self, ConvertOptions, ConvertProgress, ConvertReport};
use overtone_core::export::{self, ExportOptions, ExportProgress, ExportReport, ExportSettings};
use overtone_core::integrity::{self, IntegrityOptions, IntegrityProgress, IntegrityRunReport};
use overtone_core::library::Library;
use overtone_core::model::{AudioFormat, TrackId};
use tauri::State;
use tauri::ipc::Channel;

use super::join_error;
use crate::error::CommandError;
use crate::state::AppState;

pub fn run_integrity(
    library_path: &Path,
    track_ids: Option<Vec<TrackId>>,
    cancel: Arc<AtomicBool>,
    on_progress: &(dyn Fn(IntegrityProgress) + Sync),
) -> Result<IntegrityRunReport, CommandError> {
    let mut library = Library::open(library_path).map_err(CoreError::from)?;
    let ids = match track_ids {
        Some(ids) => ids,
        None => library
            .tracks_needing_integrity_check()
            .map_err(CoreError::from)?,
    };
    let options = IntegrityOptions {
        cancel: Some(cancel),
        ..IntegrityOptions::default()
    };
    Ok(integrity::check_tracks(
        &mut library,
        &ids,
        &options,
        on_progress,
    )?)
}

/// WAV and AIFF tracks that are present, for "convert everything".
fn convertible_tracks(library: &Library) -> Result<Vec<TrackId>, CommandError> {
    Ok(library
        .tracks()?
        .into_iter()
        .filter(|t| {
            t.missing_since.is_none()
                && matches!(
                    t.properties.format,
                    Some(AudioFormat::Wav | AudioFormat::Aiff)
                )
        })
        .map(|t| t.id)
        .collect())
}

pub fn run_convert(
    library_path: &Path,
    track_ids: Option<Vec<TrackId>>,
    write: bool,
    cancel: Option<Arc<AtomicBool>>,
    on_progress: &(dyn Fn(ConvertProgress) + Sync),
) -> Result<ConvertReport, CommandError> {
    let library = Library::open(library_path).map_err(CoreError::from)?;
    let ids = match track_ids {
        Some(ids) => ids,
        None => convertible_tracks(&library)?,
    };
    Ok(convert::convert_tracks(
        &library,
        &ids,
        &ConvertOptions {
            write,
            threads: 0,
            cancel,
        },
        on_progress,
    )?)
}

#[tauri::command]
pub async fn check_integrity(
    state: State<'_, AppState>,
    track_ids: Option<Vec<TrackId>>,
    on_progress: Channel<IntegrityProgress>,
) -> Result<IntegrityRunReport, CommandError> {
    let slot = state.start_audit()?;
    let library_path = state.library_path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = slot.cancel_flag();
        run_integrity(&library_path, track_ids, cancel, &|progress| {
            let _ = on_progress.send(progress);
        })
    })
    .await
    .map_err(|err| join_error(&err))?
}

#[tauri::command]
pub async fn export_tracks(
    state: State<'_, AppState>,
    track_ids: Vec<TrackId>,
    folder: String,
    settings: ExportSettings,
    on_progress: Channel<ExportProgress>,
) -> Result<ExportReport, CommandError> {
    let slot = state.start_audit()?;
    let library_path = state.library_path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let library = Library::open(&library_path).map_err(CoreError::from)?;
        let options = ExportOptions {
            threads: 0,
            cancel: Some(slot.cancel_flag()),
        };
        Ok(export::export_tracks(
            &library,
            &track_ids,
            &PathBuf::from(folder),
            &settings,
            &options,
            &|progress| {
                let _ = on_progress.send(progress);
            },
        )?)
    })
    .await
    .map_err(|err| join_error(&err))?
}

/// Tracks whose latest integrity check found errors inside the music.
#[tauri::command(async)]
pub fn damaged_tracks(state: State<'_, AppState>) -> Result<Vec<TrackId>, CommandError> {
    state.with_library(|library| {
        library
            .tracks_with_integrity_errors()
            .map_err(CoreError::from)
    })
}

/// Previews converting the given tracks (or every WAV and AIFF track).
#[tauri::command]
pub async fn plan_conversion(
    state: State<'_, AppState>,
    track_ids: Option<Vec<TrackId>>,
) -> Result<ConvertReport, CommandError> {
    let library_path = state.library_path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        run_convert(&library_path, track_ids, false, None, &|_| {})
    })
    .await
    .map_err(|err| join_error(&err))?
}

#[tauri::command]
pub async fn convert_tracks(
    state: State<'_, AppState>,
    track_ids: Option<Vec<TrackId>>,
    on_progress: Channel<ConvertProgress>,
) -> Result<ConvertReport, CommandError> {
    let slot = state.start_audit()?;
    let library_path = state.library_path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = slot.cancel_flag();
        run_convert(&library_path, track_ids, true, Some(cancel), &|progress| {
            let _ = on_progress.send(progress);
        })
    })
    .await
    .map_err(|err| join_error(&err))?
}

#[tauri::command]
pub fn discard_conversion(
    state: State<'_, AppState>,
    track_id: TrackId,
) -> Result<bool, CommandError> {
    state.with_library(|library| convert::discard_conversion(library, track_id))
}
