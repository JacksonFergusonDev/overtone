//! Metadata lookup across the library: identify each track with AcoustID
//! (when configured), MusicBrainz, and Discogs (when configured), and store
//! what the best match proposes as suggestions for the DJ to review.
//!
//! Lookups never change a track. They read titles, artists, and durations
//! (the DJ's own values first, see `overtone_library::metadata`), send them
//! and, for AcoustID, an audio fingerprint to the providers, and add
//! pending suggestions. Files are only read.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use overtone_audio::analysis::fingerprint;
use overtone_library::Library;
use overtone_metadata::acoustid::{AcoustId, Fingerprint, base64_url};
use overtone_metadata::discogs::Discogs;
use overtone_metadata::matching::DEFAULT_STRICTNESS;
use overtone_metadata::musicbrainz::MusicBrainz;
use overtone_metadata::{Http, LookupError, Match, Query, UreqHttp, best_match, user_agent};
use overtone_model::{MetadataField, TrackId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::CoreError;

/// Library settings keys (environment variables override them).
pub const SETTING_DISCOGS_TOKEN: &str = "lookup.discogs_token";
pub const SETTING_ACOUSTID_KEY: &str = "lookup.acoustid_key";
pub const SETTING_CONTACT: &str = "lookup.contact";

/// A metadata source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS, Deserialize)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    AcoustId,
    MusicBrainz,
    Discogs,
}

impl Provider {
    pub const ALL: [Self; 3] = [Self::AcoustId, Self::MusicBrainz, Self::Discogs];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AcoustId => "acoustid",
            Self::MusicBrainz => "musicbrainz",
            Self::Discogs => "discogs",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == value)
    }
}

/// Credentials and contact for the providers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LookupConfig {
    pub discogs_token: Option<String>,
    pub acoustid_key: Option<String>,
    /// Sent in the User-Agent when set (MusicBrainz asks for one).
    pub contact: Option<String>,
}

impl LookupConfig {
    /// Reads settings, with `OVERTONE_DISCOGS_TOKEN`,
    /// `OVERTONE_ACOUSTID_KEY`, and `OVERTONE_LOOKUP_CONTACT` taking
    /// precedence.
    pub fn load(library: &Library) -> Result<Self, CoreError> {
        let read = |env: &str, key: &str| -> Result<Option<String>, CoreError> {
            if let Some(value) = std::env::var(env).ok().filter(|v| !v.trim().is_empty()) {
                return Ok(Some(value));
            }
            Ok(library
                .setting::<String>(key)?
                .filter(|v| !v.trim().is_empty()))
        };
        Ok(Self {
            discogs_token: read("OVERTONE_DISCOGS_TOKEN", SETTING_DISCOGS_TOKEN)?,
            acoustid_key: read("OVERTONE_ACOUSTID_KEY", SETTING_ACOUSTID_KEY)?,
            contact: read("OVERTONE_LOOKUP_CONTACT", SETTING_CONTACT)?,
        })
    }

