//! Tag and stream property reading, and safe Comment writes. Owned by the
//! tags work package.

use std::path::Path;

use overtone_model::{AudioProperties, TrackTags};

use crate::AudioError;

/// Everything the scanner needs from a file's headers and tags.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileMetadata {
    pub properties: AudioProperties,
    pub tags: TrackTags,
}

/// Reads stream properties and tags from `path` without decoding audio or
/// loading artwork.
pub fn read_metadata(path: &Path) -> Result<FileMetadata, AudioError> {
    let _ = path;
    Err(AudioError::NotImplemented("tags::read_metadata"))
}
