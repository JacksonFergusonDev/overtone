//! Discogs: the best source for electronic labels, catalog numbers, and
//! styles. Searching needs a personal access token
//! (<https://www.discogs.com/settings/developers>).
//! <https://www.discogs.com/developers#page:database,header:database-search>

use overtone_model::MetadataField;
use serde::Deserialize;

use crate::{Candidate, Http, LookupError, Query, encode, matching};

const PROVIDER: &str = "discogs";

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<Result_>,
}

#[derive(Debug, Deserialize)]
struct Result_ {
    id: u64,
    /// "Artist - Release Title"
    title: String,
    year: Option<serde_json::Value>,
    #[serde(default)]
    label: Vec<String>,
    catno: Option<String>,
    #[serde(default)]
    style: Vec<String>,
    #[serde(default)]
    genre: Vec<String>,
}

pub struct Discogs<'a> {
    http: &'a dyn Http,
    token: String,
}

impl<'a> Discogs<'a> {
    pub fn new(http: &'a dyn Http, token: &str) -> Result<Self, LookupError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(LookupError::NotConfigured {
                provider: PROVIDER,
                what: "a personal access token",
            });
        }
        Ok(Self {
            http,
            token: token.to_string(),
        })
    }

    /// Releases containing a track with this title by this artist.
    ///
    /// Discogs returns releases, not tracks, without durations, so a result
    /// is matched on its artist and the queried title, and its confidence is
    /// capped below an exact title match.
    pub fn search(&self, query: &Query) -> Result<Vec<Candidate>, LookupError> {
        let url = format!(
            "https://api.discogs.com/database/search?type=release&track={}&artist={}&per_page=15",
            encode(&matching::clean_title(&query.title)),
            encode(&matching::clean_artist_searching(&query.artist)),
        );
        let headers = [("Authorization", format!("Discogs token={}", self.token))];
        let response = self
            .http
            .get(&url, &headers)
            .map_err(|message| LookupError::Network {
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
        let parsed: SearchResponse =
            serde_json::from_str(&response.body).map_err(|err| LookupError::BadResponse {
                provider: PROVIDER,
                message: err.to_string(),
            })?;
        Ok(parsed
            .results
            .into_iter()
            .map(|r| to_candidate(r, query))
            .collect())
    }
}

/// Discogs credits repeated artist names with a number, as in "Mph (2)".
fn strip_index(name: &str) -> &str {
    match name.rsplit_once(" (") {
        Some((base, rest))
            if rest
                .trim_end_matches(')')
                .chars()
                .all(|c| c.is_ascii_digit()) =>
        {
            base
        }
        _ => name,
    }
}

fn to_candidate(result: Result_, query: &Query) -> Candidate {
    let artist = result
        .title
        .split_once(" - ")
        .map_or(result.title.as_str(), |(artist, _)| artist);
    let artists = matching::split_artists(artist)
        .iter()
        .map(|a| strip_index(a).to_string())
        .collect();
    let mut fields = Vec::new();
    if let Some(label) = result.label.first() {
        fields.push((MetadataField::Label, strip_index(label).to_string()));
    }
    if let Some(catno) = result
        .catno
        .filter(|c| !c.is_empty() && !c.eq_ignore_ascii_case("none"))
    {
        fields.push((MetadataField::CatalogNumber, catno));
    }
    let year = match result.year {
        Some(serde_json::Value::String(y)) => Some(y),
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    if let Some(year) = year.filter(|y| y.len() == 4 && y != "0") {
        fields.push((MetadataField::Year, year));
    }
    let genre = if result.style.is_empty() {
        result.genre
    } else {
        result.style
    };
    if !genre.is_empty() {
        fields.push((MetadataField::StoreGenre, genre.join(", ")));
    }
    Candidate {
        provider: PROVIDER,
        provider_ref: format!("release/{}", result.id),
        // The search was for this track title, so the release holds it.
        title: query.title.clone(),
        artists,
        duration_secs: None,
        fields,
        provider_score: None,
        max_confidence: 0.8,
        preference: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::fake::FakeHttp;

    #[test]
    fn needs_a_token() {
        let http = FakeHttp::new(Vec::new());
        assert!(matches!(
            Discogs::new(&http, "  "),
            Err(LookupError::NotConfigured { .. })
        ));
    }

    #[test]
    fn maps_label_catalog_year_and_styles() {
        let body = r#"{"results":[{"id":123,"type":"release","title":"Mph (2) - Hold On",
            "year":"2023","label":["Mph UK","Some Distributor"],"catno":"MPH001",
            "style":["UK Garage","Speed Garage"],"genre":["Electronic"]}]}"#;
        let http = FakeHttp::new(vec![("database/search", 200, body.into())]);
        let discogs = Discogs::new(&http, "abc").expect("token");
        let query = Query {
            title: "Hold On (Extended Mix)".into(),
            artist: "Mph".into(),
            duration_secs: Some(300),
        };
        let found = crate::best_match(
            &query,
            discogs.search(&query).expect("search"),
            matching::DEFAULT_STRICTNESS,
        )
        .expect("match");
        let fields = &found.candidate.fields;
        assert!(fields.contains(&(MetadataField::Label, "Mph UK".into())));
        assert!(fields.contains(&(MetadataField::CatalogNumber, "MPH001".into())));
        assert!(fields.contains(&(MetadataField::Year, "2023".into())));
        assert!(fields.contains(&(MetadataField::StoreGenre, "UK Garage, Speed Garage".into())));
        assert_eq!(strip_index("Mph (2)"), "Mph");
        assert_eq!(strip_index("Song (Remix)"), "Song (Remix)");
        let url = &http.requests.lock().expect("lock")[0];
        assert!(url.contains("track=hold%20on") && url.contains("artist=mph"));
    }
}
