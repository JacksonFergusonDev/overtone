use std::path::PathBuf;

use overtone_fs::FsError;
use thiserror::Error;

/// Errors produced during audio inspection, analysis, or tag operations.
#[derive(Debug, Error)]
pub enum AudioError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("unsupported audio format: {0}")]
    UnsupportedFormat(String),
    /// The container was recognized but its structure is truncated or
    /// inconsistent.
    #[error("malformed {container} file at {path}: {reason}")]
    Malformed {
        path: PathBuf,
        container: &'static str,
        reason: String,
    },
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),
    /// The tag library couldn't parse the file's tags or headers.
    #[error("couldn't read tags from {path}: {message}")]
    TagRead { path: PathBuf, message: String },
    /// The file's structure is one Overtone can't rewrite without risking
    /// data it doesn't model. Nothing was written.
    #[error("refusing to write tags to {path}: {reason}")]
    WriteRefused { path: PathBuf, reason: String },
    /// The edited copy failed a safety check, so it was discarded and the
    /// original file was left untouched.
    #[error("tag write to {path} discarded, original untouched: {check} check failed: {detail}")]
    VerificationFailed {
        path: PathBuf,
        check: &'static str,
        detail: String,
    },
    #[error(transparent)]
    Fs(#[from] FsError),
}

impl AudioError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
