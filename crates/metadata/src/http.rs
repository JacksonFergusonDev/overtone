//! A small HTTP seam so providers can be tested against recorded responses.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A response: status and body text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// Performs GET requests. Implemented by [`UreqHttp`] and by test fakes.
pub trait Http: Send + Sync {
    fn get(&self, url: &str, headers: &[(&str, String)]) -> Result<HttpResponse, String>;
}

/// The real client: `ureq` with rustls, a fixed User-Agent, a timeout, and
/// a minimum gap between requests to the same host (MusicBrainz allows one
/// request per second).
pub struct UreqHttp {
    agent: ureq::Agent,
    user_agent: String,
    min_gap: HashMap<&'static str, Duration>,
    last: Mutex<HashMap<String, Instant>>,
}

impl UreqHttp {
    pub fn new(user_agent: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self {
            agent,
            user_agent,
            min_gap: HashMap::from([
                ("musicbrainz.org", Duration::from_millis(1_100)),
                ("api.discogs.com", Duration::from_millis(1_100)),
                ("api.acoustid.org", Duration::from_millis(350)),
            ]),
            last: Mutex::new(HashMap::new()),
        }
    }

    fn wait_for(&self, url: &str) {
        let host = url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or_default()
            .to_string();
        let gap = self
            .min_gap
            .iter()
            .find(|(h, _)| host.ends_with(*h))
            .map_or(Duration::ZERO, |(_, gap)| *gap);
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = last.get(&host)
            && let Some(remaining) = gap.checked_sub(previous.elapsed())
        {
            std::thread::sleep(remaining);
        }
        last.insert(host, Instant::now());
    }
}

impl Http for UreqHttp {
    fn get(&self, url: &str, headers: &[(&str, String)]) -> Result<HttpResponse, String> {
        // Back off and retry after a rate-limit or overload answer
        // (MusicBrainz answers 503 "busy" under load).
        const ATTEMPTS: u32 = 4;
        for attempt in 0..ATTEMPTS {
            self.wait_for(url);
            let mut request = self
                .agent
                .get(url)
                .header("User-Agent", &self.user_agent)
                .header("Accept", "application/json");
            for (name, value) in headers {
                request = request.header(*name, value);
            }
            let mut response = request.call().map_err(|err| err.to_string())?;
            let status = response.status().as_u16();
            if matches!(status, 429 | 503) && attempt + 1 < ATTEMPTS {
                std::thread::sleep(Duration::from_secs(2u64.pow(attempt + 1)));
                continue;
            }
            let body = response
                .body_mut()
                .read_to_string()
                .map_err(|err| err.to_string())?;
            return Ok(HttpResponse { status, body });
        }
        unreachable!("the loop returns on its last attempt")
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::{Http, HttpResponse};
    use std::sync::Mutex;

    /// Answers each request with the first canned response whose key is
    /// part of the URL, and records the URLs it was asked for.
    pub struct FakeHttp {
        pub responses: Vec<(&'static str, u16, String)>,
        pub requests: Mutex<Vec<String>>,
    }

    impl FakeHttp {
        pub fn new(responses: Vec<(&'static str, u16, String)>) -> Self {
            Self {
                responses,
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl Http for FakeHttp {
        fn get(&self, url: &str, _headers: &[(&str, String)]) -> Result<HttpResponse, String> {
            self.requests.lock().expect("lock").push(url.to_string());
            self.responses
                .iter()
                .find(|(key, _, _)| url.contains(key))
                .map(|(_, status, body)| HttpResponse {
                    status: *status,
                    body: body.clone(),
                })
                .ok_or_else(|| format!("no canned response for {url}"))
        }
    }
}
