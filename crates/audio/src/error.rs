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
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),
    /// The file opened, but its codec can't be decoded (for example HE-AAC).
    #[error("can't decode {path}: {reason}")]
    UnsupportedCodec { path: PathBuf, reason: String },
    /// The file is damaged or not valid audio.
    #[error("couldn't decode {path}: {message}")]
    Decode { path: PathBuf, message: String },
    /// The decoder or analyzer panicked on this file. The panic was caught so
    /// a batch can carry on with the next track.
    #[error("analysis crashed on {path}: {message}")]
    Panicked { path: PathBuf, message: String },
    /// Writing an output such as a spectrogram image failed.
    #[error(transparent)]
    Write(#[from] FsError),
    #[error("couldn't encode image {path}: {message}")]
    Image { path: PathBuf, message: String },
}
