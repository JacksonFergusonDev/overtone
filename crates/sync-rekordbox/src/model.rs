//! The slice of the Rekordbox XML format Overtone writes.

use std::path::PathBuf;

use overtone_model::AudioFormat;

/// A whole `rekordbox.xml` document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RekordboxXml {
    /// Version reported in the `PRODUCT` element.
    pub product_version: String,
    pub tracks: Vec<XmlTrack>,
    /// Children of the `ROOT` playlist node.
    pub playlists: Vec<PlaylistNode>,
}

/// One `COLLECTION/TRACK`. Only `track_id` and `location` are required;
/// attributes that are `None` are left out of the file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct XmlTrack {
    pub track_id: u32,
    pub location: PathBuf,
    pub name: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    /// For example `FLAC File`, see [`kind_for`].
    pub kind: Option<String>,
    pub size_bytes: Option<u64>,
    pub total_time_s: Option<u32>,
    pub average_bpm: Option<f64>,
    /// Musical key as written in the tag.
    pub tonality: Option<String>,
    pub bit_rate_kbps: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub comments: Option<String>,
    pub remixer: Option<String>,
    pub label: Option<String>,
    pub year: Option<i32>,
    /// `yyyy-mm-dd`.
    pub date_added: Option<String>,
}

/// A node in the playlist tree.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaylistNode {
    Folder {
        name: String,
        children: Vec<PlaylistNode>,
    },
    Playlist {
        name: String,
        track_ids: Vec<u32>,
    },
}

impl PlaylistNode {
    pub fn name(&self) -> &str {
        match self {
            Self::Folder { name, .. } | Self::Playlist { name, .. } => name,
        }
    }
}

/// Rekordbox's `Kind` string for a format, or `None` when unknown.
pub fn kind_for(format: AudioFormat) -> Option<&'static str> {
    match format {
        AudioFormat::Mp3 => Some("MP3 File"),
        AudioFormat::Flac => Some("FLAC File"),
        AudioFormat::Wav => Some("WAV File"),
        AudioFormat::Aiff => Some("AIFF File"),
        AudioFormat::Alac | AudioFormat::Aac => Some("M4A File"),
        AudioFormat::Other => None,
    }
}

/// `yyyy-mm-dd` (UTC) for seconds since the Unix epoch.
pub fn date_from_unix_secs(secs: i64) -> String {
    // Days-to-civil conversion from Howard Hinnant's date algorithms.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_dates() {
        assert_eq!(date_from_unix_secs(0), "1970-01-01");
        assert_eq!(date_from_unix_secs(951_782_400), "2000-02-29");
        assert_eq!(date_from_unix_secs(1_798_329_600), "2026-12-27");
    }

    #[test]
    fn maps_formats_to_kinds() {
        assert_eq!(kind_for(AudioFormat::Flac), Some("FLAC File"));
        assert_eq!(kind_for(AudioFormat::Alac), Some("M4A File"));
        assert_eq!(kind_for(AudioFormat::Other), None);
    }
}
