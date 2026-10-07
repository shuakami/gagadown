use crate::config::DEFAULT_UA;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};

/// Everything needed to replay the exact request the browser made.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestSpec {
    pub url: String,
    /// Extra mirrors of the same file; workers spread across all of them.
    pub mirrors: Vec<String>,
    pub headers: Vec<(String, String)>,
    pub cookies: Option<String>,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
}

/// Headers we must control ourselves; anything captured from the browser with these names is dropped.
const MANAGED: &[&str] = &[
    "range",
    "if-range",
    "accept-encoding",
    "content-length",
    "host",
    "connection",
    "transfer-encoding",
    "upgrade",
    "keep-alive",
    "te",
    "proxy-connection",
    "if-none-match",
    "if-modified-since",
    "expect",
];

impl RequestSpec {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into(), ..Default::default() }
    }

    pub fn host(&self) -> String {
        url::Url::parse(&self.url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_default()
    }

    pub fn header_map(&self, default_ua: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        let mut has_ua = false;
        let mut has_cookie = false;
        let mut has_referer = false;
        for (k, v) in &self.headers {
            let lk = k.to_ascii_lowercase();
            if MANAGED.contains(&lk.as_str()) || lk.starts_with(':') {
                continue;
            }
            let (Ok(name), Ok(val)) = (HeaderName::from_bytes(lk.as_bytes()), HeaderValue::from_str(v)) else {
                continue;
            };
            has_ua |= lk == "user-agent";
            has_cookie |= lk == "cookie";
            has_referer |= lk == "referer";
            map.append(name, val);
        }
        if !has_ua {
            let ua = self.user_agent.as_deref().filter(|s| !s.is_empty()).unwrap_or(if default_ua.is_empty() {
                DEFAULT_UA
            } else {
                default_ua
            });
            if let Ok(v) = HeaderValue::from_str(ua) {
                map.insert(reqwest::header::USER_AGENT, v);
            }
        }
        if !has_cookie {
            if let Some(c) = self.cookies.as_deref().filter(|s| !s.is_empty()) {
                if let Ok(v) = HeaderValue::from_str(c) {
                    map.insert(reqwest::header::COOKIE, v);
                }
            }
        }
        if !has_referer {
            if let Some(r) = self.referrer.as_deref().filter(|s| !s.is_empty()) {
                if let Ok(v) = HeaderValue::from_str(r) {
                    map.insert(reqwest::header::REFERER, v);
                }
            }
        }
        if !map.contains_key(reqwest::header::ACCEPT) {
            map.insert(reqwest::header::ACCEPT, HeaderValue::from_static("*/*"));
        }
        // Compression would break byte ranges.
        map.insert(reqwest::header::ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        map
    }

    pub fn all_urls(&self) -> Vec<String> {
        let mut v = vec![self.url.clone()];
        for m in &self.mirrors {
            if !m.is_empty() && !v.contains(m) {
                v.push(m.clone());
            }
        }
        v
    }
}
