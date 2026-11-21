//! 内置更新器：查 GitHub Releases 新版 → 引擎多段下载对应平台资产 → 就地安装。
//! Flatpak 走 `flatpak-spawn --host flatpak install`（sandbox 里没有 flatpak）；
//! Windows 直接拉起 NSIS 安装器（无 UAC，安装器自己 taskkill 旧进程）；
//! 裸跑 Linux 只提示去发布页。
//! Dev Mode（GAGADOWN_DEV=1）下 check/apply 走本地 mock 闭环，不触网不安装。

use crate::strings::tr;
use gagadown_core::engine::Engine;
use gagadown_core::task::{AddRequest, RemoveMode, Status};
use parking_lot::Mutex;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 上游发布仓库；release 资产名约定见 .github/workflows/release.yml。
pub const REPO: &str = "shuakami/gagadown";
pub const APP_ID: &str = "io.github.shuakami.GaGaDown";

#[derive(Clone, Debug)]
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

/// mock 更新流程是否激活过：Dev Mode 里 Installed 的"重启"要回到检查循环而不是真的退出。
static MOCK_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn mock_active() -> bool {
    MOCK_ACTIVE.load(Ordering::Relaxed) && crate::dev_mode()
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
/// Dev Mode 下走 mock：1.2s 后报告一个不存在的假版本。
pub async fn check(state: &State) {
    if crate::dev_mode() {
        MOCK_ACTIVE.store(true, Ordering::Relaxed);
        mock_check(state).await;
        return;
    }
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

/// 下载并安装（调用方保证 `can_self_install()`）。下载交给下载引擎——
/// 多段并发、线路竞速、限速与断点续传全部生效，任务短暂出现在下载列表里。
/// `mock://` URL 走 Dev Mode 假流程，不碰引擎。
pub async fn apply(state: &State, engine: &Engine, tag: String, url: String) {
    if url.starts_with("mock://") {
        mock_apply(state).await;
        return;
    }
    *state.lock() = Stage::Downloading(0);
    let suffix = if cfg!(windows) { ".exe" } else { ".flatpak" };
    let dir = engine.data_dir().join("updates");
    let filename = format!("gagadown-update-{tag}{suffix}");
    // 清掉历史版本的残留安装包；.fdpart 续传文件保留（同名任务续下要用）。
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for f in rd.flatten() {
            let n = f.file_name().to_string_lossy().to_string();
            if n.starts_with("gagadown-update-") && n != filename && !n.ends_with(".fdpart") {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
    let req = AddRequest {
        url,
        filename: Some(filename),
        dir: Some(dir),
        source: Some("updater".into()),
        ..Default::default()
    };
    let id = match engine.add(req, None) {
        Ok(o) => o.id,
        Err(e) => {
            *state.lock() = Stage::Failed(e.to_string());
            return;
        }
    };
    let path = loop {
        let view = engine.views(0).into_iter().find(|v| v.id == id);
        match view {
            // 任务被用户从列表里删了。
            None => {
                *state.lock() = Stage::Failed(tr("更新下载已取消").into());
                return;
            }
            Some(v) => match v.status {
                Status::Completed => break v.path,
                // Failed + next_retry_at 说明引擎正在自动重试，继续等。
                Status::Failed if v.next_retry_at.is_none() => {
                    let msg = v.error.map(|e| e.message).unwrap_or_default();
                    *state.lock() = Stage::Failed(msg);
                    return;
                }
                _ => {
                    if let Some(size) = v.size.filter(|s| *s > 0) {
                        let pct = ((v.downloaded * 100) / size).min(99) as u8;
                        *state.lock() = Stage::Downloading(pct);
                    }
                }
            },
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    let Some(path) = path else {
        *state.lock() = Stage::Failed(tr("更新文件缺失").into());
        return;
    };
    *state.lock() = Stage::Installing;
    *state.lock() = match install(&path) {
        Ok(()) => Stage::Installed,
        Err(e) => Stage::Failed(e),
    };
    // Windows 在 install() 里直接退出了，到不了这里；Flatpak 装完清理任务和安装包。
    engine.remove(id, RemoveMode::DeleteFiles).await;
}

async fn mock_check(state: &State) {
    *state.lock() = Stage::Checking;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    *state.lock() = Stage::Found("v99.0.0".into(), "mock://update".into());
}

async fn mock_apply(state: &State) {
    for p in 0..=100u8 {
        *state.lock() = Stage::Downloading(p);
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
    *state.lock() = Stage::Installing;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    *state.lock() = Stage::Installed;
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
