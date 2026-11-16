//! HTTPS transport abstraction with conditional HTTP (`ETag` / `Last-Modified`),
//! rate-limit detection, size caps, and a mock implementation for tests.

use std::io::Read;
use std::time::Duration;

use nekod_manifest::validate_https_url;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, RETRY_AFTER};

use crate::RegistryError;

/// Maximum bytes allowed when fetching a JSON `RegistryManifest` index (4 MiB).
pub const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
/// Maximum bytes allowed when fetching a trainer package artifact (16 MiB).
pub const MAX_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;

/// Response returned by an HTTP transport after conditional header evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpFetchResponse {
    /// Server responded `304 Not Modified`; cached copy remains current.
    NotModified,
    /// Server responded `200 OK` with bounded body bytes and optional cache headers.
    Ok {
        body: Vec<u8>,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

/// Abstract HTTPS transport for fetching registry indexes and verified trainer releases.
pub trait HttpTransport: Send + Sync {
    fn fetch(
        &self,
        url: &str,
        if_none_match: Option<&str>,
        if_modified_since: Option<&str>,
        max_bytes: usize,
    ) -> Result<HttpFetchResponse, RegistryError>;
}

/// Production `reqwest::blocking` transport enforcing HTTPS-only public URLs,
/// conditional caching, rate-limit surfacing, and strict download size limits.
pub struct ReqwestHttpTransport {
    client: Client,
}

impl ReqwestHttpTransport {
    pub fn new() -> Result<Self, RegistryError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("Nekod/0.1 (Community Game Trainer Client)")
            .build()
            .map_err(|e| RegistryError::Transport("client-init".to_owned(), e.to_string()))?;
        Ok(Self { client })
    }
}

impl HttpTransport for ReqwestHttpTransport {
    fn fetch(
        &self,
        url: &str,
        if_none_match: Option<&str>,
        if_modified_since: Option<&str>,
        max_bytes: usize,
    ) -> Result<HttpFetchResponse, RegistryError> {
        validate_https_url(url).map_err(|e| RegistryError::InvalidUrl(e.to_string()))?;

        let mut req = self.client.get(url);
        if let Some(etag) = if_none_match {
            req = req.header(IF_NONE_MATCH, etag);
        }
        if let Some(lm) = if_modified_since {
            req = req.header(IF_MODIFIED_SINCE, lm);
        }

        let resp = req
            .send()
            .map_err(|e| RegistryError::Transport(url.to_owned(), e.to_string()))?;

        let status = resp.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(HttpFetchResponse::NotModified);
        }
        if status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
            return Err(RegistryError::UpstreamNotFound(url.to_owned()));
        }
        if status == StatusCode::TOO_MANY_REQUESTS
            || (status == StatusCode::FORBIDDEN
                && resp
                    .headers()
                    .get("x-ratelimit-remaining")
                    .and_then(|v| v.to_str().ok())
                    == Some("0"))
        {
            let retry_after_secs = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
            return Err(RegistryError::RateLimited {
                target: url.to_owned(),
                retry_after_secs,
            });
        }
        if !status.is_success() {
            return Err(RegistryError::Transport(
                url.to_owned(),
                format!("HTTP status {}", status.as_u16()),
            ));
        }

        // Reject cross-origin/local redirect targets if final URL violated public HTTPS policy.
        validate_https_url(resp.url().as_str())
            .map_err(|e| RegistryError::InvalidUrl(format!("redirect rejected: {e}")))?;

        if let Some(len) = resp.content_length()
            && len > max_bytes as u64
        {
            return Err(RegistryError::PayloadTooLarge {
                target: url.to_owned(),
                limit_bytes: max_bytes,
            });
        }

        let etag = resp
            .headers()
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);
        let last_modified = resp
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);

        let mut buf = Vec::new();
        let mut reader = resp.take((max_bytes as u64).saturating_add(1));
        reader
            .read_to_end(&mut buf)
            .map_err(|e| RegistryError::Transport(url.to_owned(), e.to_string()))?;
        if buf.len() > max_bytes {
            return Err(RegistryError::PayloadTooLarge {
                target: url.to_owned(),
                limit_bytes: max_bytes,
            });
        }

        Ok(HttpFetchResponse::Ok {
            body: buf,
            etag,
            last_modified,
        })
    }
}

/// Scriptable offline HTTP mock for deterministic P3 registry & download tests.
pub mod mock {
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Debug, Clone)]
    pub enum MockHttpReply {
        Ok {
            body: Vec<u8>,
            etag: Option<String>,
            last_modified: Option<String>,
        },
        NotModified,
        Error(RegistryError),
    }

    type RouteMap = BTreeMap<String, VecDeque<MockHttpReply>>;
    type RequestLog = Vec<(String, Option<String>)>;

    #[derive(Debug, Default, Clone)]
    pub struct MockHttpTransport {
        routes: Arc<Mutex<RouteMap>>,
        requests: Arc<Mutex<RequestLog>>,
    }

    impl MockHttpTransport {
        pub fn new() -> Self {
            Self::default()
        }

        /// Enqueues a static 200 OK response for `url` (reused on subsequent calls if single entry).
        pub fn set_ok(&self, url: impl Into<String>, body: impl Into<Vec<u8>>, etag: Option<&str>) {
            let mut map = self.routes.lock().unwrap();
            let q = map.entry(url.into()).or_default();
            q.push_back(MockHttpReply::Ok {
                body: body.into(),
                etag: etag.map(ToOwned::to_owned),
                last_modified: None,
            });
        }

        /// Enqueues a scripted HTTP reply (`NotModified`, `Ok`, or `Error`) in FIFO order.
        pub fn enqueue(&self, url: impl Into<String>, reply: MockHttpReply) {
            let mut map = self.routes.lock().unwrap();
            map.entry(url.into()).or_default().push_back(reply);
        }

        /// Returns recorded `(url, if_none_match)` requests seen by this mock.
        pub fn recorded_requests(&self) -> Vec<(String, Option<String>)> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl HttpTransport for MockHttpTransport {
        fn fetch(
            &self,
            url: &str,
            if_none_match: Option<&str>,
            _if_modified_since: Option<&str>,
            max_bytes: usize,
        ) -> Result<HttpFetchResponse, RegistryError> {
            validate_https_url(url).map_err(|e| RegistryError::InvalidUrl(e.to_string()))?;
            self.requests
                .lock()
                .unwrap()
                .push((url.to_owned(), if_none_match.map(ToOwned::to_owned)));

            let reply = {
                let mut map = self.routes.lock().unwrap();
                let queue = map
                    .get_mut(url)
                    .ok_or_else(|| RegistryError::UpstreamNotFound(url.to_owned()))?;
                if queue.len() > 1 {
                    queue.pop_front().unwrap()
                } else {
                    queue
                        .front()
                        .cloned()
                        .ok_or_else(|| RegistryError::UpstreamNotFound(url.to_owned()))?
                }
            };

            match reply {
                MockHttpReply::Ok {
                    body,
                    etag,
                    last_modified,
                } => {
                    if body.len() > max_bytes {
                        return Err(RegistryError::PayloadTooLarge {
                            target: url.to_owned(),
                            limit_bytes: max_bytes,
                        });
                    }
                    Ok(HttpFetchResponse::Ok {
                        body,
                        etag,
                        last_modified,
                    })
                }
                MockHttpReply::NotModified => Ok(HttpFetchResponse::NotModified),
                MockHttpReply::Error(err) => Err(err),
            }
        }
    }
}
