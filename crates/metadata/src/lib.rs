//! Metadata lookup: search MusicBrainz, Discogs, and AcoustID for a track
//! and turn the best match into suggestions.
//!
//! Lookups only ever produce [`NewSuggestion`]s. They never write tags or
//! change a track; the DJ reviews every suggestion (see
//! `overtone_library::metadata`). Providers only fill factual fields
//! (label, catalog number, release date, ISRC, album, track number, and the
//! store's genre as `store_genre`), never Comment, Grouping, or Genre.

pub mod acoustid;
pub mod discogs;
mod http;
pub mod matching;
pub mod musicbrainz;

use overtone_model::{MetadataField, NewSuggestion};
use thiserror::Error;

pub use http::{Http, HttpResponse, UreqHttp};

#[derive(Debug, Error)]
pub enum LookupError {
    #[error("couldn't reach {provider}: {message}")]
    Network {
        provider: &'static str,
        message: String,
    },
    #[error("{provider} answered {status}: {message}")]
    Status {
        provider: &'static str,
        status: u16,
        message: String,
    },
    #[error("{provider} sent a response Overtone couldn't read: {message}")]
    BadResponse {
        provider: &'static str,
        message: String,
    },
    #[error("{provider} needs {what}")]
    NotConfigured {
        provider: &'static str,
        what: &'static str,
    },
}

/// What Overtone knows about a track when it searches.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    pub title: String,
    pub artist: String,
    pub duration_secs: Option<u64>,
}

impl Query {
    pub fn artists(&self) -> Vec<String> {
        matching::split_artists(&self.artist)
    }
}

/// One search result from a provider.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub provider: &'static str,
    /// The provider's record ID, such as a MusicBrainz recording MBID.
    pub provider_ref: String,
    pub title: String,
    pub artists: Vec<String>,
    pub duration_secs: Option<u64>,
    /// Field values the candidate carries.
    pub fields: Vec<(MetadataField, String)>,
    /// A score the provider itself gives (AcoustID's fingerprint score),
    /// used instead of title and artist matching.
    pub provider_score: Option<f64>,
    /// The highest confidence this candidate can get, for providers whose
    /// results are less specific (Discogs returns releases, not tracks).
    pub max_confidence: f64,
    /// Tie-breaker between equally good matches; lower is better (an
    /// original single or album before a compilation).
    pub preference: u8,
}

/// A candidate that matched, with its confidence (0 to 1).
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub candidate: Candidate,
    pub confidence: f64,
}

impl Match {
    /// The match's fields as suggestions for the library.
    pub fn suggestions(&self) -> Vec<NewSuggestion> {
        self.candidate
            .fields
            .iter()
            .filter(|(field, value)| field.accepts_lookups() && !value.trim().is_empty())
            .map(|(field, value)| NewSuggestion {
                field: *field,
                value: value.trim().to_string(),
                provider: self.candidate.provider.to_string(),
                provider_ref: Some(self.candidate.provider_ref.clone()),
                confidence: Some(self.confidence),
            })
            .collect()
    }
}

/// Picks the best candidate following OneTagger's rules: the duration must
/// be within 30 s, an artist must match, and the title must match exactly
/// after cleaning or fuzzily at `strictness` or better. Candidates with a
/// provider score (fingerprint matches) skip the title and artist checks.
pub fn best_match(query: &Query, candidates: Vec<Candidate>, strictness: f64) -> Option<Match> {
    let artists = query.artists();
    let mut best: Option<Match> = None;
    for candidate in candidates {
        if !matching::match_duration(query.duration_secs, candidate.duration_secs) {
            continue;
        }
        let confidence = if let Some(score) = candidate.provider_score {
            score
        } else {
            if !artists.is_empty()
                && !matching::match_artist(&artists, &candidate.artists, strictness)
            {
                continue;
            }
            match matching::title_score(&query.title, &candidate.title, strictness) {
                Some(score) => score,
                None => continue,
            }
        };
        let confidence = confidence.min(candidate.max_confidence);
        let better = best.as_ref().is_none_or(|b| {
            let gap = |c: &Candidate| {
                query
                    .duration_secs
                    .zip(c.duration_secs)
                    .map_or(u64::MAX, |(a, b)| a.abs_diff(b))
            };
            // Same confidence: the closest duration wins (the DJ's exact
            // version, extended or radio edit), counting gaps within 3 s as
            // equal, then the more original release.
            let key = |c: &Candidate| (gap(c) / 3, c.preference);
            confidence > b.confidence + 1e-9
                || ((confidence - b.confidence).abs() <= 1e-9
                    && key(&candidate) < key(&b.candidate))
        });
        if better {
            best = Some(Match {
                candidate,
                confidence,
            });
        }
    }
    best
}

/// A User-Agent that names Overtone and, when the DJ sets one, a contact
/// (MusicBrainz asks clients for one). No personal data is sent otherwise.
pub fn user_agent(contact: Option<&str>) -> String {
    let base = format!("Overtone/{}", env!("CARGO_PKG_VERSION"));
    match contact.map(str::trim).filter(|c| !c.is_empty()) {
        Some(contact) => format!("{base} ( {contact} )"),
        None => base,
    }
}

/// Percent-encodes a query parameter value.
pub(crate) fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(title: &str, artist: &str, secs: u64) -> Candidate {
        Candidate {
            provider: "test",
            provider_ref: title.into(),
            title: title.into(),
            artists: vec![artist.into()],
            duration_secs: Some(secs),
            fields: vec![
                (MetadataField::Label, "Hospital".into()),
                (MetadataField::Comment, "never".into()),
            ],
            provider_score: None,
            max_confidence: 1.0,
            preference: 0,
        }
    }

    #[test]
    fn picks_the_matching_candidate_and_never_suggests_comment() {
        let query = Query {
            title: "Hold On (Extended Mix)".into(),
            artist: "Mph".into(),
            duration_secs: Some(300),
        };
        let found = best_match(
            &query,
            vec![
                candidate("Hold On", "Someone Else", 300),
                candidate("Hold On", "MPH", 900),
                candidate("Hold On (Extended Mix)", "MPH", 305),
            ],
            matching::DEFAULT_STRICTNESS,
        )
        .expect("match");
        assert_eq!(found.candidate.duration_secs, Some(305));
        assert!((found.confidence - 1.0).abs() < f64::EPSILON);
        let suggestions = found.suggestions();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].field, MetadataField::Label);
    }

    #[test]
    fn equal_matches_prefer_closer_durations_then_originals() {
        let query = Query {
            title: "Hold On".into(),
            artist: "Mph".into(),
            duration_secs: Some(300),
        };
        let mut compilation = candidate("Hold On", "MPH", 300);
        compilation.preference = 2;
        compilation.provider_ref = "compilation".into();
        let mut single_far = candidate("Hold On", "MPH", 320);
        single_far.provider_ref = "single-far".into();
        let mut single_near = candidate("Hold On", "MPH", 301);
        single_near.provider_ref = "single-near".into();
        let found = best_match(
            &query,
            vec![single_far, compilation, single_near],
            matching::DEFAULT_STRICTNESS,
        )
        .expect("match");
        assert_eq!(found.candidate.provider_ref, "single-near");
    }

    #[test]
    fn user_agent_has_no_contact_unless_set() {
        assert!(!user_agent(None).contains('('));
        assert!(user_agent(Some("dj@example.com")).ends_with("( dj@example.com )"));
    }

    #[test]
    fn encodes_query_values() {
        assert_eq!(encode("a b&c/é"), "a%20b%26c%2F%C3%A9");
    }
}
