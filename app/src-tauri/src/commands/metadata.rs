//! Field sources, lookup suggestions, lookups, and lookup credentials.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use overtone_core::CoreError;
use overtone_core::library::{Library, LibraryError};
use overtone_core::lookup::{
    self, LookupConfig, LookupOptions, LookupProgress, LookupReport, SETTING_ACOUSTID_KEY,
    SETTING_CONTACT, SETTING_DISCOGS_TOKEN,
};
use overtone_core::model::{
    FieldValue, MetadataField, MetadataSuggestion, SuggestionId, SuggestionStatus, TrackId,
};
use tauri::State;
use tauri::ipc::Channel;

use super::join_error;
use crate::error::CommandError;
use crate::ipc::{LookupSettings, TrackDetails};
use crate::state::AppState;

pub fn details(library: &Library, track_id: TrackId) -> Result<TrackDetails, CoreError> {
    Ok(TrackDetails {
        track_id,
        fields: library.field_values(track_id)?,
        suggestions: library.suggestions(Some(track_id), Some(SuggestionStatus::Pending))?,
        loudness: library.loudness(track_id)?,
        integrity: library.integrity_check(track_id)?,
        conversion: library.pending_conversion(track_id)?,
    })
}

pub fn settings(library: &Library) -> Result<LookupSettings, CoreError> {
    let config = LookupConfig::load(library)?;
    Ok(LookupSettings {
        acoustid_key_set: config.acoustid_key.is_some(),
        discogs_token_set: config.discogs_token.is_some(),
        contact: library
            .setting::<String>(SETTING_CONTACT)?
            .filter(|c| !c.trim().is_empty()),
        providers: config.available(),
    })
}

/// Saves the given credentials. `None` leaves a value as it is; an empty
/// string removes it.
pub fn save_settings(
    library: &Library,
    acoustid_key: Option<String>,
    discogs_token: Option<String>,
    contact: Option<String>,
) -> Result<LookupSettings, CoreError> {
    for (key, value) in [
        (SETTING_ACOUSTID_KEY, acoustid_key),
        (SETTING_DISCOGS_TOKEN, discogs_token),
        (SETTING_CONTACT, contact),
    ] {
        if let Some(value) = value {
            library.set_setting(key, &value.trim().to_string())?;
        }
    }
    settings(library)
}

pub fn run_lookup(
    library_path: &Path,
    track_ids: Option<Vec<TrackId>>,
    cancel: Arc<AtomicBool>,
    on_progress: &dyn Fn(LookupProgress),
) -> Result<LookupReport, CommandError> {
    let mut library = Library::open(library_path).map_err(CoreError::from)?;
    let config = LookupConfig::load(&library)?;
    let ids = match track_ids {
        Some(ids) => ids,
        None => lookup::tracks_needing_lookup(&library, &config.available())?,
    };
    Ok(lookup::lookup_tracks(
        &mut library,
        &ids,
        &LookupOptions {
            providers: Vec::new(),
            cancel: Some(cancel),
        },
        on_progress,
    )?)
}

fn parse_field(name: &str) -> Result<MetadataField, CommandError> {
    MetadataField::parse(name).ok_or_else(|| {
        CommandError::from(LibraryError::NotFoundByName {
            kind: "metadata field",
            name: name.to_string(),
        })
    })
}

#[tauri::command]
pub fn track_details(
    state: State<'_, AppState>,
    track_id: TrackId,
) -> Result<TrackDetails, CommandError> {
    state.with_library(|library| details(library, track_id))
}

#[tauri::command]
pub fn set_track_field(
    state: State<'_, AppState>,
    track_id: TrackId,
    field: String,
    value: Option<String>,
) -> Result<FieldValue, CommandError> {
    let field = parse_field(&field)?;
    state.with_library(|library| Ok(library.set_user_field(track_id, field, value.as_deref())?))
}

