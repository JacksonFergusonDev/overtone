use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::UnixMillis;

/// Stable database identifier for a vibe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(transparent)]
pub struct VibeId(pub i64);

/// Separator between levels of a nested vibe, as typed and displayed.
pub const VIBE_SEPARATOR: char = '/';

/// Longest accepted vibe path, in characters.
pub const MAX_VIBE_PATH_CHARS: usize = 200;

/// Why a vibe path was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VibePathError {
    #[error("vibe name is empty")]
    Empty,
    #[error("vibe path has an empty level")]
    EmptySegment,
    #[error("vibe path is longer than {MAX_VIBE_PATH_CHARS} characters")]
    TooLong,
    #[error("vibe names can't contain {0:?}")]
    ForbiddenText(String),
}

/// A validated, possibly nested vibe name such as `Dubs/Deep`.
///
/// Levels are separated by `/`, trimmed, and must not contain text that
/// would break the places vibes are written: `%%` (Serato's subcrate
/// separator in filenames), `#` (hashtag boundaries in Comment tags),
/// `:` and `\` (path separators), or control characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub struct VibePath {
    segments: Vec<String>,
}

impl VibePath {
    pub fn parse(input: &str) -> Result<Self, VibePathError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(VibePathError::Empty);
        }
        if trimmed.chars().count() > MAX_VIBE_PATH_CHARS {
            return Err(VibePathError::TooLong);
        }
        let segments = trimmed
            .split(VIBE_SEPARATOR)
            .map(|segment| {
                let segment = collapse_whitespace(segment);
                if segment.is_empty() {
                    return Err(VibePathError::EmptySegment);
                }
                if let Some(bad) = forbidden_text(&segment) {
                    return Err(VibePathError::ForbiddenText(bad));
                }
                Ok(segment)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { segments })
    }

    /// Levels from outermost to innermost, for example `["Dubs", "Deep"]`.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// The innermost level's name.
    pub fn name(&self) -> &str {
        self.segments.last().map_or("", String::as_str)
    }

    pub fn depth(&self) -> usize {
        self.segments.len()
    }

    pub fn parent(&self) -> Option<Self> {
        (self.segments.len() > 1).then(|| Self {
            segments: self.segments[..self.segments.len() - 1].to_vec(),
        })
    }

    /// Every enclosing vibe, outermost first. `Dubs/Deep/Late` gives `Dubs`, `Dubs/Deep`.
    pub fn ancestors(&self) -> Vec<Self> {
        (1..self.segments.len())
            .map(|len| Self {
                segments: self.segments[..len].to_vec(),
            })
            .collect()
    }

    /// Whether `self` encloses `other` (strictly).
    pub fn is_ancestor_of(&self, other: &Self) -> bool {
        other.segments.len() > self.segments.len()
            && self
                .segments
                .iter()
                .zip(&other.segments)
                .all(|(a, b)| slug_segment(a) == slug_segment(b))
    }

    /// Normalized identity used for uniqueness and Comment hashtags:
    /// lowercase, whitespace replaced by `-`, levels joined with `/`.
    /// `Dubs/Deep Cuts` becomes `dubs/deep-cuts`.
    pub fn slug(&self) -> String {
        self.segments
            .iter()
            .map(|segment| slug_segment(segment))
            .collect::<Vec<_>>()
            .join("/")
    }

    /// The token written into Comment tags, for example `#dubs/deep-cuts`.
    pub fn hashtag(&self) -> String {
        format!("#{}", self.slug())
    }

    /// Returns a new path with `child` nested inside this one.
    pub fn join(&self, child: &str) -> Result<Self, VibePathError> {
        Self::parse(&format!("{self}{VIBE_SEPARATOR}{child}"))
    }
}

impl fmt::Display for VibePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.segments.join("/"))
    }
}

impl TryFrom<String> for VibePath {
    type Error = VibePathError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<VibePath> for String {
    fn from(value: VibePath) -> Self {
        value.to_string()
    }
}

impl std::str::FromStr for VibePath {
    type Err = VibePathError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// A vibe grouping. Tracks can belong to any number of vibes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct Vibe {
    pub id: VibeId,
    pub path: VibePath,
    pub created_at: UnixMillis,
    /// Tracks assigned directly to this vibe (not counting nested vibes).
    pub track_count: u64,
}

fn collapse_whitespace(segment: &str) -> String {
    segment.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn slug_segment(segment: &str) -> String {
    segment
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

fn forbidden_text(segment: &str) -> Option<String> {
    if segment.contains("%%") {
        return Some("%%".to_string());
    }
    segment
        .chars()
        .find(|c| matches!(c, '#' | ':' | '\\') || c.is_control())
        .map(|c| c.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(input: &str) -> VibePath {
        VibePath::parse(input).expect("valid vibe path")
    }

    #[test]
    fn parses_and_normalizes_whitespace() {
        let vibe = path("  Dubs /  Deep   Cuts ");
        assert_eq!(vibe.segments(), ["Dubs", "Deep Cuts"]);
        assert_eq!(vibe.to_string(), "Dubs/Deep Cuts");
        assert_eq!(vibe.name(), "Deep Cuts");
        assert_eq!(vibe.depth(), 2);
    }

    #[test]
    fn slugs_and_hashtags() {
        let vibe = path("Dubs/Deep Cuts");
        assert_eq!(vibe.slug(), "dubs/deep-cuts");
        assert_eq!(vibe.hashtag(), "#dubs/deep-cuts");
        assert_eq!(path("Größe").slug(), "größe");
    }

    #[test]
    fn rejects_invalid_paths() {
        assert_eq!(VibePath::parse("   "), Err(VibePathError::Empty));
        assert_eq!(
            VibePath::parse("Dubs//Deep"),
            Err(VibePathError::EmptySegment)
        );
        assert_eq!(VibePath::parse("Dubs/"), Err(VibePathError::EmptySegment));
        assert!(matches!(
            VibePath::parse("a%%b"),
            Err(VibePathError::ForbiddenText(_))
        ));
        assert!(matches!(
            VibePath::parse("#warmup"),
            Err(VibePathError::ForbiddenText(_))
        ));
        assert!(matches!(
            VibePath::parse("a:b"),
            Err(VibePathError::ForbiddenText(_))
        ));
        assert!(matches!(
            VibePath::parse("a\tb\u{7}"),
            Err(VibePathError::ForbiddenText(_))
        ));
        assert_eq!(
            VibePath::parse(&"x".repeat(MAX_VIBE_PATH_CHARS + 1)),
            Err(VibePathError::TooLong)
        );
    }

    #[test]
    fn hierarchy_helpers() {
        let leaf = path("Dubs/Deep/Late");
        assert_eq!(leaf.parent(), Some(path("Dubs/Deep")));
        assert_eq!(leaf.ancestors(), vec![path("Dubs"), path("Dubs/Deep")]);
        assert!(path("dubs").is_ancestor_of(&leaf));
        assert!(!leaf.is_ancestor_of(&leaf));
        assert!(!path("Dub").is_ancestor_of(&path("Dubs/Deep")));
        assert_eq!(path("Dubs").parent(), None);
        assert_eq!(path("Dubs").join("Deep").expect("join"), path("Dubs/Deep"));
    }

    #[test]
    fn serializes_as_display_string() {
        let json = serde_json::to_string(&path("Dubs/Deep")).expect("serialize");
        assert_eq!(json, "\"Dubs/Deep\"");
        let back: VibePath = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, path("Dubs/Deep"));
        assert!(serde_json::from_str::<VibePath>("\"a//b\"").is_err());
    }
}
