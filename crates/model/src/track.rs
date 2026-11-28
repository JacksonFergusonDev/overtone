use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{QualityReport, UnixMillis, VibeId};

/// Stable database identifier for a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(transparent)]
pub struct TrackId(pub i64);

/// Audio codec family of a file, as decoded rather than as named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    Flac,
    Wav,
    Aiff,
    /// Apple Lossless in an MP4 container.
    Alac,
    Mp3,
    /// AAC in an MP4 container.
    Aac,
    Other,
}

impl AudioFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Flac => "flac",
            Self::Wav => "wav",
            Self::Aiff => "aiff",
            Self::Alac => "alac",
            Self::Mp3 => "mp3",
            Self::Aac => "aac",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "flac" => Self::Flac,
            "wav" => Self::Wav,
            "aiff" => Self::Aiff,
            "alac" => Self::Alac,
            "mp3" => Self::Mp3,
            "aac" => Self::Aac,
            "other" => Self::Other,
            _ => return None,
        })
    }

    /// Whether the container claims lossless audio.
    pub fn is_lossless(self) -> bool {
        matches!(self, Self::Flac | Self::Wav | Self::Aiff | Self::Alac)
    }

    /// Guesses the format from a file extension, before decoding confirms it.
    /// `.m4a` returns `None` because it can hold either AAC or ALAC.
    pub fn from_extension(extension: &str) -> Option<Self> {
        Some(match extension.to_ascii_lowercase().as_str() {
            "flac" => Self::Flac,
            "wav" | "wave" => Self::Wav,
            "aif" | "aiff" | "aifc" => Self::Aiff,
            "mp3" => Self::Mp3,
            "aac" => Self::Aac,
            _ => return None,
        })
    }
}

/// File extensions Overtone scans as audio.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "flac", "wav", "wave", "aif", "aiff", "aifc", "mp3", "m4a", "aac", "alac", "mp4",
];

/// Whether `extension` (without the dot, any case) is one Overtone scans.
pub fn is_audio_extension(extension: &str) -> bool {
    AUDIO_EXTENSIONS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(extension))
}

/// Technical properties read from the file's stream headers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct AudioProperties {
    pub format: Option<AudioFormat>,
    pub sample_rate_hz: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub bitrate_kbps: Option<u32>,
    pub duration_ms: Option<u64>,
}

/// Embedded metadata read from the file's tags.
#[derive(Debug, Clone, PartialEq, Default, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct TrackTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub remixer: Option<String>,
    pub label: Option<String>,
    pub genre: Option<String>,
    pub year: Option<i32>,
    pub bpm: Option<f64>,
    /// Musical key as written in the tag (for example `8A` or `Am`).
    pub key: Option<String>,
    pub comment: Option<String>,
    pub grouping: Option<String>,
    /// The label's catalog number, such as `HOSP123`.
    #[serde(default)]
    pub catalog_number: Option<String>,
    #[serde(default)]
    pub isrc: Option<String>,
    /// Release date as written in the tag (`2021`, `2021-06`, or `2021-06-04`).
    #[serde(default)]
    pub release_date: Option<String>,
    /// Track number as written, such as `3` or `3/12`.
    #[serde(default)]
    pub track_number: Option<String>,
    /// Disc number as written, such as `1` or `1/2`.
    #[serde(default)]
    pub disc_number: Option<String>,
}

/// A track in the library: one immutable file on disk.
#[derive(Debug, Clone, PartialEq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct Track {
    pub id: TrackId,
    /// Absolute path. Immutable after ingest; see `engineering/decisions/architecture-and-ui.md`.
    pub path: String,
    pub file_size: u64,
    pub modified_at: UnixMillis,
    /// Hash of the audio content only, so tag edits keep it stable.
    pub audio_hash: Option<String>,
    pub properties: AudioProperties,
    pub tags: TrackTags,
    pub vibes: Vec<VibeId>,
    pub quality: Option<QualityReport>,
    pub added_at: UnixMillis,
    /// Set when a rescan no longer finds the file. The record and its vibes are kept.
    pub missing_since: Option<UnixMillis>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_strings_roundtrip() {
        for format in [
            AudioFormat::Flac,
            AudioFormat::Wav,
            AudioFormat::Aiff,
            AudioFormat::Alac,
            AudioFormat::Mp3,
            AudioFormat::Aac,
            AudioFormat::Other,
        ] {
            assert_eq!(AudioFormat::parse(format.as_str()), Some(format));
        }
    }

    #[test]
    fn recognizes_audio_extensions_case_insensitively() {
        assert!(is_audio_extension("FLAC"));
        assert!(is_audio_extension("m4a"));
        assert!(!is_audio_extension("jpg"));
        assert!(!is_audio_extension("nfo"));
        assert_eq!(AudioFormat::from_extension("AIF"), Some(AudioFormat::Aiff));
        assert_eq!(AudioFormat::from_extension("m4a"), None);
    }

    #[test]
    fn track_id_serializes_as_number() {
        assert_eq!(
            serde_json::to_string(&TrackId(42)).expect("serialize"),
            "42"
        );
    }
}
