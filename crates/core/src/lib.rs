//! Shared high-level workflows for the Overtone desktop app and CLI.

use std::path::{Path, PathBuf};

use overtone_audio::AudioError;
use overtone_fs::FsError;
use overtone_library::{Library, LibraryError, LibrarySummary};
use overtone_sync_rekordbox::RekordboxError;
use overtone_sync_serato::SeratoError;
use overtone_sync_traktor::TraktorError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

pub mod scan;
pub mod sync;
pub mod tags;

pub use overtone_fs as fs;
pub use overtone_library as library;
pub use overtone_model as model;

/// Unified error type across core Overtone workflows.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error(transparent)]
    Fs(#[from] FsError),
    #[error(transparent)]
    Library(#[from] LibraryError),
    #[error(transparent)]
    Serato(#[from] SeratoError),
    #[error(transparent)]
    Rekordbox(#[from] RekordboxError),
    #[error(transparent)]
    Traktor(#[from] TraktorError),
    #[error("couldn't find a folder for Overtone's data; set OVERTONE_DATA_DIR")]
    NoDataDir,
    #[error("can't add {path} as a library folder: {reason}")]
    InvalidRoot { path: PathBuf, reason: String },
}

impl CoreError {
    /// Stable, machine-readable error category for the CLI's JSON output and
    /// the desktop app.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Audio(_) => "audio",
            Self::Fs(FsError::Protected { .. })
            | Self::Serato(SeratoError::Fs(FsError::Protected { .. })) => "protected_path",
            Self::Fs(_) => "io",
            Self::Library(LibraryError::NotFound { .. } | LibraryError::NotFoundByName { .. }) => {
                "not_found"
            }
            Self::Library(LibraryError::InvalidVibe(_)) => "invalid_vibe",
            Self::Library(LibraryError::VibeExists(_)) => "vibe_exists",
            Self::Library(LibraryError::SchemaTooNew { .. }) => "schema_too_new",
            Self::Library(_) => "library",
            Self::Serato(SeratoError::SeratoRunning)
            | Self::Rekordbox(RekordboxError::AppRunning)
            | Self::Traktor(TraktorError::AppRunning) => "dj_app_running",
            Self::Serato(_) => "serato",
            Self::Rekordbox(_) => "rekordbox",
            Self::Traktor(_) => "traktor",
            Self::NoDataDir => "no_data_dir",
            Self::InvalidRoot { .. } => "invalid_root",
        }
    }
}

/// Resolves the library database path: `explicit` if given, otherwise the
/// shared default in the app data folder.
pub fn library_path(explicit: Option<&Path>) -> Result<PathBuf, CoreError> {
    match explicit {
        Some(path) => Ok(path.to_path_buf()),
        None => overtone_fs::paths::default_library_path().ok_or(CoreError::NoDataDir),
    }
}

/// Opens (creating if needed) the library at `explicit` or the default path.
pub fn open_library(explicit: Option<&Path>) -> Result<(Library, PathBuf), CoreError> {
    let path = library_path(explicit)?;
    let library = Library::open(&path)?;
    Ok((library, path))
}

/// Diagnostic snapshot returned by both the CLI and desktop shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct AppStatus {
    pub version: String,
    pub library_path: String,
    pub library: LibrarySummary,
}

/// Returns current status for an open library.
pub fn status(library: &Library, library_path: &Path) -> Result<AppStatus, CoreError> {
    Ok(AppStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        library_path: library_path.display().to_string(),
        library: library.summary()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn opens_explicit_library_and_reports_status() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("lib.sqlite");
        let (library, opened) = open_library(Some(&path)).expect("open");
        assert_eq!(opened, path);
        let report = status(&library, &opened).expect("status");
        assert_eq!(report.library.track_count, 0);
        assert_eq!(report.library_path, path.display().to_string());
    }

    #[test]
    fn status_counts_tracks_in_a_temp_library() {
        let mut temp = overtone_testkit::TempLibrary::new();
        let path = temp.add_fixture(
            "tone",
            &overtone_testkit::Signal::cd(16_000.0),
            overtone_testkit::Fixture::Wav { bits: 16 },
        );
        temp.library
            .upsert_scanned(&library::ScannedTrack {
                path: path.display().to_string(),
                file_size: std::fs::metadata(&path).expect("stat").len(),
                ..library::ScannedTrack::default()
            })
            .expect("insert");
        let report = status(&temp.library, &temp.library_path).expect("status");
        assert_eq!(report.library.track_count, 1);
    }

    #[test]
    fn error_codes_are_stable() {
        let err = CoreError::from(LibraryError::VibeExists("Dubs".into()));
        assert_eq!(err.code(), "vibe_exists");
        assert_eq!(CoreError::NoDataDir.code(), "no_data_dir");
        assert_eq!(
            CoreError::from(SeratoError::SeratoRunning).code(),
            "dj_app_running"
        );
    }
}
