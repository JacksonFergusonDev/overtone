//! Runs ffmpeg as a child process for the jobs Rust crates don't cover:
//! decoding formats Symphonia can't (HE-AAC, Opus, WMA, damaged files),
//! integrity checks, and encoding (FLAC, AAC through Apple's AudioToolbox,
//! MP3 through LAME).
//!
//! A child process rather than linked bindings: a crash on one bad file
//! only kills that process, there is no C toolchain or linking in the
//! build, and ffmpeg can be updated without rebuilding Overtone. See
//! `engineering/decisions/ffmpeg.md`.
//!
//! Overtone never trusts ffmpeg's output blindly: callers verify decoded
//! or encoded audio with Symphonia, which shares no code with ffmpeg.

mod decode;
mod locate;
mod run;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use thiserror::Error;

pub use decode::{DecodeOptions, PcmReader};
pub use locate::{ENV_VAR, MIN_MAJOR_VERSION, Origin};
pub use run::{IntegrityReport, Job, JobOutcome};

#[derive(Debug, Error)]
pub enum FfmpegError {
    #[error(
        "ffmpeg wasn't found. Install it (for example `brew install ffmpeg`) or set {ENV_VAR} to its path"
    )]
    NotFound,
    #[error("ffmpeg at {path} is version {found}; Overtone needs {MIN_MAJOR_VERSION} or newer")]
    TooOld { path: PathBuf, found: String },
    #[error("couldn't run ffmpeg at {path}: {source}")]
    Spawn {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("ffmpeg failed on {input}: {message}")]
    Failed { input: PathBuf, message: String },
    #[error("ffmpeg's output for {input} couldn't be read: {message}")]
    BadOutput { input: PathBuf, message: String },
    #[error("cancelled")]
    Cancelled,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A located, version-checked ffmpeg binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ffmpeg {
    path: PathBuf,
    version: String,
    origin: Origin,
}

impl Ffmpeg {
    /// Finds ffmpeg: the `OVERTONE_FFMPEG` variable, then the copy bundled
    /// next to the running executable, then `PATH`, then the Homebrew
    /// folders (apps opened from Finder don't inherit the shell's `PATH`).
    pub fn locate() -> Result<Self, FfmpegError> {
        locate::locate()
    }

    /// Uses the ffmpeg at `path`, after checking its version.
    pub fn at(path: &Path) -> Result<Self, FfmpegError> {
        locate::probe(path, Origin::Explicit)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The version string ffmpeg reports, such as `9.1` or `n7.1.1`.
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// A command with the flags every Overtone run uses: no stdin, no
    /// banner, errors only.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.path);
        command
            .args(["-nostdin", "-hide_banner", "-loglevel", "error"])
            .stdin(Stdio::null());
        command
    }

    /// Whether this build has the named encoder (for example `aac_at`,
    /// `libmp3lame`, or `flac`).
    pub fn has_encoder(&self, name: &str) -> Result<bool, FfmpegError> {
        self.lists_codec("-encoders", name)
    }

    /// Whether this build has the named decoder.
    pub fn has_decoder(&self, name: &str) -> Result<bool, FfmpegError> {
        self.lists_codec("-decoders", name)
    }

    /// Whether this build includes the SoX resampler (`--enable-libsoxr`).
    pub fn has_soxr(&self) -> Result<bool, FfmpegError> {
        let output = self
            .command()
            .arg("-buildconf")
            .output()
            .map_err(|source| self.spawn_error(source))?;
        Ok(String::from_utf8_lossy(&output.stdout).contains("--enable-libsoxr"))
    }

    fn lists_codec(&self, flag: &str, name: &str) -> Result<bool, FfmpegError> {
        let output = self
            .command()
            .arg(flag)
            .output()
            .map_err(|source| self.spawn_error(source))?;
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text
            .lines()
            .filter_map(|line| line.split_whitespace().nth(1))
            .any(|codec| codec == name))
    }

    fn spawn_error(&self, source: std::io::Error) -> FfmpegError {
        FfmpegError::Spawn {
            path: self.path.clone(),
            source,
        }
    }

    /// Decodes the first audio stream of `input` to 32-bit float PCM,
    /// streamed from the child process.
    pub fn decode(&self, input: &Path, options: &DecodeOptions) -> Result<PcmReader, FfmpegError> {
        decode::start(self, input, options)
    }

    /// Decodes the whole first audio stream and reports every error ffmpeg
    /// finds (bad frame CRCs, broken bitstreams, truncation).
    pub fn check_integrity(&self, input: &Path) -> Result<IntegrityReport, FfmpegError> {
        run::check_integrity(self, input, None)
    }

    /// Like [`Self::check_integrity`], but only `length` of audio starting
    /// at `start`. Used to tell damage inside the music from junk at the
    /// very start or end of a file.
    pub fn check_integrity_window(
        &self,
        input: &Path,
        start: std::time::Duration,
        length: std::time::Duration,
    ) -> Result<IntegrityReport, FfmpegError> {
        run::check_integrity(self, input, Some((start, length)))
    }

    /// Prepares a job (an encode or remux) with progress reporting.
    pub fn job<I, S>(&self, input: &Path, args: I) -> Job
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Job::new(self, input, args)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::Ffmpeg;

    /// The local ffmpeg, or `None` (and a note) when it isn't installed, so
    /// machines without ffmpeg skip these tests rather than fail them.
    pub fn ffmpeg() -> Option<Ffmpeg> {
        match Ffmpeg::locate() {
            Ok(ffmpeg) => Some(ffmpeg),
            Err(err) => {
                eprintln!("skipping: {err}");
                None
            }
        }
    }
}
