//! Shared high-level workflows for the Overtone desktop app and CLI.

use overtone_audio::AudioError;
use overtone_library::{Library, LibraryError, LibrarySummary};
use overtone_sync_rekordbox::RekordboxError;
use overtone_sync_serato::SeratoError;
use overtone_sync_traktor::TraktorError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Unified error type across core Overtone workflows.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error(transparent)]
    Library(#[from] LibraryError),
    #[error(transparent)]
    Serato(#[from] SeratoError),
    #[error(transparent)]
    Rekordbox(#[from] RekordboxError),
    #[error(transparent)]
    Traktor(#[from] TraktorError),
}

/// Diagnostic snapshot returned by both the CLI and desktop shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppStatus {
    pub version: String,
    pub library: LibrarySummary,
}

/// Returns current status for an open library.
pub fn status(library: &Library) -> Result<AppStatus, CoreError> {
    Ok(AppStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        library: library.summary()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_status_for_empty_library() {
        let library = Library::open_in_memory().expect("open library");
        let report = status(&library).expect("status");
        assert_eq!(report.library.track_count, 0);
    }
}
