//! Tag and stream property reading, and safe Comment writes.
//!
//! - [`read_metadata`]: fast scan read with lofty's concrete file types
//!   (no artwork, best-attempt parsing).
//! - [`vibes`]: pure hashtag logic for mirroring vibes into Comment.
//! - [`write_comment`]: the only tag write Overtone performs, guarded by
//!   [`raw`]'s independent byte-level walkers. See
//!   `engineering/decisions/tag-writing.md`.

mod meta;
pub mod raw;
mod read;
mod verify;
pub mod vibes;
mod write;

use std::path::Path;

use overtone_model::{AudioProperties, TrackTags};

use crate::AudioError;

pub use read::parse_bpm;
pub use vibes::{apply_vibes_to_comment, hashtags_in};
pub use write::write_comment;

/// Version of the metadata reader. Bump it when [`read_metadata`] starts
/// reading more or reading better, so the next scan rereads every file.
pub const READER_VERSION: u32 = 1;

/// Everything the scanner needs from a file's headers and tags.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileMetadata {
    pub properties: AudioProperties,
    pub tags: TrackTags,
}

/// What [`write_comment`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// The Comment already had the requested value; the file wasn't touched.
    Unchanged,
    /// The verified copy replaced the original.
    Written(WriteReport),
}

/// Details of a verified write, for diagnostics and spikes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WriteReport {
    /// Text frames lofty re-encoded: different bytes, identical decoded
    /// text. Binary frames (GEOB, PRIV, APIC) are never in this list; a
    /// change to them aborts the write.
    pub reencoded_frames: Vec<String>,
    /// Whether frames, chunks, or FLAC blocks changed order.
    pub reordered: bool,
    /// Size of the tag region (ID3v2 tag, or FLAC metadata) before and after.
    pub tag_bytes_before: u64,
    pub tag_bytes_after: u64,
    /// Anything unusual about how the write was done.
    pub notes: Vec<String>,
}

/// Reads stream properties and tags from `path` without decoding audio or
/// loading artwork.
///
/// Damaged tags fall back step by step (relaxed parsing, then, for MP3,
/// the ID3v2 tag alone without trailing ID3v1/APE tags, then stream
/// properties without tags) so one broken frame doesn't hide a track.
pub fn read_metadata(path: &Path) -> Result<FileMetadata, AudioError> {
    read::read_tolerant(path)
}

/// Checks whether `write_comment` would accept `path`, without writing:
/// supported format, a structure the walkers fully account for, and tags
/// that parse strictly. Returns the reason it would refuse.
pub fn check_writable(path: &Path) -> Result<(), AudioError> {
    write::prepare(path).map(|_| ())
}
