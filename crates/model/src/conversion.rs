use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{TrackId, UnixMillis};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ConversionStatus {
    /// The FLAC exists beside the original, which is still the track.
    PendingRelocation,
    /// Overtone and the DJ apps now use the FLAC.
    Relocated,
    /// The DJ discarded the FLAC.
    Discarded,
}

impl ConversionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PendingRelocation => "pending_relocation",
            Self::Relocated => "relocated",
            Self::Discarded => "discarded",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending_relocation" => Self::PendingRelocation,
            "relocated" => Self::Relocated,
            "discarded" => Self::Discarded,
            _ => return None,
        })
    }
}

/// A verified lossless conversion of a track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct Conversion {
    pub id: i64,
    pub track_id: TrackId,
    pub output_path: String,
    pub format: String,
    pub status: ConversionStatus,
    /// Source frames with no FLAC equivalent.
    pub dropped: Vec<String>,
    /// Serato data carried over, such as `Serato Markers2`.
    pub serato: Vec<String>,
    /// Audio frames, verified identical to the source.
    pub frames: u64,
    pub created_at: UnixMillis,
}
