//! MusicBrainz: free, open data; one request per second.
//! <https://musicbrainz.org/doc/MusicBrainz_API>

use overtone_model::MetadataField;
use serde::Deserialize;

use crate::{Candidate, Http, LookupError, Query, encode, matching};

const PROVIDER: &str = "musicbrainz";
const BASE: &str = "https://musicbrainz.org/ws/2";

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    recordings: Vec<Recording>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Recording {
    id: String,
    title: String,
    length: Option<u64>,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<ArtistCredit>,
    #[serde(default)]
    isrcs: Vec<String>,
    #[serde(default)]
    releases: Vec<Release>,
}

#[derive(Debug, Deserialize)]
struct ArtistCredit {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    id: String,
    title: String,
    date: Option<String>,
    status: Option<String>,
    #[serde(rename = "release-group")]
    group: Option<ReleaseGroup>,
    #[serde(default)]
    media: Vec<Medium>,
}

#[derive(Debug, Deserialize)]
struct ReleaseGroup {
    #[serde(rename = "primary-type")]
    primary_type: Option<String>,
    #[serde(rename = "secondary-types", default)]
    secondary_types: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Medium {
    #[serde(default, alias = "tracks")]
    track: Vec<MediumTrack>,
}

#[derive(Debug, Deserialize)]
struct MediumTrack {
    number: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReleaseLabels {
    #[serde(rename = "label-info", default)]
    label_info: Vec<LabelInfo>,
}

#[derive(Debug, Deserialize)]
struct LabelInfo {
    #[serde(rename = "catalog-number")]
    catalog_number: Option<String>,
    label: Option<Label>,
}

#[derive(Debug, Deserialize)]
struct Label {
    name: String,
}

pub struct MusicBrainz<'a> {
    http: &'a dyn Http,
}

fn get<T: for<'de> Deserialize<'de>>(http: &dyn Http, url: &str) -> Result<T, LookupError> {
    let response = http.get(url, &[]).map_err(|message| LookupError::Network {
        provider: PROVIDER,
        message,
    })?;
    if response.status != 200 {
        return Err(LookupError::Status {
            provider: PROVIDER,
            status: response.status,
            message: response.body.chars().take(200).collect(),
        });
    }
    serde_json::from_str(&response.body).map_err(|err| LookupError::BadResponse {
        provider: PROVIDER,
        message: err.to_string(),
    })
}

/// Escapes Lucene query syntax inside a quoted phrase.
fn phrase(value: &str) -> String {
    let escaped: String = value
        .chars()
        .flat_map(|c| match c {
            '"' | '\\' => vec!['\\', c],
            _ => vec![c],
        })
        .collect();
    format!("\"{escaped}\"")
}

/// How much a release represents the original: official singles, EPs,
/// and albums first, compilations and promos last, then oldest first.
fn release_rank(release: &Release) -> (u8, String) {
    let group = release.group.as_ref();
    let compilation = is_compilation(release);
    let official = release.status.as_deref() == Some("Official");
    let primary = group
        .and_then(|g| g.primary_type.as_deref())
        .is_some_and(|t| matches!(t, "Single" | "EP" | "Album"));
    let rank = u8::from(compilation) * 2 + u8::from(!official) + u8::from(!primary);
    (rank, release.date.clone().unwrap_or_else(|| "9999".into()))
}

impl<'a> MusicBrainz<'a> {
    pub fn new(http: &'a dyn Http) -> Self {
        Self { http }
    }

    /// Recordings matching the title and artist.
    pub fn search(&self, query: &Query) -> Result<Vec<Candidate>, LookupError> {
        let title = matching::clean_title(&query.title);
        let artist = matching::clean_artist_searching(&query.artist);
        let mut lucene = format!("recording:{}", phrase(&title));
        if !artist.is_empty() {
            lucene.push_str(&format!(" AND artist:{}", phrase(&artist)));
        }
        let url = format!(
            "{BASE}/recording?query={}&fmt=json&limit=15",
            encode(&lucene)
        );
        let response: SearchResponse = get(self.http, &url)?;
        Ok(response.recordings.into_iter().map(to_candidate).collect())
    }

    /// One recording by MBID, with its releases and ISRCs.
    pub fn recording(&self, mbid: &str) -> Result<Candidate, LookupError> {
        let url = format!(
            "{BASE}/recording/{}?inc=releases+release-groups+artist-credits+isrcs+media&fmt=json",
            encode(mbid)
        );
        let recording: Recording = get(self.http, &url)?;
        Ok(to_candidate(recording))
    }

    /// Completes the winning match: fetches the full recording (search
    /// results list only some of its releases), picks the original release
    /// from all of them, and adds that release's label and catalog number.
    /// Keeps the match's identity (provider and score).
    pub fn complete(&self, candidate: &mut Candidate) -> Result<(), LookupError> {
        let mbid = candidate
            .provider_ref
            .split('/')
            .next()
            .unwrap_or_default()
            .to_string();
        let full = self.recording(&mbid)?;
        candidate.fields = full.fields;
        candidate.provider_ref = full.provider_ref;
        candidate.preference = full.preference;
        self.add_labels(candidate)
    }

