//! Tauri desktop shell for Overtone.
//!
//! Keep library, audio analysis, tagging, and DJ sync logic in `crates/`;
//! this crate exposes thin Tauri commands and window management.

use overtone_core::AppStatus;
use overtone_library::Library;

#[tauri::command]
fn app_status() -> Result<AppStatus, String> {
    let library = Library::open_in_memory().map_err(|err| err.to_string())?;
    overtone_core::status(&library).map_err(|err| err.to_string())
}

/// Starts the Tauri application.
///
/// # Panics
///
/// Panics if Tauri fails to build or run the desktop application.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![app_status])
        .run(tauri::generate_context!())
        .expect("error while running Overtone");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_status_command_returns_valid_status() {
        let status = app_status().expect("app_status");
        assert_eq!(status.library.track_count, 0);
    }
}
