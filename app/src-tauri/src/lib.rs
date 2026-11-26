//! Tauri desktop shell for Overtone.
//!
//! Keep library, audio analysis, tagging, and DJ sync logic in `crates/`;
//! this crate exposes thin Tauri commands and window management.

// Tauri passes command arguments such as `State` by value.
#![allow(clippy::needless_pass_by_value)]

mod error;

use std::path::PathBuf;
use std::sync::Mutex;

use overtone_core::library::Library;
use overtone_core::{AppStatus, CoreError};
use tauri::Manager;

pub use error::CommandError;

/// The open library, shared by every command. `Err` holds why it couldn't be
/// opened, so the UI can explain instead of the app failing to start.
pub struct AppState {
    library: Result<Mutex<Library>, CommandError>,
    library_path: PathBuf,
}

impl AppState {
    pub fn open(explicit: Option<&std::path::Path>) -> Self {
        let library_path = overtone_core::library_path(explicit).unwrap_or_default();
        let library = overtone_core::open_library(explicit)
            .map(|(library, _)| Mutex::new(library))
            .map_err(|err| CommandError::from(&err));
        Self {
            library,
            library_path,
        }
    }

    /// Runs `f` with the library locked.
    pub fn with_library<T>(
        &self,
        f: impl FnOnce(&mut Library) -> Result<T, CoreError>,
    ) -> Result<T, CommandError> {
        let mutex = self.library.as_ref().map_err(Clone::clone)?;
        let mut library = mutex
            .lock()
            .map_err(|_| CommandError::internal("library lock poisoned"))?;
        f(&mut library).map_err(|err| CommandError::from(&err))
    }

    pub fn status(&self) -> Result<AppStatus, CommandError> {
        self.with_library(|library| overtone_core::status(library, &self.library_path))
    }
}

#[tauri::command]
fn app_status(state: tauri::State<'_, AppState>) -> Result<AppStatus, CommandError> {
    state.status()
}

/// Starts the Tauri application.
///
/// # Panics
///
/// Panics if Tauri fails to build or run the desktop application.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(AppState::open(None));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![app_status])
        .run(tauri::generate_context!())
        .expect("error while running Overtone");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reads_the_opened_library() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("lib.sqlite");
        let state = AppState::open(Some(&path));
        let status = state.status().expect("status");
        assert_eq!(status.library.track_count, 0);
        assert_eq!(status.library_path, path.display().to_string());
    }

    #[test]
    fn unopenable_library_reports_an_error_instead_of_panicking() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, b"").expect("seed");
        let state = AppState::open(Some(&blocker.join("lib.sqlite")));
        let err = state.status().expect_err("should fail");
        assert_eq!(err.code, "library");
    }
}
