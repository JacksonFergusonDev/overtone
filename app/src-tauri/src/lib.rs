//! Tauri desktop shell for Overtone.
//!
//! Keep library, audio analysis, tagging, and DJ sync logic in `crates/`;
//! this crate exposes thin Tauri commands and window management.

// Tauri passes command arguments such as `State` by value.
#![allow(clippy::needless_pass_by_value)]

mod commands;
mod error;
pub mod ipc;
mod state;

use tauri::Manager;

use commands::{library, quality, sync, tags, vibes};
pub use error::CommandError;
pub use state::AppState;

/// Starts the Tauri application.
///
/// # Panics
///
/// Panics if Tauri fails to build or run the desktop application.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let cache = app
                .path()
                .app_cache_dir()
                .map_err(|err| err.to_string())?
                .join("spectrograms");
            let state = AppState::open(None, cache);
            library::allow_startup_directories(app.handle(), &state);
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            library::app_status,
            library::list_tracks,
            library::list_roots,
            library::add_root,
            library::remove_root,
            library::scan_library,
            quality::audit_tracks,
            quality::cancel_audit,
            quality::set_review,
            quality::track_waveform,
            quality::render_spectrogram,
            vibes::list_vibes,
            vibes::create_vibe,
            vibes::rename_vibe,
            vibes::delete_vibe,
            vibes::assign_vibe,
            vibes::unassign_vibe,
            sync::dj_apps_running,
            sync::plan_serato_sync,
            sync::apply_serato_sync,
            sync::plan_rekordbox_export,
            sync::export_rekordbox_xml,
            sync::sync_history,
            tags::plan_tag_mirror,
            tags::apply_tag_mirror,
            tags::restore_comments,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Overtone");
}