#[tauri::command]
pub fn reset_track_field(
    state: State<'_, AppState>,
    track_id: TrackId,
    field: String,
) -> Result<FieldValue, CommandError> {
    let field = parse_field(&field)?;
    state.with_library(|library| {
        library.clear_field_override(track_id, field)?;
        Ok(library.field_value(track_id, field)?)
    })
}

#[tauri::command]
pub fn list_suggestions(
    state: State<'_, AppState>,
) -> Result<Vec<MetadataSuggestion>, CommandError> {
    state.with_library(|library| Ok(library.suggestions(None, Some(SuggestionStatus::Pending))?))
}

#[tauri::command]
pub fn accept_suggestion(
    state: State<'_, AppState>,
    suggestion_id: SuggestionId,
    replace: bool,
) -> Result<FieldValue, CommandError> {
    state.with_library(|library| Ok(library.accept_suggestion(suggestion_id, replace)?))
}

#[tauri::command]
pub fn reject_suggestion(
    state: State<'_, AppState>,
    suggestion_id: SuggestionId,
) -> Result<(), CommandError> {
    state.with_library(|library| Ok(library.reject_suggestion(suggestion_id)?))
}

#[tauri::command]
pub fn lookup_settings(state: State<'_, AppState>) -> Result<LookupSettings, CommandError> {
    state.with_library(|library| settings(library))
}

#[tauri::command]
pub fn set_lookup_settings(
    state: State<'_, AppState>,
    acoustid_key: Option<String>,
    discogs_token: Option<String>,
    contact: Option<String>,
) -> Result<LookupSettings, CommandError> {
    state.with_library(|library| save_settings(library, acoustid_key, discogs_token, contact))
}

/// Looks up the given tracks, or every track the configured providers
/// haven't searched. Uses the background job slot (one job at a time;
/// `cancel_audit` stops it).
#[tauri::command]
pub async fn lookup_tracks(
    state: State<'_, AppState>,
    track_ids: Option<Vec<TrackId>>,
    on_progress: Channel<LookupProgress>,
) -> Result<LookupReport, CommandError> {
    let slot = state.start_audit()?;
    let library_path = state.library_path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = slot.cancel_flag();
        run_lookup(&library_path, track_ids, cancel, &|progress| {
            let _ = on_progress.send(progress);
        })
    })
    .await
    .map_err(|err| join_error(&err))?
}

#[cfg(test)]
mod tests {
    use overtone_core::library::ScannedTrack;
    use overtone_core::model::{AudioProperties, FieldSource, NewSuggestion, TrackTags};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn details_show_sources_and_pending_suggestions() {
        let dir = TempDir::new().expect("tempdir");
        let mut library = Library::open(&dir.path().join("lib.sqlite")).expect("open");
        let (id, _) = library
            .upsert_scanned(&ScannedTrack {
                path: "/m/a.flac".into(),
                file_size: 1,
                modified_at: 1,
                audio_hash: None,
                properties: AudioProperties::default(),
                tags: TrackTags {
                    title: Some("T".into()),
                    ..TrackTags::default()
                },
            })
            .expect("insert");
        library
            .add_suggestions(
                id,
                &[NewSuggestion {
                    field: MetadataField::Label,
                    value: "Hospital".into(),
                    provider: "musicbrainz".into(),
                    provider_ref: None,
                    confidence: Some(1.0),
                }],
            )
            .expect("suggest");
        let found = details(&library, id).expect("details");
        assert_eq!(found.suggestions.len(), 1);
        let title = found
            .fields
            .iter()
            .find(|f| f.field == MetadataField::Title)
            .expect("title");
        assert_eq!(title.source, FieldSource::File);

        let saved =
            save_settings(&library, Some("key".into()), None, Some(" ".into())).expect("save");
        assert!(saved.acoustid_key_set && !saved.discogs_token_set);
        assert_eq!(saved.contact, None);
    }
}