    /// The providers this configuration can use, in lookup order.
    pub fn available(&self) -> Vec<Provider> {
        Provider::ALL
            .into_iter()
            .filter(|p| match p {
                Provider::AcoustId => self.acoustid_key.is_some(),
                Provider::MusicBrainz => true,
                Provider::Discogs => self.discogs_token.is_some(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct LookupOptions {
    /// Providers to use; empty means every configured one.
    pub providers: Vec<Provider>,
    pub cancel: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct LookupProgress {
    pub done: u32,
    pub total: u32,
    pub track_id: TrackId,
    pub suggestions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct LookupFailure {
    pub track_id: TrackId,
    pub provider: Provider,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, Deserialize)]
#[ts(export)]
pub struct LookupReport {
    pub looked_up: u32,
    /// Tracks with at least one match.
    pub matched: u32,
    pub suggestions_added: u32,
    pub providers: Vec<Provider>,
    /// Providers turned off during the run because they refused the
    /// credentials.
    pub disabled: Vec<Provider>,
    pub failures: Vec<LookupFailure>,
    pub cancelled: bool,
}

fn effective(library: &Library, track: TrackId, field: MetadataField) -> Result<String, CoreError> {
    Ok(library.field_value(track, field)?.value.unwrap_or_default())
}

fn query_for(library: &Library, track: TrackId) -> Result<(Query, String), CoreError> {
    let stored = library.track(track)?;
    let mut title = effective(library, track, MetadataField::Title)?;
    if title.is_empty() {
        title = Path::new(&stored.path)
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    }
    Ok((
        Query {
            title,
            artist: effective(library, track, MetadataField::Artist)?,
            duration_secs: stored.properties.duration_ms.map(|ms| ms / 1000),
        },
        stored.path,
    ))
}

fn is_auth_error(err: &LookupError) -> bool {
    matches!(
        err,
        LookupError::Status {
            status: 401 | 403,
            ..
        }
    ) || err.to_string().contains("invalid API key")
}

/// Looks up `ids` with the real HTTP client.
pub fn lookup_tracks(
    library: &mut Library,
    ids: &[TrackId],
    options: &LookupOptions,
    on_progress: &dyn Fn(LookupProgress),
) -> Result<LookupReport, CoreError> {
    let config = LookupConfig::load(library)?;
    let http = UreqHttp::new(user_agent(config.contact.as_deref()));
    lookup_tracks_with(library, ids, &config, &http, options, on_progress)
}

/// Looks up `ids` through `http`, one track at a time (the providers rate
/// limit), adding suggestions and recording each lookup.
pub fn lookup_tracks_with(
    library: &mut Library,
    ids: &[TrackId],
    config: &LookupConfig,
    http: &dyn Http,
    options: &LookupOptions,
    on_progress: &dyn Fn(LookupProgress),
) -> Result<LookupReport, CoreError> {
    let available = config.available();
    let mut providers: Vec<Provider> = if options.providers.is_empty() {
        available.clone()
    } else {
        options
            .providers
            .iter()
            .copied()
            .filter(|p| available.contains(p))
            .collect()
    };
    let ffmpeg = overtone_ffmpeg::Ffmpeg::locate().ok();
    let musicbrainz = MusicBrainz::new(http);
    let discogs = config
        .discogs_token
        .as_deref()
        .and_then(|t| Discogs::new(http, t).ok());
    let acoustid = config
        .acoustid_key
        .as_deref()
        .and_then(|k| AcoustId::new(http, k).ok());

    let total = u32::try_from(ids.len()).unwrap_or(u32::MAX);
    let mut report = LookupReport {
        looked_up: 0,
        matched: 0,
        suggestions_added: 0,
        providers: providers.clone(),
        disabled: Vec::new(),
        failures: Vec::new(),
        cancelled: false,
    };
    for &track in ids {
        if options
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
        {
            report.cancelled = true;
            break;
        }
        let (query, path) = query_for(library, track)?;
        let mut added = 0u32;
        let mut any_match = false;
        let mut identified = false;
        for provider in providers.clone() {
            // A fingerprint match already fetched the MusicBrainz recording.
            if provider == Provider::MusicBrainz && identified {
                library.record_lookup(track, provider.as_str(), true)?;
                continue;
            }
            let found: Result<Option<Match>, LookupError> = match provider {
                Provider::AcoustId => {
                    let Some(acoustid) = &acoustid else { continue };
                    match fingerprint::fingerprint(Path::new(&path), ffmpeg.as_ref()) {
                        Ok(print) => acoustid
                            .identify(
                                &Fingerprint {
                                    encoded: base64_url(&print.compressed),
                                    duration_secs: print.duration_secs,
                                },
                                &musicbrainz,
                            )
                            .and_then(|candidate| {
                                let Some(found) = candidate
                                    .and_then(|c| best_match(&query, vec![c], DEFAULT_STRICTNESS))
                                else {
                                    return Ok(None);
                                };
                                let mut found = found;
                                musicbrainz.add_labels(&mut found.candidate)?;
                                Ok(Some(found))
                            }),
                        Err(err) => Err(LookupError::BadResponse {
                            provider: "acoustid",
                            message: format!("couldn't fingerprint the file: {err}"),
                        }),
                    }
                }
                Provider::MusicBrainz => musicbrainz.search(&query).and_then(|candidates| {
                    let Some(mut found) = best_match(&query, candidates, DEFAULT_STRICTNESS) else {
                        return Ok(None);
                    };
                    musicbrainz.complete(&mut found.candidate)?;
                    Ok(Some(found))
                }),
                Provider::Discogs => {
                    let Some(discogs) = &discogs else { continue };
                    discogs
                        .search(&query)
                        .map(|c| best_match(&query, c, DEFAULT_STRICTNESS))
                }
            };
            match found {
                Ok(Some(found)) => {
                    any_match = true;
                    identified |= provider == Provider::AcoustId;
                    let n = library.add_suggestions(track, &found.suggestions())?;
                    added += u32::try_from(n).unwrap_or(u32::MAX);
                    library.record_lookup(track, provider.as_str(), true)?;
                }
                Ok(None) => library.record_lookup(track, provider.as_str(), false)?,
                Err(err) => {
                    if is_auth_error(&err) {
                        providers.retain(|p| *p != provider);
                        report.disabled.push(provider);
                    }
                    report.failures.push(LookupFailure {
                        track_id: track,
                        provider,
                        message: err.to_string(),
                    });
                }
            }
        }
        report.looked_up += 1;
        report.matched += u32::from(any_match);
        report.suggestions_added += added;
        on_progress(LookupProgress {
            done: report.looked_up,
            total,
            track_id: track,
            suggestions: added,
        });
    }
    Ok(report)
}

/// Tracks no selected provider has looked up yet.
pub fn tracks_needing_lookup(
    library: &Library,
    providers: &[Provider],
) -> Result<Vec<TrackId>, CoreError> {
    let mut ids = Vec::new();
    for provider in providers {
        for id in library.tracks_needing_lookup(provider.as_str())? {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use overtone_library::ScannedTrack;
    use overtone_metadata::HttpResponse;
    use overtone_model::{AudioProperties, SuggestionStatus, TrackTags};

    use super::*;

    struct Canned(Vec<(&'static str, u16, String)>);

    impl Http for Canned {
        fn get(&self, url: &str, _: &[(&str, String)]) -> Result<HttpResponse, String> {
            self.0
                .iter()
                .find(|(key, _, _)| url.contains(key))
                .map(|(_, status, body)| HttpResponse {
                    status: *status,
                    body: body.clone(),
                })
                .ok_or_else(|| format!("no response for {url}"))
        }
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/../metadata/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture")
    }

    fn library_with(title: &str, artist: &str, secs: u64) -> (Library, TrackId) {
        let mut library = Library::open_in_memory().expect("open");
        let (id, _) = library
            .upsert_scanned(&ScannedTrack {
                path: "/music/one-more-time.flac".into(),
                file_size: 1,
                modified_at: 1,
                audio_hash: None,
                properties: AudioProperties {
                    duration_ms: Some(secs * 1000),
                    ..AudioProperties::default()
                },
                tags: TrackTags {
                    title: Some(title.into()),
                    artist: Some(artist.into()),
                    ..TrackTags::default()
                },
            })
            .expect("insert");
        (library, id)
    }

    #[test]
    fn musicbrainz_match_becomes_pending_suggestions_and_is_remembered() {
        let (mut library, id) = library_with("One More Time", "Daft Punk", 320);
        let http = Canned(vec![
            ("/recording?query=", 200, fixture("mb_search.json")),
            ("/recording/", 200, fixture("mb_recording.json")),
            ("/release/", 200, fixture("mb_release.json")),
        ]);
        let report = lookup_tracks_with(
            &mut library,
            &[id],
            &LookupConfig::default(),
            &http,
            &LookupOptions::default(),
            &|_| {},
        )
        .expect("lookup");
        assert_eq!(report.providers, [Provider::MusicBrainz]);
        assert_eq!((report.looked_up, report.matched), (1, 1));
        assert!(report.suggestions_added >= 2, "{report:?}");
        let pending = library
            .suggestions(Some(id), Some(SuggestionStatus::Pending))
            .expect("pending");
        assert!(pending.iter().any(|s| s.field == MetadataField::Label));
        // Nothing changed until the DJ accepts.
        assert_eq!(
            library
                .field_value(id, MetadataField::Label)
                .expect("label")
                .value,
            None
        );
        assert!(
            tracks_needing_lookup(&library, &[Provider::MusicBrainz])
                .expect("pending")
                .is_empty()
        );
    }

    #[test]
    fn a_refused_discogs_token_disables_discogs_for_the_run() {
        let (mut library, id) = library_with("One More Time", "Daft Punk", 320);
        let http = Canned(vec![
            ("/recording?query=", 200, r#"{"recordings":[]}"#.into()),
            (
                "database/search",
                401,
                r#"{"message":"You must authenticate"}"#.into(),
            ),
        ]);
        let config = LookupConfig {
            discogs_token: Some("bad".into()),
            ..LookupConfig::default()
        };
        let report = lookup_tracks_with(
            &mut library,
            &[id],
            &config,
            &http,
            &LookupOptions::default(),
            &|_| {},
        )
        .expect("lookup");
        assert_eq!(report.disabled, [Provider::Discogs]);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.matched, 0);
    }

    #[test]
    fn provider_names_match_their_json() {
        for provider in Provider::ALL {
            let json = serde_json::to_string(&provider).expect("serialize");
            assert_eq!(json, format!("\"{}\"", provider.as_str()));
        }
    }

    #[test]
    fn providers_need_their_credentials() {
        assert_eq!(LookupConfig::default().available(), [Provider::MusicBrainz]);
        let config = LookupConfig {
            discogs_token: Some("t".into()),
            acoustid_key: Some("k".into()),
            contact: None,
        };
        assert_eq!(config.available(), Provider::ALL);
    }
}
