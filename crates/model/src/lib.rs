//! Shared domain types for Overtone.
//!
//! These types cross every boundary in the workspace: the SQLite library
//! stores them, audio analysis produces them, sync crates consume them, and
//! the desktop app and CLI serialize them. Keep this crate free of heavy
//! dependencies so every other crate can depend on it.

mod conversion;
mod integrity;
mod metadata;
mod quality;
mod track;
mod vibe;

pub use conversion::{Conversion, ConversionStatus};
pub use integrity::IntegrityCheck;
pub use metadata::{
    FieldSource, FieldValue, MetadataField, MetadataSuggestion, NewSuggestion, SuggestionId,
    SuggestionStatus,
};
pub use quality::{LoudnessReport, QualityReport, QualityVerdict, ReviewStatus};
pub use track::{
    AUDIO_EXTENSIONS, AudioFormat, AudioProperties, Track, TrackId, TrackTags, is_audio_extension,
};
pub use vibe::{MAX_VIBE_PATH_CHARS, VIBE_SEPARATOR, Vibe, VibeId, VibePath, VibePathError};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Milliseconds since the Unix epoch, UTC.
pub type UnixMillis = i64;

/// DJ applications Overtone syncs with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DjApp {
    Serato,
    Rekordbox,
    Traktor,
}

impl DjApp {
    pub const ALL: [Self; 3] = [Self::Serato, Self::Rekordbox, Self::Traktor];

    /// Human-readable product name.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Serato => "Serato DJ",
            Self::Rekordbox => "Rekordbox",
            Self::Traktor => "Traktor Pro",
        }
    }
}
