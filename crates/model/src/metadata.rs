//! Where each metadata value came from, and suggestions waiting for review.
//!
//! Values the DJ typed must never get lost among values Overtone fetched.
//! Every field of every track has a source: the file as scanned, the DJ, or
//! a lookup provider. Lookups only ever arrive as suggestions; nothing a
//! provider returns becomes a track's value until the DJ accepts it. See
//! `engineering/plans/metadata-and-conversion.md` (step 2).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{TrackId, TrackTags, UnixMillis};

/// A metadata field Overtone tracks provenance for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum MetadataField {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Remixer,
    Label,
    Genre,
    Year,
    Bpm,
    Key,
    Comment,
    Grouping,
    CatalogNumber,
    Isrc,
    ReleaseDate,
    TrackNumber,
    DiscNumber,
    /// The genre a store or database files the release under. Kept apart
    /// from Genre, which is the DJ's own, and only copied there on request.
    StoreGenre,
}

impl MetadataField {
    pub const ALL: [Self; 18] = [
        Self::Title,
        Self::Artist,
        Self::Album,
        Self::AlbumArtist,
        Self::Remixer,
        Self::Label,
        Self::Genre,
        Self::Year,
        Self::Bpm,
        Self::Key,
        Self::Comment,
        Self::Grouping,
        Self::CatalogNumber,
        Self::Isrc,
        Self::ReleaseDate,
        Self::TrackNumber,
        Self::DiscNumber,
        Self::StoreGenre,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::AlbumArtist => "album_artist",
            Self::Remixer => "remixer",
            Self::Label => "label",
            Self::Genre => "genre",
            Self::Year => "year",
            Self::Bpm => "bpm",
            Self::Key => "key",
            Self::Comment => "comment",
            Self::Grouping => "grouping",
            Self::CatalogNumber => "catalog_number",
            Self::Isrc => "isrc",
            Self::ReleaseDate => "release_date",
            Self::TrackNumber => "track_number",
            Self::DiscNumber => "disc_number",
            Self::StoreGenre => "store_genre",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.as_str() == value)
    }

    /// Fields that hold the DJ's own curation. Lookups never propose them,
    /// and Comment is only changed through vibe mirroring.
    pub fn is_personal(self) -> bool {
        matches!(self, Self::Comment | Self::Grouping)
    }

    /// Whether a lookup provider may suggest a value for this field.
    /// Genre is excluded: providers suggest `StoreGenre` instead.
    pub fn accepts_lookups(self) -> bool {
        !self.is_personal() && self != Self::Genre
    }

    /// The field's value as scanned from the file, as text.
    pub fn file_value(self, tags: &TrackTags) -> Option<String> {
        match self {
            Self::Title => tags.title.clone(),
            Self::Artist => tags.artist.clone(),
            Self::Album => tags.album.clone(),
            Self::AlbumArtist => tags.album_artist.clone(),
            Self::Remixer => tags.remixer.clone(),
            Self::Label => tags.label.clone(),
            Self::Genre => tags.genre.clone(),
            Self::Year => tags.year.map(|year| year.to_string()),
            Self::Bpm => tags.bpm.map(format_bpm),
            Self::Key => tags.key.clone(),
            Self::Comment => tags.comment.clone(),
            Self::Grouping => tags.grouping.clone(),
            Self::CatalogNumber
            | Self::Isrc
            | Self::ReleaseDate
            | Self::TrackNumber
            | Self::DiscNumber
            | Self::StoreGenre => None,
        }
    }
}

impl std::fmt::Display for MetadataField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Formats a BPM without trailing zeros (`128`, `127.99`).
fn format_bpm(bpm: f64) -> String {
    let text = format!("{bpm:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Where a field's current value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum FieldSource {
    /// Read from the file when it was scanned.
    File,
    /// Entered by the DJ in Overtone.
    User,
    /// A lookup suggestion the DJ accepted.
    Lookup,
}

impl FieldSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::User => "user",
            Self::Lookup => "lookup",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "file" => Self::File,
            "user" => Self::User,
            "lookup" => Self::Lookup,
            _ => return None,
        })
    }
}

/// A field's current value and where it came from.
#[derive(Debug, Clone, PartialEq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct FieldValue {
    pub field: MetadataField,
    /// `None` when the field is empty (or the DJ cleared it).
    pub value: Option<String>,
    pub source: FieldSource,
    /// For `Lookup`: the provider, such as `musicbrainz`.
    pub provider: Option<String>,
    /// For `Lookup`: the provider's record ID, for tracing the value back.
    pub provider_ref: Option<String>,
    /// When the DJ set or accepted the value. `None` for file values.
    pub set_at: Option<UnixMillis>,
    /// The file's own value, when the current value overrides it.
    pub file_value: Option<String>,
}

/// Stable database identifier for a metadata suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(transparent)]
pub struct SuggestionId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionStatus {
    Pending,
    Accepted,
    Rejected,
}

impl SuggestionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "accepted" => Self::Accepted,
            "rejected" => Self::Rejected,
            _ => return None,
        })
    }
}

/// A value a lookup provider proposes for one field of one track.
#[derive(Debug, Clone, PartialEq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct MetadataSuggestion {
    pub id: SuggestionId,
    pub track_id: TrackId,
    pub field: MetadataField,
    pub value: String,
    pub provider: String,
    pub provider_ref: Option<String>,
    /// The provider match's confidence, 0 to 1.
    pub confidence: Option<f64>,
    pub fetched_at: UnixMillis,
    pub status: SuggestionStatus,
    pub decided_at: Option<UnixMillis>,
    /// The field's current value when this was read. Accepting a suggestion
    /// that would replace a non-empty value needs an explicit choice.
    pub current: FieldValue,
}

impl MetadataSuggestion {
    /// Whether accepting would replace a value that is already there.
    pub fn replaces_value(&self) -> bool {
        self.current
            .value
            .as_deref()
            .is_some_and(|current| !current.trim().is_empty() && current != self.value)
    }
}

/// A suggestion as a provider returns it, before it is stored.
#[derive(Debug, Clone, PartialEq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct NewSuggestion {
    pub field: MetadataField,
    pub value: String,
    pub provider: String,
    pub provider_ref: Option<String>,
    pub confidence: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_names_roundtrip() {
        for field in MetadataField::ALL {
            assert_eq!(MetadataField::parse(field.as_str()), Some(field));
        }
        assert_eq!(MetadataField::parse("nope"), None);
    }

    #[test]
    fn personal_fields_and_genre_refuse_lookups() {
        assert!(!MetadataField::Comment.accepts_lookups());
        assert!(!MetadataField::Grouping.accepts_lookups());
        assert!(!MetadataField::Genre.accepts_lookups());
        assert!(MetadataField::StoreGenre.accepts_lookups());
        assert!(MetadataField::Label.accepts_lookups());
    }

    #[test]
    fn reads_file_values_as_text() {
        let tags = TrackTags {
            bpm: Some(127.99),
            year: Some(2019),
            ..TrackTags::default()
        };
        assert_eq!(
            MetadataField::Bpm.file_value(&tags).as_deref(),
            Some("127.99")
        );
        assert_eq!(
            MetadataField::Year.file_value(&tags).as_deref(),
            Some("2019")
        );
        assert_eq!(format_bpm(128.0), "128");
    }
}
