use std::path::PathBuf;

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
}
