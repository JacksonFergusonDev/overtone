//! Traktor `collection.nml` playlist sync for Traktor Pro 3 and 4.
//!
//! Overtone never rewrites `collection.nml` as a whole. [`scan`] reads the
//! file once to collect every collection entry's key and the byte span of
//! the `Overtone` playlist folder directly under `$ROOT`. [`patch`] then
//! copies the original bytes around that span unchanged and puts a freshly
//! rendered folder ([`render_overtone_folder`]) in its place, and [`verify`]
//! checks the result before anything is written. See "Traktor: splice the
//! Overtone playlist folder into collection.nml" in
//! `engineering/decisions/dj-software-sync.md`.

use std::path::PathBuf;

use thiserror::Error;

mod key;
mod locate;
mod patch;
mod render;
mod running;
mod scan;

pub use key::{
    DEFAULT_STARTUP_VOLUME, Location, NML_DIR_SEPARATOR, decode_name, encode_name, encode_nml_dir,
    location_for, nfc, path_from_key, traktor_key, volume_for,
};
pub use locate::{COLLECTION_FILE_NAME, find_collections};
pub use patch::{patch, verify};
pub use render::{PlaylistNode, playlist_uuid, render_overtone_folder};
pub use running::{is_traktor_executable, traktor_running};
pub use scan::{CollectionScan, scan};

/// Name of the top-level playlist folder Overtone owns.
pub const OVERTONE_FOLDER: &str = "Overtone";

/// Errors produced while reading or patching `collection.nml`.
#[derive(Debug, Error)]
pub enum TraktorError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Traktor must be closed before Overtone changes collection.nml")]
    AppRunning,
    #[error("couldn't read collection.nml: {0}")]
    Malformed(String),
    #[error("the patched collection.nml failed verification, so nothing was written: {0}")]
    Verify(String),
    #[error("no Traktor collection.nml found in {0}; choose the file explicitly")]
    NoCollection(PathBuf),
    #[error("found several Traktor collections ({}); choose one explicitly", list(.0))]
    SeveralCollections(Vec<PathBuf>),
    #[error("collection.nml changed while Overtone was syncing it; try again")]
    ChangedDuringSync,
}

fn list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
