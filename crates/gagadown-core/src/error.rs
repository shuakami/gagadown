use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Coarse failure classes; drives retry policy and what the UI suggests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    Network,
    Timeout,
    Tls,
    ServerBusy,
    ServerError,
    Auth,
    NotFound,
    RangeUnsupported,
    ResourceChanged,
    DiskFull,
    Io,
    Cancelled,
    /// The server answered with a web page instead of a file.
    NotDownload,
    /// Another GaGaDown process already owns the data directory.
    AlreadyRunning,
    Other,
}

impl ErrorKind {
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorKind::Network
                | ErrorKind::Timeout
                | ErrorKind::Tls
                | ErrorKind::ServerBusy
                | ErrorKind::ServerError
                | ErrorKind::ResourceChanged
                | ErrorKind::RangeUnsupported
        )
    }

    /// Errors that suggest a different network route may help.
    pub fn route_related(self) -> bool {
        matches!(self, ErrorKind::Network | ErrorKind::Timeout | ErrorKind::Tls)
    }

    pub fn label(self) -> &'static str {
        match self {
            ErrorKind::Network => "网络连接失败",
            ErrorKind::Timeout => "连接超时",
            ErrorKind::Tls => "TLS 握手失败",
            ErrorKind::ServerBusy => "服务器限流",
            ErrorKind::ServerError => "服务器错误",
            ErrorKind::Auth => "认证失败或链接已过期",
            ErrorKind::NotFound => "文件不存在",
            ErrorKind::RangeUnsupported => "服务器不支持分段",
            ErrorKind::ResourceChanged => "远端文件已变化",
            ErrorKind::DiskFull => "磁盘空间不足",
            ErrorKind::Io => "文件读写错误",
            ErrorKind::Cancelled => "已取消",
            ErrorKind::NotDownload => "不是下载链接",
            ErrorKind::AlreadyRunning => "GaGaDown 已在运行",
            ErrorKind::Other => "未知错误",
        }
    }
}

/// How one network route failed; kept for the error report.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RouteFailure {
    pub route: String,
    pub status: Option<u16>,
    pub message: String,
}

/// Short Chinese reason for common transport errors; the raw text stays in the report.
pub fn brief(message: &str) -> Option<&'static str> {
    let m = message.to_ascii_lowercase();
    [
        ("dns error", "域名解析失败"),
        ("failed to lookup", "域名解析失败"),
        ("no such host", "域名解析失败"),
        ("connection refused", "连接被拒绝"),
        ("10061", "连接被拒绝"),
        ("connection reset", "连接被重置"),
        ("10054", "连接被重置"),
        ("timed out", "连接超时"),
        ("certificate", "证书无效"),
        ("unexpected eof", "连接意外断开"),
        ("connection closed", "连接意外断开"),
    ]
    .iter()
    .find(|(k, _)| m.contains(k))
    .map(|x| x.1)
}

/// One-line reason for lists and notifications, e.g. `服务器错误（HTTP 502）`.
pub fn summary(kind: ErrorKind, status: Option<u16>, message: &str) -> String {
    if let Some(s) = status {
        return format!("{}（HTTP {s}）", kind.label());
    }
    if kind == ErrorKind::Other && !message.is_empty() {
        return message.to_string();
    }
    match brief(message) {
        Some(b) if b != kind.label() => format!("{}（{b}）", kind.label()),
        _ => kind.label().to_string(),
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct DlError {
    pub kind: ErrorKind,
    pub message: String,
    pub status: Option<u16>,
    pub retry_after: Option<Duration>,
    /// Per-route failures when every route failed.
    pub routes: Vec<RouteFailure>,
}

impl DlError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), status: None, retry_after: None, routes: Vec::new() }
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "cancelled")
    }

    pub fn from_status(status: u16, retry_after: Option<Duration>) -> Self {
        let kind = match status {
            401 | 403 | 407 => ErrorKind::Auth,
            404 | 410 => ErrorKind::NotFound,
            416 => ErrorKind::RangeUnsupported,
            408 => ErrorKind::Timeout,
            429 | 503 => ErrorKind::ServerBusy,
            500..=599 => ErrorKind::ServerError,
            _ => ErrorKind::Other,
        };
        Self { kind, message: format!("HTTP {status}"), status: Some(status), retry_after, routes: Vec::new() }
    }

    pub fn from_reqwest(e: &reqwest::Error) -> Self {
        let mut chain = String::new();
        let mut src: Option<&dyn std::error::Error> = Some(e);
        while let Some(s) = src {
            if !chain.is_empty() {
                chain.push_str(": ");
            }
            chain.push_str(&s.to_string());
            src = s.source();
        }
        let lower = chain.to_lowercase();
        let kind = if e.is_timeout() {
            ErrorKind::Timeout
        } else if lower.contains("certificate") || lower.contains("tls") || lower.contains("handshake") {
            ErrorKind::Tls
        } else if let Some(st) = e.status() {
            return Self::from_status(st.as_u16(), None);
        } else {
            ErrorKind::Network
        };
        Self::new(kind, chain)
    }

    pub fn from_io(e: &std::io::Error) -> Self {
        let full = e.raw_os_error().map(|c| c == 28 || c == 112).unwrap_or(false)
            || e.kind() == std::io::ErrorKind::StorageFull;
        Self::new(if full { ErrorKind::DiskFull } else { ErrorKind::Io }, e.to_string())
    }
}

impl From<std::io::Error> for DlError {
    fn from(e: std::io::Error) -> Self {
        Self::from_io(&e)
    }
}

impl From<reqwest::Error> for DlError {
    fn from(e: reqwest::Error) -> Self {
        Self::from_reqwest(&e)
    }
}

pub type DlResult<T> = Result<T, DlError>;
