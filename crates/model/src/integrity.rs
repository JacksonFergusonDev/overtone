use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{TrackId, UnixMillis};

/// The result of decoding a whole file with ffmpeg and listing every error
/// it found (bad frame checksums, broken bitstreams, truncation). A second
/// opinion beside the spectral verdict; it never changes the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct IntegrityCheck {
    pub track_id: TrackId,
    /// No errors and the decode reached the end.
    pub clean: bool,
    /// Every error sits in the first half second or the last few seconds,
    /// where junk bytes and leftover tags live (common in MP3s and
    /// harmless to playback). The music itself decoded cleanly.
    pub edge_only: bool,
    /// What ffmpeg reported, one line each (at most [`Self::MAX_ERRORS`]).
    pub errors: Vec<String>,
    /// How many error lines there were in all.
    pub error_count: u32,
    /// The file's modification time when it was checked, so a changed file
    /// is checked again.
    pub file_modified_at: UnixMillis,
    pub ffmpeg_version: String,
    pub checked_at: UnixMillis,
}

impl IntegrityCheck {
    /// Error lines kept per track.
    pub const MAX_ERRORS: usize = 20;

    /// Whether the errors reach into the music, so the DJ should listen.
    pub fn is_damaged(&self) -> bool {
        !self.clean && !self.edge_only
    }
}
