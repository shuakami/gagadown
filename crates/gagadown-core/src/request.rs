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

pub(crate) const BAIDU_PAN_UA: &str = "pan.baidu.com";

fn domain_or_subdomain(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).is_some_and(|prefix| prefix.ends_with('.'))
}

fn normalized_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

pub(crate) fn is_baidu_host(host: &str) -> bool {
    let host = normalized_host(host);
    domain_or_subdomain(&host, "baidu.com") || domain_or_subdomain(&host, "baidupcs.com")
}

pub(crate) fn is_baidu_pcs_host(host: &str) -> bool {
    let host = normalized_host(host);
    domain_or_subdomain(&host, "pcs.baidu.com") || domain_or_subdomain(&host, "baidupcs.com")
}

pub(crate) fn is_baidu_download_url(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(is_baidu_host))
        .unwrap_or(false)
}

pub(crate) fn is_baidu_pcs_url(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(is_baidu_pcs_host))
        .unwrap_or(false)
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
        if is_baidu_download_url(&self.url) {
            map.insert(reqwest::header::USER_AGENT, HeaderValue::from_static(BAIDU_PAN_UA));
        } else if !has_ua {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ua(spec: &RequestSpec) -> String {
        spec.header_map(DEFAULT_UA)
            .get(reqwest::header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    }

    #[test]
    fn baidu_download_hosts_match_exact_domain_boundaries() {
        for host in [
            "baidu.com",
            "pan.baidu.com",
            "d.pcs.baidu.com",
            "baidupcs.com",
            "yq-cm20.baidupcs.com",
            "D.PCS.BAIDU.COM.",
        ] {
            assert!(is_baidu_host(host), "{host}");
        }
        for host in ["notbaidu.com", "evilbaidupcs.com", "baidu.com.evil.com", "evil.pcs.baidu.com.evil.com"] {
            assert!(!is_baidu_host(host), "{host}");
        }
    }

    #[test]
    fn baidu_pcs_hosts_stay_scoped_to_download_endpoints() {
        for host in ["pcs.baidu.com", "d.pcs.baidu.com", "baidupcs.com", "yq-cm20.baidupcs.com"] {
            assert!(is_baidu_pcs_host(host), "{host}");
        }
        for host in ["baidu.com", "pan.baidu.com", "www.baidu.com", "evilbaidupcs.com"] {
            assert!(!is_baidu_pcs_host(host), "{host}");
        }
    }

    #[test]
    fn baidu_download_urls_force_pan_user_agent() {
        for url in [
            "https://pan.baidu.com/rest/2.0/xpan/file?method=download",
            "https://d.pcs.baidu.com/file/example",
            "https://yq-cm20.baidupcs.com/file/example",
        ] {
            let mut spec = RequestSpec::new(url);
            spec.user_agent = Some("custom-agent".into());
            spec.headers.push(("User-Agent".into(), "browser-agent".into()));
            assert_eq!(ua(&spec), BAIDU_PAN_UA, "{url}");
        }
    }

    #[test]
    fn other_urls_keep_existing_user_agent_priority() {
        let mut spec = RequestSpec::new("https://example.com/file.zip");
        spec.user_agent = Some("task-agent".into());
        assert_eq!(ua(&spec), "task-agent");

        spec.headers.push(("User-Agent".into(), "captured-agent".into()));
        assert_eq!(ua(&spec), "captured-agent");
    }
}
