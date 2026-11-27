//! Tauri commands, one module per area. Each command is a thin
//! `#[tauri::command]` wrapper over a plain function that takes `&AppState`
//! or paths, so the logic is testable without a webview.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use overtone_core::CoreError;
use overtone_core::library::LibraryError;
use overtone_core::model::VibePath;

use crate::error::CommandError;

pub mod library;
pub mod quality;
pub mod sync;
pub mod tags;
pub mod vibes;

/// Parses a vibe path typed by the DJ, mapping failures to `invalid_vibe`.
pub(crate) fn parse_vibe(input: &str) -> Result<VibePath, CommandError> {
    parse_vibe_core(input).map_err(CommandError::from)
}

/// [`parse_vibe`] for code that works in `CoreError`.
pub(crate) fn parse_vibe_core(input: &str) -> Result<VibePath, CoreError> {
    VibePath::parse(input).map_err(|err| CoreError::Library(LibraryError::InvalidVibe(err)))
}

/// Joins a background task, turning a panic or cancellation into an error.
pub(crate) fn join_error(err: &tauri::Error) -> CommandError {
    CommandError::internal(format!("background task failed: {err}"))
}

/// Limits how often chatty progress events reach the webview. Events whose
/// `key` differs from the previous one (a new phase) and final events always
/// pass.
pub(crate) struct Throttle {
    interval: Duration,
    last: Mutex<Option<(Instant, u8)>>,
}

impl Throttle {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: Mutex::new(None),
        }
    }

    pub fn allow(&self, key: u8, is_final: bool) -> bool {
        let Ok(mut last) = self.last.lock() else {
            return true;
        };
        let now = Instant::now();
        let pass = is_final
            || last.is_none_or(|(at, prev)| prev != key || now.duration_since(at) >= self.interval);
        if pass {
            *last = Some((now, key));
        }
        pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_drops_bursts_but_keeps_phase_changes_and_the_last_event() {
        let throttle = Throttle::new(Duration::from_secs(60));
        assert!(throttle.allow(0, false), "first event passes");
        assert!(!throttle.allow(0, false), "burst is dropped");
        assert!(throttle.allow(1, false), "new phase passes");
        assert!(!throttle.allow(1, false));
        assert!(throttle.allow(1, true), "final event always passes");
    }

    #[test]
    fn bad_vibe_paths_report_invalid_vibe() {
        assert_eq!(
            parse_vibe("a%%b").expect_err("invalid").code,
            "invalid_vibe"
        );
        assert!(parse_vibe("Dubs/Deep").is_ok());
    }
}
