use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ProxyMode {
    /// Direct connection first, fall back to proxies when direct fails or stalls.
    #[default]
    DirectThenProxy,
    DirectOnly,
    ProxyThenDirect,
    ProxyOnly,
}

impl ProxyMode {
    pub const ALL: [ProxyMode; 4] = [
        ProxyMode::DirectThenProxy,
        ProxyMode::DirectOnly,
        ProxyMode::ProxyThenDirect,
        ProxyMode::ProxyOnly,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ProxyMode::DirectThenProxy => "默认直连，失败自动走代理",
            ProxyMode::DirectOnly => "仅直连",
            ProxyMode::ProxyThenDirect => "优先代理，失败回退直连",
            ProxyMode::ProxyOnly => "仅代理",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxySettings {
    pub mode: ProxyMode,
    /// Manually configured proxies, e.g. `http://127.0.0.1:7890`, `socks5h://127.0.0.1:1080`.
    pub proxies: Vec<String>,
    /// Scan well-known local proxy ports (Clash, v2rayN, Shadowsocks...) and verify them.
    pub auto_detect: bool,
    /// Use OS / environment proxy settings as an extra candidate.
    pub use_system: bool,
    /// How long the direct route gets before proxies start racing it (ms).
    pub hedge_delay_ms: u64,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            mode: ProxyMode::DirectThenProxy,
            proxies: Vec::new(),
            auto_detect: true,
            use_system: true,
            hedge_delay_ms: 1500,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Presentation preference: zh-CN (default), en, or system.
    pub language: String,
    pub download_dir: PathBuf,
    /// Connections a single task starts with (the controller then probes upward).
    pub initial_connections: usize,
    /// Hard ceiling of connections per task.
    pub max_connections_per_task: usize,
    /// Ceiling across all running tasks.
    pub max_connections_total: usize,
    pub max_concurrent_tasks: usize,
    /// Segments never get split below this size.
    pub min_split_size: u64,
    /// Global speed limit in bytes/s, 0 = unlimited.
    pub speed_limit: u64,
    pub segment_retries: u32,
    pub task_auto_retries: u32,
    pub proxy: ProxySettings,
    pub api_port: u16,
    /// Browser extension only takes over downloads at least this large (bytes).
    pub takeover_min_size: u64,
    /// Days a deleted task stays in "recently deleted" before its leftovers are purged.
    pub deleted_retention_days: u64,
    /// Orphaned partial files older than this are cleaned automatically (0 = never).
    pub orphan_auto_clean_days: u64,
    pub user_agent: String,
    pub sync_on_complete: bool,
    /// Where partial files live while downloading; None = next to the target file.
    pub cache_dir: Option<PathBuf>,
    /// Start with the user session (Windows `Run` key).
    pub launch_at_login: bool,
    /// When launched at login, stay in the tray instead of opening the main window.
    pub start_minimized: bool,
}

pub const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

impl Default for Settings {
    fn default() -> Self {
        let download_dir = directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        Self {
            language: "zh-CN".to_owned(),
            download_dir,
            initial_connections: 16,
            max_connections_per_task: 64,
            max_connections_total: 512,
            max_concurrent_tasks: 4,
            min_split_size: 256 * 1024,
            speed_limit: 0,
            segment_retries: 16,
            task_auto_retries: 5,
            proxy: ProxySettings::default(),
            api_port: 18765,
            takeover_min_size: 1024 * 1024,
            deleted_retention_days: 7,
            orphan_auto_clean_days: 14,
            user_agent: DEFAULT_UA.to_string(),
            sync_on_complete: true,
            cache_dir: None,
            launch_at_login: false,
            start_minimized: false,
        }
    }
}

impl Settings {
    pub fn sanitized(mut self) -> Self {
        if !matches!(self.language.as_str(), "zh-CN" | "en" | "system") {
            self.language = "zh-CN".to_owned();
        }
        self.initial_connections = self.initial_connections.clamp(1, 256);
        self.max_connections_per_task = self.max_connections_per_task.clamp(1, 256);
        self.initial_connections = self.initial_connections.min(self.max_connections_per_task);
        self.max_connections_total = self.max_connections_total.clamp(1, 4096);
        self.max_concurrent_tasks = self.max_concurrent_tasks.clamp(1, 64);
        self.min_split_size = self.min_split_size.max(64 * 1024);
        self
    }
}