    /// Adds the label and catalog number of the candidate's chosen release
    /// (one more request, so only for the match that won).
    pub fn add_labels(&self, candidate: &mut Candidate) -> Result<(), LookupError> {
        let Some(release) = candidate
            .fields
            .iter()
            .find(|(field, _)| *field == MetadataField::Album)
            .and_then(|_| release_id(candidate))
        else {
            return Ok(());
        };
        let url = format!("{BASE}/release/{}?inc=labels&fmt=json", encode(&release));
        let labels: ReleaseLabels = get(self.http, &url)?;
        if let Some(info) = labels.label_info.first() {
            if let Some(label) = &info.label {
                candidate
                    .fields
                    .push((MetadataField::Label, label.name.clone()));
            }
            if let Some(catno) = info
                .catalog_number
                .as_deref()
                .filter(|c| !c.is_empty() && *c != "[none]")
            {
                candidate
                    .fields
                    .push((MetadataField::CatalogNumber, catno.to_string()));
            }
        }
        Ok(())
    }
}

/// The release MBID stored in a candidate's ref (`recording/release`).
fn release_id(candidate: &Candidate) -> Option<String> {
    candidate
        .provider_ref
        .split_once('/')
        .map(|(_, release)| release.to_string())
}

fn to_candidate(recording: Recording) -> Candidate {
    let mut fields = Vec::new();
    if let Some(isrc) = recording.isrcs.first() {
        fields.push((MetadataField::Isrc, isrc.clone()));
    }
    let release = recording.releases.iter().min_by_key(|r| release_rank(r));
    let mut provider_ref = recording.id.clone();
    let preference = release.map_or(4, |r| release_rank(r).0);
    // A compilation's title, date, and track number describe the
    // compilation, not the track, so only ISRCs are kept from one.
    if let Some(release) = release.filter(|r| !is_compilation(r)) {
        provider_ref = format!("{}/{}", recording.id, release.id);
        fields.push((MetadataField::Album, release.title.clone()));
        if let Some(date) = release.date.as_deref().filter(|d| !d.is_empty()) {
            fields.push((MetadataField::ReleaseDate, date.to_string()));
            if let Some(year) = date
                .get(..4)
                .filter(|y| y.chars().all(|c| c.is_ascii_digit()))
            {
                fields.push((MetadataField::Year, year.to_string()));
            }
        }
        if let Some(number) = release
            .media
            .iter()
            .flat_map(|m| &m.track)
            .find_map(|t| t.number.clone())
        {
            fields.push((MetadataField::TrackNumber, number));
        }
    }
    Candidate {
        provider: PROVIDER,
        provider_ref,
        title: recording.title,
        artists: recording
            .artist_credit
            .into_iter()
            .map(|credit| credit.name)
            .collect(),
        duration_secs: recording.length.map(|ms| ms / 1000),
        fields,
        provider_score: None,
        max_confidence: 1.0,
        preference,
    }
}

/// Compilations and DJ mixes: their title, date, and track number describe
/// the collection, not the track.
fn is_compilation(release: &Release) -> bool {
    release.group.as_ref().is_some_and(|g| {
        g.secondary_types
            .iter()
            .any(|t| matches!(t.as_str(), "Compilation" | "DJ-mix" | "Mixtape/Street"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::fake::FakeHttp;
    use crate::{best_match, matching::DEFAULT_STRICTNESS};

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture")
    }

    #[test]
    fn searches_and_matches_by_title_artist_and_duration() {
        let http = FakeHttp::new(vec![
            ("/recording?query=", 200, fixture("mb_search.json")),
            ("/recording/", 200, fixture("mb_recording.json")),
            ("/release/", 200, fixture("mb_release.json")),
        ]);
        let mb = MusicBrainz::new(&http);
        let query = Query {
            title: "One More Time (Original Mix)".into(),
            artist: "Daft Punk".into(),
            duration_secs: Some(321),
        };
        let candidates = mb.search(&query).expect("search");
        assert!(candidates.len() >= 5);
        let mut found = best_match(&query, candidates, DEFAULT_STRICTNESS).expect("match");
        assert_eq!(found.candidate.duration_secs, Some(320));
        mb.complete(&mut found.candidate).expect("complete");
        let fields: Vec<_> = found.candidate.fields.iter().map(|(f, _)| *f).collect();
        assert!(fields.contains(&MetadataField::Album));
        assert!(fields.contains(&MetadataField::Label));
        assert!(
            !fields.contains(&MetadataField::CatalogNumber),
            "[none] is dropped"
        );

        let requests = http.requests.lock().expect("lock");
        assert!(requests[0].contains("recording%3A%22one%20more%20time%22"));
        assert!(requests[0].contains("artist%3A%22daft%20punk%22"));
    }

    #[test]
    fn prefers_the_original_single_over_compilations() {
        let http = FakeHttp::new(vec![("/recording/", 200, fixture("mb_recording.json"))]);
        let candidate = MusicBrainz::new(&http)
            .recording("1611a62a-bc3b-491f-bc56-75565ca42c38")
            .expect("lookup");
        let album = candidate
            .fields
            .iter()
            .find(|(f, _)| *f == MetadataField::Album)
            .map(|(_, v)| v.as_str());
        assert_eq!(album, Some("One More Time"));
        assert!(
            candidate
                .fields
                .contains(&(MetadataField::ReleaseDate, "2001-02-14".into()))
        );
        assert!(
            candidate
                .fields
                .contains(&(MetadataField::TrackNumber, "1".into()))
        );
    }

    #[test]
    fn reports_server_errors() {
        let http = FakeHttp::new(vec![(
            "/recording?query=",
            503,
            r#"{"error":"busy"}"#.into(),
        )]);
        let err = MusicBrainz::new(&http)
            .search(&Query {
                title: "x".into(),
                ..Query::default()
            })
            .expect_err("503");
        assert!(matches!(err, LookupError::Status { status: 503, .. }));
    }
}
