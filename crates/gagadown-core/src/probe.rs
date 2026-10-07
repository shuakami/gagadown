use crate::error::{DlError, DlResult};
use crate::request::RequestSpec;
use reqwest::header::{HeaderMap, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, LAST_MODIFIED};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProbeInfo {
    pub final_url: String,
    pub size: Option<u64>,
    pub ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub route: String,
    pub latency_ms: u64,
    #[serde(default)]
    pub status: u16,
}

pub fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let v = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(secs) = v.trim().parse::<u64>() {
        return Some(Duration::from_secs(secs.min(600)));
    }
    let t = httpdate::parse_http_date(v).ok()?;
    t.duration_since(std::time::SystemTime::now()).ok().map(|d| d.min(Duration::from_secs(600)))
}

/// `bytes 0-0/12345` -> (0, 0, Some(12345))
pub fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let v = v.trim().strip_prefix("bytes")?.trim();
    let (range, total) = v.split_once('/')?;
    let total = total.trim().parse::<u64>().ok();
    let (a, b) = range.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?, total))
}

pub fn filename_from_disposition(v: &str) -> Option<String> {
    // RFC 6266: filename* wins over filename.
    let mut plain = None;
    for part in v.split(';') {
        let part = part.trim();
        let Some((k, val)) = part.split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let val = val.trim().trim_matches('"');
        if k == "filename*" {
            let enc = val.splitn(3, '\'').nth(2).unwrap_or(val);
            if let Ok(s) = percent_encoding::percent_decode_str(enc).decode_utf8() {
                if !s.is_empty() {
                    return Some(s.into_owned());
                }
            }
        } else if k == "filename" && !val.is_empty() {
            let decoded = percent_encoding::percent_decode_str(val)
                .decode_utf8()
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| val.to_string());
            plain = Some(decoded);
        }
    }
    plain
}

pub fn filename_from_url(u: &str) -> Option<String> {
    let url = url::Url::parse(u).ok()?;
    let seg = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    let s = percent_encoding::percent_decode_str(seg).decode_utf8().ok()?.into_owned();
    if s.is_empty() { None } else { Some(s) }
}

pub fn sanitize_filename(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    out = out.trim().trim_end_matches(['.', ' ']).to_string();
    let stem = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3"];
    if RESERVED.contains(&stem.as_str()) {
        out.insert(0, '_');
    }
    if out.is_empty() {
        out = "download".into();
    }
    if out.len() > 200 {
        let ext = std::path::Path::new(&out).extension().map(|e| e.to_string_lossy().to_string());
        let mut cut = 180;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        if let Some(e) = ext {
            out.push('.');
            out.push_str(&e);
        }
    }
    out
}

/// Probe with `GET Range: bytes=0-0`: works with presigned URLs that reject HEAD, and
/// tells us size + range support in one round trip.
pub async fn probe(client: &reqwest::Client, spec: &RequestSpec, url: &str, ua: &str, timeout: Duration) -> DlResult<ProbeInfo> {
    let started = std::time::Instant::now();
    let req = client
        .get(url)
        .headers(spec.header_map(ua))
        .header(reqwest::header::RANGE, "bytes=0-0")
        .timeout(timeout);
    let resp = req.send().await?;
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let final_url = resp.url().to_string();
    drop(resp);

    let hs = |k| headers.get(k).and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok()).map(|s| s.to_string());
    let mut info = ProbeInfo {
        final_url: final_url.clone(),
        etag: hs(ETAG).filter(|e| !e.starts_with("W/")),
        last_modified: hs(LAST_MODIFIED),
        content_type: hs(CONTENT_TYPE),
        filename: hs(CONTENT_DISPOSITION).and_then(|d| filename_from_disposition(&d)),
        latency_ms: started.elapsed().as_millis() as u64,
        status,
        ..Default::default()
    };
    match status {
        206 => {
            let cr = hs(CONTENT_RANGE).and_then(|v| parse_content_range(&v));
            match cr {
                Some((0, _, total)) => {
                    info.size = total;
                    info.ranges = total.is_some();
                }
                _ => {
                    info.ranges = false;
                }
            }
        }
        200 => {
            info.size = hs(CONTENT_LENGTH).and_then(|v| v.parse().ok());
            info.ranges = false;
        }
        416 => {
            // Zero-length resources answer 416 to bytes=0-0.
            let total = hs(CONTENT_RANGE).and_then(|v| v.rsplit('/').next().and_then(|t| t.trim().parse().ok()));
            info.size = Some(total.unwrap_or(0));
            info.ranges = false;
        }
        s => return Err(DlError::from_status(s, retry_after(&headers))),
    }
    if info.filename.is_none() {
        info.filename = filename_from_url(&final_url).or_else(|| filename_from_url(url));
    }
    if let Some(n) = &info.filename {
        info.filename = Some(sanitize_filename(n));
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition() {
        assert_eq!(filename_from_disposition("attachment; filename=\"a b.zip\"").unwrap(), "a b.zip");
        assert_eq!(
            filename_from_disposition("attachment; filename=\"x.zip\"; filename*=UTF-8''%E4%B8%AD%E6%96%87.zip").unwrap(),
            "中文.zip"
        );
    }

    #[test]
    fn content_range() {
        assert_eq!(parse_content_range("bytes 0-0/1234"), Some((0, 0, Some(1234))));
        assert_eq!(parse_content_range("bytes 10-20/*"), Some((10, 20, None)));
    }

    #[test]
    fn sanitize() {
        assert_eq!(sanitize_filename("a/b:c?.txt"), "a_b_c_.txt");
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
    }
}

impl ProbeInfo {
    /// Small HTML answers are almost always login walls or interstitials, not the file.
    pub fn looks_like_page(&self) -> bool {
        self.content_type.as_deref().map(|c| c.starts_with("text/html")).unwrap_or(false)
            && self.size.map(|s| s < 2 * 1024 * 1024).unwrap_or(true)
    }
}
