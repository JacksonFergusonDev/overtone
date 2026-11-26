use std::path::PathBuf;

use thiserror::Error;

/// Errors from guarded file operations.
#[derive(Debug, Error)]
pub enum FsError {
    #[error("refusing to write {path}: it is inside protected folder {root}")]
    Protected { path: PathBuf, root: PathBuf },
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl FsError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
