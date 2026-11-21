//! 内置更新器：查 GitHub Releases 新版 → 下载对应平台资产 → 就地安装。
//! Flatpak 走 `flatpak-spawn --host flatpak install`（sandbox 里没有 flatpak）；
//! Windows 直接拉起 NSIS 安装器（无 UAC，安装器自己 taskkill 旧进程）；
//! 裸跑 Linux 只提示去发布页。

use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 上游发布仓库；release 资产名约定见 .github/workflows/release.yml。
pub const REPO: &str = "shuakami/gagadown";
pub const APP_ID: &str = "io.github.shuakami.GaGaDown";

#[derive(Clone, Debug)]
#[allow(dead_code)] // UI 入口待重新设计；Downloading/Failed 载荷暂无人读取
pub enum Stage {
    Idle,
    Checking,
    /// 发现新版：(tag, 资产下载 URL——非 flatpak Linux 时是发布页 URL)。
    Found(String, String),
    Downloading(u8),
    Installing,
    /// 已装好，等重启（Flatpak）。
    Installed,
    UpToDate,
    Failed(String),
}

pub type State = Arc<Mutex<Stage>>;

pub fn new_state() -> State {
    Arc::new(Mutex::new(Stage::Idle))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("GaGaDown/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

fn parse_ver(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches('v');
    let mut it = s.split('.');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

fn current() -> (u32, u32, u32) {
    parse_ver(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0))
}

/// 当前环境能否一键安装：Windows 总是可以；Linux 只有 flatpak 里可以。
pub fn can_self_install() -> bool {
    if cfg!(windows) {
        true
    } else {
        Path::new("/.flatpak-info").exists()
    }
}

fn want_asset(name: &str) -> bool {
    #[cfg(windows)]
    return name.starts_with("GagaDown-Setup-") && name.ends_with(".exe");
    #[cfg(target_os = "linux")]
    return name.ends_with(".flatpak");
    #[cfg(not(any(windows, target_os = "linux")))]
    return false;
}

/// 检查更新。结果写入 `state`：Found(tag, url) / UpToDate / Failed。
pub async fn check(state: &State) {
    *state.lock() = Stage::Checking;
    *state.lock() = match fetch().await {
        Ok(Some((tag, url))) => Stage::Found(tag, url),
        Ok(None) => Stage::UpToDate,
        Err(e) => Stage::Failed(e),
    };
}

async fn fetch() -> Result<Option<(String, String)>, String> {
    let resp = client()
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API HTTP {}", resp.status()));
    }
    let j: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let tag = j["tag_name"].as_str().unwrap_or_default().to_string();
    if tag.is_empty() || parse_ver(&tag).is_none_or(|t| t <= current()) {
        return Ok(None);
    }
    let asset = j["assets"].as_array().and_then(|a| {
        a.iter()
            .find(|a| a["name"].as_str().is_some_and(want_asset))
            .and_then(|a| a["browser_download_url"].as_str())
    });
    let page = j["html_url"].as_str();
    // 能自装的环境要资产直链；不能自装的（裸跑 Linux）要发布页。
    let url = (if can_self_install() { asset.or(page) } else { page.or(asset) })
        .unwrap_or_default()
        .to_string();
    Ok(Some((tag, url)))
}

/// 下载并安装（调用方保证 `can_self_install()`）。
pub async fn apply(state: &State, tag: String, url: String, dir: PathBuf) {
    *state.lock() = Stage::Downloading(0);
    let suffix = if cfg!(windows) { ".exe" } else { ".flatpak" };
    let dest = dir.join(format!("gagadown-update-{tag}{suffix}"));
    if let Err(e) = download(&url, &dest, state).await {
        *state.lock() = Stage::Failed(e);
        return;
    }
    *state.lock() = Stage::Installing;
    *state.lock() = match install(&dest) {
        Ok(()) => Stage::Installed,
        Err(e) => Stage::Failed(e),
    };
}

async fn download(url: &str, dest: &Path, state: &State) -> Result<(), String> {
    use futures_util::StreamExt;
    let resp = client().get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("下载 HTTP {}", resp.status()));
    }
    let total = resp.content_length();
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut got = 0u64;
    use std::io::Write;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        got += chunk.len() as u64;
        if let Some(t) = total {
            *state.lock() = Stage::Downloading(((got * 100) / t.max(1)).min(99) as u8);
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn install(path: &Path) -> Result<(), String> {
    let st = std::process::Command::new("flatpak-spawn")
        .args(["--host", "flatpak", "install", "--user", "--or-update", "--noninteractive"])
        .arg(path)
        .status()
        .map_err(|e| format!("flatpak-spawn: {e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("flatpak install 退出码 {st}"))
    }
}

#[cfg(windows)]
fn install(path: &Path) -> Result<(), String> {
    // NSIS 用 user 权限装到 %LOCALAPPDATA%，不需要 UAC；
    // 安装器开始复制前会 taskkill 本进程，直接先退。
    std::process::Command::new(path).spawn().map_err(|e| e.to_string())?;
    std::process::exit(0);
}

/// Flatpak：host 上重新拉起新版（等一拍让单实例锁释放），然后退出。
#[cfg(target_os = "linux")]
pub fn restart() -> ! {
    let _ = std::process::Command::new("flatpak-spawn")
        .args(["--host", "sh", "-c", &format!("sleep 1; exec flatpak run {APP_ID}")])
        .spawn();
    std::process::exit(0);
}
