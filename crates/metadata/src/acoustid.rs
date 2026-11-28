//! AcoustID: identifies a recording from its audio fingerprint, then the
//! MusicBrainz recording supplies the metadata. Needs an application API
//! key (<https://acoustid.org/new-application>).
//! <https://acoustid.org/webservice#lookup>

use serde::Deserialize;

use crate::musicbrainz::MusicBrainz;
use crate::{Candidate, Http, LookupError, encode};

const PROVIDER: &str = "acoustid";

/// Lowest AcoustID score accepted as the same recording.
pub const MIN_SCORE: f64 = 0.8;

#[derive(Debug, Deserialize)]
struct LookupResponse {
    status: String,
    #[serde(default)]
    results: Vec<LookupResult>,
    error: Option<ErrorBody>,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    message: String,
}

#[derive(Debug, Deserialize)]
struct LookupResult {
    score: f64,
    #[serde(default)]
    recordings: Vec<RecordingId>,
}

#[derive(Debug, Deserialize)]
struct RecordingId {
    id: String,
}

/// A Chromaprint fingerprint, compressed and base64-encoded as AcoustID
/// expects, with the audio's length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub encoded: String,
    pub duration_secs: u64,
}

pub struct AcoustId<'a> {
    http: &'a dyn Http,
    key: String,
}

impl<'a> AcoustId<'a> {
    pub fn new(http: &'a dyn Http, key: &str) -> Result<Self, LookupError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(LookupError::NotConfigured {
                provider: PROVIDER,
                what: "an application API key",
            });
        }
        Ok(Self {
            http,
            key: key.to_string(),
        })
    }

    /// MusicBrainz recording IDs for a fingerprint, best score first, at
    /// [`MIN_SCORE`] or better.
    pub fn lookup(&self, fingerprint: &Fingerprint) -> Result<Vec<(f64, String)>, LookupError> {
        let url = format!(
            "https://api.acoustid.org/v2/lookup?client={}&meta=recordingids&duration={}&fingerprint={}",
            encode(&self.key),
            fingerprint.duration_secs,
            encode(&fingerprint.encoded)
        );
        let response = self
            .http
            .get(&url, &[])
            .map_err(|message| LookupError::Network {
                provider: PROVIDER,
                message,
            })?;
        let parsed: LookupResponse =
            serde_json::from_str(&response.body).map_err(|err| LookupError::BadResponse {
                provider: PROVIDER,
                message: err.to_string(),
            })?;
        if parsed.status != "ok" {
            return Err(LookupError::Status {
                provider: PROVIDER,
                status: response.status,
                message: parsed.error.map_or(parsed.status, |e| e.message),
            });
        }
        let mut ids: Vec<(f64, String)> = parsed
            .results
            .into_iter()
            .filter(|r| r.score >= MIN_SCORE)
            .flat_map(|r| r.recordings.into_iter().map(move |rec| (r.score, rec.id)))
            .collect();
        ids.sort_by(|a, b| b.0.total_cmp(&a.0));
        ids.dedup_by(|a, b| a.1 == b.1);
        Ok(ids)
    }

    /// Identifies the recording and fetches it from MusicBrainz. The
    /// candidate carries the fingerprint score, so it skips title matching.
    pub fn identify(
        &self,
        fingerprint: &Fingerprint,
        musicbrainz: &MusicBrainz<'_>,
    ) -> Result<Option<Candidate>, LookupError> {
        let Some((score, mbid)) = self.lookup(fingerprint)?.into_iter().next() else {
            return Ok(None);
        };
        let mut candidate = musicbrainz.recording(&mbid)?;
        candidate.provider = PROVIDER;
        candidate.provider_score = Some(score);
        Ok(Some(candidate))
    }
}

/// URL-safe base64 without padding, the encoding `fpcalc` and AcoustID use.
pub fn base64_url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::fake::FakeHttp;

    #[test]
    fn encodes_like_fpcalc() {
        assert_eq!(base64_url(b"Man"), "TWFu");
        assert_eq!(base64_url(b"Ma"), "TWE");
        assert_eq!(base64_url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn keeps_confident_results_and_fetches_the_recording() {
        let lookup = r#"{"status":"ok","results":[
            {"id":"a","score":0.62,"recordings":[{"id":"low"}]},
            {"id":"b","score":0.97,"recordings":[{"id":"1611a62a-bc3b-491f-bc56-75565ca42c38"}]}]}"#;
        let recording = std::fs::read_to_string(format!(
            "{}/tests/fixtures/mb_recording.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture");
        let http = FakeHttp::new(vec![
            ("acoustid.org", 200, lookup.into()),
            ("musicbrainz.org/ws/2/recording/", 200, recording),
        ]);
        let acoustid = AcoustId::new(&http, "key").expect("key");
        let fingerprint = Fingerprint {
            encoded: "AQAA".into(),
            duration_secs: 320,
        };
        let ids = acoustid.lookup(&fingerprint).expect("lookup");
        assert_eq!(ids.len(), 1);
        let candidate = acoustid
            .identify(&fingerprint, &MusicBrainz::new(&http))
            .expect("identify")
            .expect("found");
        assert_eq!(candidate.provider, "acoustid");
        assert_eq!(candidate.provider_score, Some(0.97));
    }

    #[test]
    fn reports_api_errors() {
        let http = FakeHttp::new(vec![(
            "acoustid.org",
            400,
            r#"{"status":"error","error":{"code":4,"message":"invalid API key"}}"#.into(),
        )]);
        let err = AcoustId::new(&http, "bad")
            .expect("key")
            .lookup(&Fingerprint {
                encoded: "x".into(),
                duration_secs: 1,
            })
            .expect_err("error");
        assert!(err.to_string().contains("invalid API key"));
    }
}
