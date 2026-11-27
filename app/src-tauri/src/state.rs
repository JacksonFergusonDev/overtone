//! Shared application state: the main library connection plus the flags and
//! paths that long-running jobs need.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use overtone_core::library::Library;
use overtone_core::{AppStatus, CoreError};

use crate::error::CommandError;

/// The open library, shared by every short command. Long jobs open their own
/// connection to `library_path` so they never hold this lock.
pub struct AppState {
    /// `Err` holds why the library couldn't be opened, so the UI can explain
    /// instead of the app failing to start.
    library: Result<Mutex<Library>, CommandError>,
    library_path: PathBuf,
    spectrogram_dir: PathBuf,
    audit_busy: Arc<AtomicBool>,
    audit_cancel: Arc<AtomicBool>,
}

impl AppState {
    /// Opens the library at `explicit` (or the default path). Spectrogram
    /// PNGs are cached in `spectrogram_dir`.
    pub fn open(explicit: Option<&Path>, spectrogram_dir: PathBuf) -> Self {
        let library_path = overtone_core::library_path(explicit).unwrap_or_default();
        let library = overtone_core::open_library(explicit)
            .map(|(library, _)| Mutex::new(library))
            .map_err(|err| CommandError::from(&err));
        Self {
            library,
            library_path,
            spectrogram_dir,
            audit_busy: Arc::default(),
            audit_cancel: Arc::default(),
        }
    }

    pub fn library_path(&self) -> &Path {
        &self.library_path
    }

    pub fn spectrogram_dir(&self) -> &Path {
        &self.spectrogram_dir
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
        f(&mut library).map_err(CommandError::from)
    }

    pub fn status(&self) -> Result<AppStatus, CommandError> {
        self.with_library(|library| overtone_core::status(library, &self.library_path))
    }

    /// Claims the single audit slot, clearing any earlier cancel request.
    /// Fails with code `busy` while another audit runs.
    pub fn start_audit(&self) -> Result<AuditSlot, CommandError> {
        if self
            .audit_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(CommandError::busy("an audit is already running"));
        }
        self.audit_cancel.store(false, Ordering::Release);
        Ok(AuditSlot {
            busy: Arc::clone(&self.audit_busy),
            cancel: Arc::clone(&self.audit_cancel),
        })
    }

    /// Asks the running audit (if any) to stop after the tracks in flight.
    pub fn cancel_audit(&self) {
        self.audit_cancel.store(true, Ordering::Release);
    }
}

/// Held while an audit runs; releases the slot when dropped.
pub struct AuditSlot {
    busy: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

impl AuditSlot {
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }
}

impl Drop for AuditSlot {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::Release);
    }
}

#[cfg(test)]
pub mod tests {
    use tempfile::TempDir;

    use super::*;

    /// A state backed by a library inside `dir`.
    pub fn state_in(dir: &TempDir) -> AppState {
        AppState::open(
            Some(&dir.path().join("lib.sqlite")),
            dir.path().join("spectrograms"),
        )
    }

    #[test]
    fn status_reads_the_opened_library() {
        let dir = TempDir::new().expect("tempdir");
        let state = state_in(&dir);
        let status = state.status().expect("status");
        assert_eq!(status.library.track_count, 0);
        assert_eq!(
            status.library_path,
            dir.path().join("lib.sqlite").display().to_string()
        );
    }

    #[test]
    fn unopenable_library_reports_an_error_instead_of_panicking() {
        let dir = TempDir::new().expect("tempdir");
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, b"").expect("seed");
        let state = AppState::open(Some(&blocker.join("lib.sqlite")), dir.path().join("c"));
        let err = state.status().expect_err("should fail");
        assert_eq!(err.code, "library");
    }

    #[test]
    fn only_one_audit_slot_at_a_time() {
        let dir = TempDir::new().expect("tempdir");
        let state = state_in(&dir);
        let slot = state.start_audit().expect("first");
        assert_eq!(state.start_audit().err().expect("busy").code, "busy");
        state.cancel_audit();
        assert!(slot.cancel_flag().load(Ordering::Acquire));
        drop(slot);
        let again = state.start_audit().expect("slot is free again");
        assert!(
            !again.cancel_flag().load(Ordering::Acquire),
            "a new audit clears the old cancel request"
        );
    }
}
