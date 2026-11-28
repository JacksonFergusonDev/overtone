//! Finding ffmpeg and checking its version.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{Ffmpeg, FfmpegError};

/// Environment variable that points Overtone at a specific ffmpeg.
pub const ENV_VAR: &str = "OVERTONE_FFMPEG";

/// Oldest ffmpeg major version Overtone accepts. 6.0 (2023) is the first
/// with the channel-layout API and `-progress` fields Overtone relies on.
pub const MIN_MAJOR_VERSION: u32 = 6;

/// Where the ffmpeg in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Passed to [`Ffmpeg::at`].
    Explicit,
    /// The `OVERTONE_FFMPEG` environment variable.
    Environment,
    /// Bundled next to the running executable (the desktop app's sidecar).
    Bundled,
    /// Found on `PATH` or in a Homebrew folder.
    System,
}

const FALLBACK_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

pub(crate) fn locate() -> Result<Ffmpeg, FfmpegError> {
    if let Some(path) = std::env::var_os(ENV_VAR).filter(|v| !v.is_empty()) {
        return probe(Path::new(&path), Origin::Environment);
    }
    if let Some(bundled) = bundled_path() {
        return probe(&bundled, Origin::Bundled);
    }
    let path_dirs = std::env::var_os("PATH").map_or_else(Vec::new, |value| {
        std::env::split_paths(&value).collect::<Vec<_>>()
    });
    path_dirs
        .into_iter()
        .chain(FALLBACK_DIRS.iter().map(PathBuf::from))
        .map(|dir| dir.join("ffmpeg"))
        .find(|candidate| is_executable(candidate))
        .map_or(Err(FfmpegError::NotFound), |found| {
            probe(&found, Origin::System)
        })
}

fn bundled_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join("ffmpeg");
    is_executable(&candidate).then_some(candidate)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub(crate) fn probe(path: &Path, origin: Origin) -> Result<Ffmpeg, FfmpegError> {
    let output = Command::new(path)
        .args(["-hide_banner", "-version"])
        .stdin(Stdio::null())
        .output()
        .map_err(|source| FfmpegError::Spawn {
            path: path.to_path_buf(),
            source,
        })?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = parse_version(&text).ok_or_else(|| FfmpegError::BadOutput {
        input: path.to_path_buf(),
        message: "`-version` printed no version".into(),
    })?;
    if major_version(&version).is_some_and(|major| major < MIN_MAJOR_VERSION) {
        return Err(FfmpegError::TooOld {
            path: path.to_path_buf(),
            found: version,
        });
    }
    Ok(Ffmpeg {
        path: path.to_path_buf(),
        version,
        origin,
    })
}

/// The version word from `ffmpeg version 9.1 Copyright ...`.
fn parse_version(text: &str) -> Option<String> {
    text.lines()
        .next()?
        .strip_prefix("ffmpeg version ")?
        .split_whitespace()
        .next()
        .map(str::to_string)
}

/// The major version of a release (`9.1`, `n7.1.1`). Git snapshots such as
/// `N-117000-g...` have none and are accepted.
fn major_version(version: &str) -> Option<u32> {
    let digits: String = version
        .trim_start_matches('n')
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_and_snapshot_versions() {
        assert_eq!(
            parse_version("ffmpeg version 9.1 Copyright (c) 2000-2026").as_deref(),
            Some("9.1")
        );
        assert_eq!(major_version("9.1"), Some(9));
        assert_eq!(major_version("n7.1.1"), Some(7));
        assert_eq!(major_version("N-117000-gabc"), None);
        assert_eq!(parse_version("something else"), None);
    }

    #[test]
    fn explicit_path_that_does_not_exist_fails_cleanly() {
        assert!(matches!(
            Ffmpeg::at(Path::new("/nonexistent/ffmpeg")),
            Err(FfmpegError::Spawn { .. })
        ));
    }

    #[test]
    fn locates_the_local_ffmpeg_when_installed() {
        if let Some(ffmpeg) = crate::testing::ffmpeg() {
            assert!(major_version(ffmpeg.version()).is_none_or(|m| m >= MIN_MAJOR_VERSION));
            assert!(ffmpeg.has_encoder("flac").expect("encoders"));
            assert!(!ffmpeg.has_encoder("no_such_codec").expect("encoders"));
        }
    }
}
