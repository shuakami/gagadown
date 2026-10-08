#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Margin, Painter, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui,
    ViewportCommand, pos2, vec2,
};
use egui_phosphor::regular as ic;
use std::sync::atomic::{AtomicBool, Ordering};
use gagadown_core::config::{ProxyMode, Settings};
use gagadown_core::engine::{CacheKind, CacheReport, Engine, PopupEvent};
use gagadown_core::task::{now_secs, AddRequest, DeletedRecord, Phase, RemoveMode, Status, TaskView};
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

static ICON: &[u8] = include_bytes!("../../../assets/icon_256.rgba");
const DEFAULT_ACCENT: Color32 = Color32::from_rgb(0, 120, 212);

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 * t + y as f32 * (1.0 - t)).round() as u8;
    rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

#[derive(Clone, Copy)]
struct Pal {
    dark: bool,
    chrome: Color32,
    editor: Color32,
    line: Color32,
    hover: Color32,
    press: Color32,
    sel: Color32,
    text: Color32,
    weak: Color32,
    accent: Color32,
    track: Color32,
    input: Color32,
    green: Color32,
    red: Color32,
    orange: Color32,
}

impl Pal {
    fn new(dark: bool, accent: Color32) -> Self {
        if dark {
            Pal {
                dark,
                chrome: rgb(24, 24, 24),
                editor: rgb(31, 31, 31),
                line: rgb(43, 43, 43),
                hover: rgb(42, 45, 46),
                press: rgb(58, 61, 65),
                sel: blend(accent, rgb(31, 31, 31), 0.30),
                text: rgb(212, 212, 212),
                weak: rgb(145, 145, 145),
                accent,
                track: rgb(58, 58, 60),
                input: rgb(43, 43, 43),
                green: rgb(87, 190, 122),
                red: rgb(241, 76, 76),
                orange: rgb(224, 165, 75),
            }
        } else {
            Pal {
                dark,
                chrome: rgb(248, 248, 248),
                editor: rgb(255, 255, 255),
                line: rgb(229, 229, 229),
                hover: rgb(242, 242, 242),
                press: rgb(226, 226, 226),
                sel: blend(accent, rgb(255, 255, 255), 0.16),
                text: rgb(31, 31, 31),
                weak: rgb(110, 110, 110),
                accent,
                track: rgb(226, 226, 226),
                input: rgb(240, 240, 240),
                green: rgb(38, 150, 88),
                red: rgb(205, 49, 49),
                orange: rgb(190, 120, 20),
            }
        }
    }

    fn status(&self, v: &TaskView) -> Color32 {
        match v.status {
            Status::Running => self.accent,
            Status::Completed if v.file_missing => self.orange,
            Status::Completed => self.green,
            Status::Failed => self.red,
            _ => self.weak,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Downloads,
    Trash,
    Cache,
    Extension,
    Settings,
}

#[derive(Clone, Copy)]
enum Act {
    Pause,
    Resume,
    Redownload,
    Open,
    Reveal,
    CopyLink,
    Remove,
    ErrorInfo,
}

#[derive(Clone, Copy)]
enum Sec {
    Active,
    Failed,
    Done,
}

#[derive(Clone, Copy)]
enum Glyph {
    Min,
    Max,
    Restore,
    Close,
}

struct App {
    rt: tokio::runtime::Runtime,
    engine: Engine,
    page: Page,
    input: String,
    views: Vec<TaskView>,
    deleted: Vec<DeletedRecord>,
    last_refresh: Option<Instant>,
    selected: Option<Uuid>,
    removing: Option<Uuid>,
    err_report: Option<Uuid>,
    rm_fade: Fade,
    er_fade: Fade,
    browsers: Vec<Browser>,
    ext_guide: Option<ExtGuide>,
    ext_tab: usize,
    remove_mode: RemoveMode,
    draft: Settings,
    draft_proxies: String,
    cache: Option<CacheReport>,
    toasts: Vec<(Instant, String, bool)>,
    handoff_fails: Vec<Handoff>,
    api_error: Arc<Mutex<Option<String>>>,
    popups: Vec<Pop>,
    dismissed: std::collections::HashSet<Uuid>,
    tex: Option<egui::TextureHandle>,
    icon: Arc<egui::IconData>,
    accent: Color32,
    accent_checked: Instant,
    decorated: bool,
    settings_dirty: Option<(String, Instant)>,
    ficons: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    geo: Arc<Mutex<std::collections::HashMap<String, Option<Geo>>>>,
    flags: std::collections::HashMap<String, egui::TextureHandle>,
    tray: Option<Box<dyn std::any::Any>>,
    quit: Arc<AtomicBool>,
    hidden: Arc<AtomicBool>,
}

/// A download popup: `task` is set once the hand-off probe accepted the link.
#[derive(Clone)]
struct Pop {
    key: Uuid,
    task: Option<Uuid>,
    name: String,
    size: Option<u64>,
    host: String,
    pos: Pos2,
    /// Hand-off failed; the browser keeps the download.
    error: Option<String>,
}

#[derive(Clone)]
struct Handoff {
    at: u64,
    name: String,
    reason: String,
}

fn bytes(n: u64) -> String {
    let n = n as f64;
    let k = 1024.0;
    if n < k {
        format!("{n:.0} B")
    } else if n < k * k {
        format!("{:.1} KB", n / k)
    } else if n < k * k * k {
        format!("{:.1} MB", n / k / k)
    } else {
        format!("{:.2} GB", n / k / k / k)
    }
}

fn speed(s: f64) -> String {
    format!("{}/s", bytes(s.max(0.0) as u64))
}

fn dur(s: u64) -> String {
    if s < 60 {
        format!("{s} 秒")
    } else if s < 3600 {
        format!("{} 分 {} 秒", s / 60, s % 60)
    } else {
        format!("{} 小时 {} 分", s / 3600, s % 3600 / 60)
    }
}

fn ago(ts: u64) -> String {
    let d = now_secs().saturating_sub(ts);
    if d < 60 {
        "刚刚".into()
    } else if d < 3600 {
        format!("{} 分钟前", d / 60)
    } else if d < 86400 {
        format!("{} 小时前", d / 3600)
    } else {
        format!("{} 天前", d / 86400)
    }
}

/// Plain-text error report; query strings are masked since they often carry tokens.
fn error_report(v: &TaskView, e: &gagadown_core::task::TaskError) -> String {
    use std::fmt::Write;
    let masked = match v.url.split_once('?') {
        Some((a, _)) => format!("{a}?…"),
        None => v.url.clone(),
    };
    let scrub = |s: &str| s.replace(&v.url, &masked);
    let mut s = String::new();
    let _ = writeln!(s, "文件    {}", v.filename);
    let _ = writeln!(s, "链接    {masked}");
    let _ = writeln!(s, "时间    {}", when(e.at));
    let _ = writeln!(s, "类型    {:?}", e.kind);
    if let Some(c) = e.status {
        let _ = writeln!(s, "状态码  HTTP {c}");
    }
    let _ = writeln!(s, "原始信息  {}", scrub(&e.message));
    if !e.routes.is_empty() {
        s.push_str("\n各线路结果\n");
        for r in &e.routes {
            match r.status {
                Some(c) => {
                    let _ = writeln!(s, "  {}  HTTP {c}", r.route);
                }
                None => {
                    let why = gagadown_core::error::brief(&r.message).unwrap_or("无响应");
                    let _ = writeln!(s, "  {}  {why}\n    {}", r.route, scrub(&r.message));
                }
            }
        }
    }
    if !v.log.is_empty() {
        s.push_str("\n最近日志\n");
        for l in v.log.iter().rev().take(12).rev() {
            let _ = writeln!(s, "  {}  {}", when(l.at), scrub(&l.text));
        }
    }
    let _ = write!(s, "\nGaGaDown v{}", env!("CARGO_PKG_VERSION"));
    s
}

/// Open/close fade for a modal that has to stay on screen while it animates out.
#[derive(Default)]
struct Fade {
    t0: f64,
    closing: bool,
}

impl Fade {
    /// Opacity for this frame; `None` once the close animation has finished.
    fn step(&mut self, ctx: &egui::Context) -> Option<f32> {
        let now = ctx.input(|i| i.time);
        if self.t0 == 0.0 {
            self.t0 = now;
        }
        let k = ((now - self.t0) / 0.16).clamp(0.0, 1.0) as f32;
        if k < 1.0 {
            ctx.request_repaint();
        }
        if !self.closing {
            return Some(ease_out_cubic(k));
        }
        if k >= 1.0 {
            *self = Fade::default();
            return None;
        }
        Some(1.0 - ease_out_cubic(k))
    }

    fn close(&mut self, ctx: &egui::Context) {
        if !self.closing {
            self.closing = true;
            self.t0 = ctx.input(|i| i.time);
        }
    }
}

fn when(ts: u64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(t) = Local.timestamp_opt(ts as i64, 0).single() else { return String::new() };
    let now = Local::now();
    let days = now.date_naive().signed_duration_since(t.date_naive()).num_days();
    match days {
        0 => t.format("今天 %H:%M").to_string(),
        1 => t.format("昨天 %H:%M").to_string(),
        _ if t.year() == now.year() => format!("{}月{}日 {}", t.month(), t.day(), t.format("%H:%M")),
        _ => format!("{}年{}月{}日", t.year(), t.month(), t.day()),
    }
}

fn clock(ts: u64) -> String {
    use chrono::{Local, TimeZone};
    Local.timestamp_opt(ts as i64, 0).single().map(|t| t.format("%H:%M:%S").to_string()).unwrap_or_default()
}

#[derive(Clone, Default)]
struct Geo {
    ip: String,
    cc: String,
    flag: Option<egui::ColorImage>,
}

async fn ip_api(c: &reqwest::Client, host: &str) -> Option<(String, String)> {
    let j: serde_json::Value = c.get(format!("http://ip-api.com/json/{host}?fields=status,countryCode,query")).send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
    if j["status"].as_str() != Some("success") {
        return None;
    }
    Some((j["query"].as_str()?.to_string(), j["countryCode"].as_str()?.to_ascii_lowercase()))
}

/// uapis.cn standard source; key is optional (UAPI_KEY env var).
async fn uapi(c: &reqwest::Client, host: &str) -> Option<(String, String)> {
    let mut req = c.get("https://uapis.cn/api/v1/network/ipinfo").query(&[("ip", host)]);
    if let Ok(k) = std::env::var("UAPI_KEY") {
        if k.starts_with("uapi-") {
            req = req.bearer_auth(k);
        }
    }
    let r = req.send().await.ok()?;
    if !r.status().is_success() {
        tracing::debug!("uapis ipinfo {host}: {}", r.status());
        return None;
    }
    let j: serde_json::Value = r.json().await.ok()?;
    let ip = j["ip"].as_str()?.to_string();
    let country = j["region"].as_str().unwrap_or_default().split_whitespace().next().unwrap_or_default();
    Some((ip, country_code(country).unwrap_or_default().to_string()))
}

fn country_code(name: &str) -> Option<&'static str> {
    const T: &[(&str, &str)] = &[
        ("中国", "cn"), ("香港", "hk"), ("中国香港", "hk"), ("澳门", "mo"), ("中国澳门", "mo"), ("台湾", "tw"), ("中国台湾", "tw"),
        ("美国", "us"), ("日本", "jp"), ("韩国", "kr"), ("新加坡", "sg"), ("英国", "gb"), ("德国", "de"), ("法国", "fr"),
        ("荷兰", "nl"), ("加拿大", "ca"), ("澳大利亚", "au"), ("俄罗斯", "ru"), ("印度", "in"), ("爱尔兰", "ie"), ("瑞典", "se"),
        ("瑞士", "ch"), ("芬兰", "fi"), ("挪威", "no"), ("丹麦", "dk"), ("意大利", "it"), ("西班牙", "es"), ("葡萄牙", "pt"),
        ("波兰", "pl"), ("比利时", "be"), ("奥地利", "at"), ("捷克", "cz"), ("乌克兰", "ua"), ("土耳其", "tr"), ("以色列", "il"),
        ("阿联酋", "ae"), ("沙特阿拉伯", "sa"), ("巴西", "br"), ("墨西哥", "mx"), ("阿根廷", "ar"), ("智利", "cl"), ("南非", "za"),
        ("新西兰", "nz"), ("马来西亚", "my"), ("泰国", "th"), ("越南", "vn"), ("印度尼西亚", "id"), ("菲律宾", "ph"), ("卢森堡", "lu"),
        ("罗马尼亚", "ro"), ("匈牙利", "hu"), ("希腊", "gr"), ("保加利亚", "bg"), ("立陶宛", "lt"), ("拉脱维亚", "lv"), ("爱沙尼亚", "ee"),
        ("冰岛", "is"), ("哈萨克斯坦", "kz"), ("巴基斯坦", "pk"), ("孟加拉", "bd"), ("埃及", "eg"), ("尼日利亚", "ng"), ("肯尼亚", "ke"),
    ];
    T.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

async fn lookup_geo(host: &str) -> Option<Geo> {
    let c = reqwest::Client::builder().timeout(Duration::from_secs(8)).build().ok()?;
    let (ip, cc) = match ip_api(&c, host).await {
        Some(x) => x,
        None => uapi(&c, host).await?,
    };
    let mut g = Geo { ip, cc, flag: None };
    if g.cc.len() == 2 {
        if let Ok(r) = c.get(format!("https://flagcdn.com/w40/{}.png", g.cc)).send().await {
            if let Ok(b) = r.bytes().await {
                g.flag = decode_png(&b);
            }
        }
    }
    Some(g)
}

fn decode_png(b: &[u8]) -> Option<egui::ColorImage> {
    let mut d = png::Decoder::new(std::io::Cursor::new(b));
    d.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        _ => return None,
    };
    Some(egui::ColorImage::from_rgba_unmultiplied([info.width as usize, info.height as usize], &rgba))
}

fn route_info(v: &TaskView) -> (Option<String>, Option<u64>) {
    for l in v.log.iter().rev() {
        if let Some(rest) = l.text.strip_prefix("线路 ") {
            if let Some((name, ms)) = rest.split_once("，响应 ") {
                return (v.route.clone().or(Some(name.to_string())), ms.trim_end_matches(" ms").parse().ok());
            }
        }
    }
    (v.route.clone(), None)
}

const EXTENSION: &[(&str, &[u8])] = &[
    ("manifest.json", include_bytes!("../../../extension/manifest.json")),
    ("background.js", include_bytes!("../../../extension/background.js")),
    ("popup.html", include_bytes!("../../../extension/popup.html")),
    ("popup.js", include_bytes!("../../../extension/popup.js")),
    ("_locales/zh_CN/messages.json", include_bytes!("../../../extension/_locales/zh_CN/messages.json")),
    ("_locales/en/messages.json", include_bytes!("../../../extension/_locales/en/messages.json")),
    ("icons/16.png", include_bytes!("../../../extension/icons/16.png")),
    ("icons/32.png", include_bytes!("../../../extension/icons/32.png")),
    ("icons/48.png", include_bytes!("../../../extension/icons/48.png")),
    ("icons/128.png", include_bytes!("../../../extension/icons/128.png")),
];

/// Store listing IDs. Once set, installing goes through the browser's own
/// external-extension prompt instead of loading the unpacked folder.
const CHROME_STORE_ID: Option<&str> = None;
const EDGE_STORE_ID: Option<&str> = None;

struct ExtGuide {
    browser: usize,
    since: u64,
    dir: std::path::PathBuf,
}

#[derive(Clone)]
struct Browser {
    key: &'static str,
    name: &'static str,
    exe: std::path::PathBuf,
    page: &'static str,
    reg: &'static str,
    store_id: Option<&'static str>,
    update_url: &'static str,
    store_page: &'static str,
}

fn browsers() -> Vec<Browser> {
    [
        ("chrome", "Google Chrome", "chrome.exe", "chrome://extensions", r"Software\Google\Chrome\Extensions", CHROME_STORE_ID, "https://clients2.google.com/service/update2/crx", "https://chromewebstore.google.com/detail/"),
        ("edge", "Microsoft Edge", "msedge.exe", "edge://extensions", r"Software\Microsoft\Edge\Extensions", EDGE_STORE_ID, "https://edge.microsoft.com/extensionwebstorebase/v1/crx", "https://microsoftedge.microsoft.com/addons/detail/"),
    ]
    .into_iter()
    .filter_map(|(key, name, exe, page, reg, store_id, update_url, store_page)| Some(Browser { key, name, exe: win::app_path(exe)?, page, reg, store_id, update_url, store_page }))
    .collect()
}

/// Writes the bundled extension into `<data>/extension`, touching only files that changed.
fn unpack_extension(data: &Path) -> std::io::Result<std::path::PathBuf> {
    let root = data.join("extension");
    for (name, bytes) in EXTENSION {
        let p = root.join(name);
        if std::fs::read(&p).ok().as_deref() != Some(*bytes) {
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&p, bytes)?;
        }
    }
    Ok(root)
}

fn open_path(p: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let f: Vec<u16> = p.as_os_str().encode_wide().chain([0]).collect();
        let n = std::ptr::null();
        unsafe { ShellExecuteW(std::ptr::null_mut(), n, f.as_ptr(), n, n, SW_SHOWNORMAL) };
    }
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

fn reveal(p: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if p.exists() {
            let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn();
        } else if let Some(d) = p.parent() {
            open_path(d);
        }
    }
    #[cfg(not(windows))]
    if let Some(d) = if p.is_dir() { Some(p) } else { p.parent() } {
        open_path(d);
    }
}

fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

fn ifont(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Single-line text clipped with an ellipsis; `anchor` picks which edge `pos` is.
fn done_path(v: &TaskView) -> Option<&std::path::Path> {
    if v.status == Status::Completed && !v.file_missing { v.path.as_deref() } else { None }
}

fn text_line(pt: &Painter, pos: Pos2, anchor: Align2, text: impl Into<String>, f: FontId, color: Color32, max_w: f32) -> Rect {
    let mut job = egui::text::LayoutJob::simple_singleline(text.into(), f, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(8.0));
    let g = pt.layout_job(job);
    let r = anchor.anchor_size(pos, g.size());
    let ppp = pt.pixels_per_point();
    pt.galley(pos2((r.min.x * ppp).round() / ppp, (r.min.y * ppp).round() / ppp), g, color);
    r
}

/// `text_line` with case-insensitive matches of `q` tinted (search highlight).
#[allow(clippy::too_many_arguments)]
fn text_line_hl(pt: &Painter, pos: Pos2, anchor: Align2, text: &str, q: &str, f: FontId, color: Color32, p: &Pal, max_w: f32) -> Rect {
    let lc = |c: char| c.to_lowercase().next().unwrap_or(c);
    let q: Vec<char> = q.chars().map(lc).collect();
    let chars: Vec<(usize, char)> = text.char_indices().map(|(i, c)| (i, lc(c))).collect();
    let mut ranges = Vec::new();
    if !q.is_empty() {
        let mut i = 0;
        while i + q.len() <= chars.len() {
            if chars[i..i + q.len()].iter().map(|x| x.1).eq(q.iter().copied()) {
                let end = chars.get(i + q.len()).map_or(text.len(), |x| x.0);
                ranges.push(chars[i].0..end);
                i += q.len();
            } else {
                i += 1;
            }
        }
    }
    if ranges.is_empty() {
        return text_line(pt, pos, anchor, text, f, color, max_w);
    }
    let plain = egui::TextFormat { font_id: f.clone(), color, ..Default::default() };
    let hit = egui::TextFormat { font_id: f, color: p.accent, background: p.accent.gamma_multiply(0.16), ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    let mut at = 0;
    for r in ranges {
        job.append(&text[at..r.start], 0.0, plain.clone());
        job.append(&text[r.clone()], 0.0, hit.clone());
        at = r.end;
    }
    job.append(&text[at..], 0.0, plain);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(8.0));
    let g = pt.layout_job(job);
    let r = anchor.anchor_size(pos, g.size());
    let ppp = pt.pixels_per_point();
    pt.galley(pos2((r.min.x * ppp).round() / ppp, (r.min.y * ppp).round() / ppp), g, color);
    r
}

fn file_glyph(name: &str, p: &Pal) -> (&'static str, Color32) {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "zst" | "cab" => (ic::FILE_ZIP, rgb(214, 160, 60)),
        "exe" | "msi" | "msix" | "appx" | "apk" | "dmg" | "pkg" | "deb" | "rpm" | "appimage" => (ic::APP_WINDOW, p.accent),
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" | "ts" => (ic::FILE_VIDEO, rgb(160, 120, 230)),
        "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" | "opus" | "ape" => (ic::FILE_AUDIO, rgb(225, 105, 155)),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif" | "heic" | "ico" => (ic::FILE_IMAGE, rgb(70, 175, 115)),
        "pdf" => (ic::FILE_PDF, rgb(222, 80, 80)),
        "iso" | "img" | "vhd" | "vhdx" | "esd" | "wim" => (ic::DISC, rgb(60, 165, 190)),
        "txt" | "md" | "log" | "csv" | "json" | "xml" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" => (ic::FILE_TEXT, p.weak),
        "js" | "py" | "rs" | "go" | "c" | "cpp" | "h" | "java" | "sh" | "ps1" | "bat" => (ic::FILE_CODE, p.weak),
        _ => (ic::FILE, p.weak),
    }
}

fn frac(v: &TaskView) -> f32 {
    match (v.status, v.size) {
        (Status::Completed, _) => 1.0,
        (_, Some(s)) if s > 0 => (v.downloaded as f64 / s as f64).clamp(0.0, 1.0) as f32,
        _ => 0.0,
    }
}

fn of(v: &TaskView) -> String {
    match v.size {
        Some(s) => format!("{} / {}", bytes(v.downloaded), bytes(s)),
        None => bytes(v.downloaded),
    }
}

fn meta_line(v: &TaskView, p: &Pal) -> (String, Color32) {
    match v.status {
        Status::Queued if v.downloaded > 0 => (format!("排队中   {}", of(v)), p.weak),
        Status::Queued => ("排队中".into(), p.weak),
        Status::Paused => (format!("已暂停   {}", of(v)), p.weak),
        Status::Running => match v.phase {
            Some(Phase::Downloading) | None => {
                let mut s = format!("{}   {}", of(v), speed(v.speed));
                if let Some(e) = v.eta_secs {
                    s += &format!("   剩余 {}", dur(e));
                }
                (s, p.weak)
            }
            Some(Phase::Connecting) => ("连接中".into(), p.weak),
            Some(Phase::Finalizing) => ("写入中".into(), p.weak),
            Some(Phase::Verifying) => ("校验中".into(), p.weak),
        },
        Status::Completed if v.file_missing => ("文件已被移动或删除".into(), p.orange),
        Status::Completed => {
            let mut s = v.size.map(bytes).unwrap_or_default();
            if v.avg_speed > 0.0 {
                s += &format!("   平均 {}", speed(v.avg_speed));
            }
            (s, p.weak)
        }
        Status::Failed => {
            let mut s = match &v.error {
                Some(e) => e.summary(),
                None => "失败".into(),
            };
            if let Some(t) = v.next_retry_at {
                s += &format!("，{} 后自动重试", dur(t.saturating_sub(now_secs())));
            }
            (s, p.red)
        }
    }
}

/// Critically damped spring toward `target` (stiffness `w` 1/s), frame-rate independent.
/// Starts at `init` the first time `id` is seen; repaints until settled.
fn spring(ctx: &egui::Context, id: Id, target: f32, w: f32, init: f32) -> f32 {
    let now = ctx.input(|i| i.time);
    let (x, v) = ctx.data_mut(|d| {
        let s = d.get_temp_mut_or(id, (init, 0.0f32, now));
        let dt = (now - s.2) as f32;
        s.2 = now;
        if dt > 0.5 || !s.0.is_finite() {
            *s = (target, 0.0, now);
        } else if dt > 0.0 {
            let d0 = s.0 - target;
            let e = (-w * dt).exp();
            let k = (s.1 + w * d0) * dt;
            s.0 = target + (d0 + k) * e;
            s.1 = (s.1 - w * k) * e;
        }
        let eps = (target.abs() * 1e-3).max(1e-4);
        if (s.0 - target).abs() < eps && s.1.abs() < eps * w {
            s.0 = target;
            s.1 = 0.0;
        }
        (s.0, s.1)
    });
    if x != target || v != 0.0 {
        ctx.request_repaint();
    }
    x
}

/// Seconds since `on` turned true; `None` while false or if it was already true when first seen.
fn since_on(ctx: &egui::Context, id: Id, on: bool) -> Option<f32> {
    let now = ctx.input(|i| i.time);
    let at = ctx.data_mut(|d| {
        let s = d.get_temp_mut_or(id, (on, f64::NEG_INFINITY));
        if on && !s.0 {
            s.1 = now;
        }
        s.0 = on;
        s.1
    });
    (on && at.is_finite()).then_some((now - at) as f32)
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0) - 1.0;
    let c1 = 1.70158;
    1.0 + (c1 + 1.0) * t * t * t + c1 * t * t
}

/// Soft white highlight centered at `cx`; clip with the painter.
fn glow_band(pt: &Painter, r: Rect, cx: f32, half: f32, a: f32) {
    if a <= 0.0 || half <= 0.0 {
        return;
    }
    let mut m = egui::Mesh::default();
    let n = 9;
    for i in 0..n {
        let u = i as f32 / (n - 1) as f32 * 2.0 - 1.0;
        let c = Color32::from_white_alpha((a * (1.0 - u * u).powi(2) * 255.0) as u8);
        m.colored_vertex(pos2(cx + u * half, r.top()), c);
        m.colored_vertex(pos2(cx + u * half, r.bottom()), c);
    }
    for i in 0..(n as u32 - 1) {
        let b = i * 2;
        m.add_triangle(b, b + 1, b + 2);
        m.add_triangle(b + 1, b + 2, b + 3);
    }
    pt.add(Shape::mesh(m));
}

fn indeterminate(pt: &Painter, r: Rect, time: f64, col: Color32) {
    let ph = ease_in_out_cubic((time / 1.4).fract() as f32);
    let w = r.width() * 0.32;
    let x = r.left() - w + ph * (r.width() + w);
    let seg = Rect::from_min_max(pos2(x.max(r.left()), r.top()), pos2((x + w).min(r.right()), r.bottom()));
    if seg.width() > 0.0 {
        pt.rect_filled(seg, CornerRadius::same((r.height() / 2.0).min(3.0) as u8), col);
    }
}

/// Animated progress bar; returns the eased fraction for the percent label.
fn seg_bar(ctx: &egui::Context, pt: &Painter, r: Rect, v: &TaskView, p: &Pal, key: Id) -> f32 {
    let cr = CornerRadius::same((r.height() / 2.0).min(3.0) as u8);
    pt.rect_filled(r, cr, p.track);
    let time = ctx.input(|i| i.time);
    let done = v.status == Status::Completed;
    let f = spring(ctx, key.with("f"), if done { 1.0 } else { frac(v) }, 9.0, 0.0).clamp(0.0, 1.0);
    if let Some(t) = since_on(ctx, key.with("done"), done && !v.file_missing) {
        // Fill runs out to the end, the bar eases from accent into green, one bright pass.
        let fill = Rect::from_min_size(r.min, vec2(r.width() * f, r.height()));
        pt.rect_filled(fill, cr, blend(p.green, p.accent, ease_in_out_cubic((t - 0.15) / 0.55)));
        let g = (t - 0.2) / 0.75;
        if (0.0..1.0).contains(&g) {
            let cx = r.left() - 40.0 + ease_in_out_cubic(g) * (r.width() + 80.0);
            glow_band(&pt.with_clip_rect(fill), r, cx, 40.0, 0.6 * (1.0 - g));
        }
        if t < 1.2 {
            ctx.request_repaint();
        }
        return f;
    }
    let col = p.status(v);
    if done {
        pt.rect_filled(r, cr, col);
        return 1.0;
    }
    let running = v.status == Status::Running;
    let size = v.size.unwrap_or(0);
    if size == 0 {
        if running {
            indeterminate(pt, r, time, col);
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        return 0.0;
    }
    let mut filled = Vec::new();
    if v.segments.is_empty() {
        let fill = Rect::from_min_size(r.min, vec2(r.width() * f, r.height()));
        if fill.width() > 0.5 {
            pt.rect_filled(fill, cr, col);
            filled.push(fill);
        }
    } else {
        let shape = pt.with_clip_rect(r);
        let x = |u: f32| r.left() + u.clamp(0.0, 1.0) * r.width();
        for s in &v.segments {
            if s.done <= s.start {
                continue;
            }
            let a = s.start as f32 / size as f32;
            let b = spring(ctx, key.with(("s", s.start)), s.done as f32 / size as f32, 9.0, a);
            let seg = Rect::from_x_y_ranges(x(a)..=x(b).max(x(a) + 1.0), r.y_range());
            shape.rect_filled(seg, 0.0, if s.active { col } else { col.gamma_multiply(0.8) });
            filled.push(seg);
        }
    }
    if running {
        let ph = ((time % 2.4) / 1.5) as f32;
        if ph < 1.0 {
            let half = (r.width() * 0.12).clamp(24.0, 60.0);
            let cx = r.left() - half + ease_in_out_cubic(ph) * (r.width() * f + 2.0 * half);
            for fr in &filled {
                glow_band(&pt.with_clip_rect(*fr), *fr, cx, half, 0.3);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }
    f
}

fn speed_graph(ctx: &egui::Context, pt: &Painter, r: Rect, hist: &[f32], p: &Pal, key: Id) {
    let n = 60usize;
    let data = &hist[hist.len().saturating_sub(n)..];
    pt.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.line));
    if data.is_empty() {
        return;
    }
    let now = ctx.input(|i| i.time);
    // Each new sample slides the whole chart one slot left while the curve extends to it.
    let sig = (hist.len(), data[data.len() - 1].to_bits());
    let since = ctx.data_mut(|d| {
        let s = d.get_temp_mut_or(key.with("tick"), (sig, f64::NEG_INFINITY));
        if s.0 != sig {
            *s = (sig, now);
        }
        s.1
    });
    let t = ((now - since) / 0.7) as f32;
    let (slide, grow) = if t < 1.0 {
        ctx.request_repaint();
        (1.0 - ease_out_cubic(t), ease_out_cubic(t))
    } else {
        (0.0, 1.0)
    };
    let peak = data.iter().cloned().fold(1.0f32, f32::max);
    let max = spring(ctx, key, peak * 1.15, 6.0, peak * 1.15).max(1.0);
    let bw = r.width() / n as f32;
    let off = n - data.len();
    let last = data.len() - 1;
    let clip = pt.with_clip_rect(r);
    let mut pts = Vec::with_capacity(data.len());
    let mut e = 0.0f32;
    for (i, s) in data.iter().enumerate() {
        let x = r.left() + (off + i) as f32 * bw + slide * bw;
        let h = (s / max * r.height()).min(r.height());
        let a = if i == last { 0.8 } else { 0.5 };
        clip.rect_filled(Rect::from_min_max(pos2(x + 1.0, r.bottom() - h), pos2(x + bw - 1.0, r.bottom())), CornerRadius::same(1), p.accent.gamma_multiply(a));
        e = if i == 0 { *s } else { e * 0.7 + s * 0.3 };
        pts.push(pos2(x + bw / 2.0, r.bottom() - (e / max * r.height()).min(r.height())));
    }
    if pts.len() > 1 {
        let prev = pts[last - 1];
        pts[last] = prev + (pts[last] - prev) * grow;
        let mut m = egui::Mesh::default();
        for q in &pts {
            m.colored_vertex(*q, p.green.gamma_multiply(0.22));
            m.colored_vertex(pos2(q.x, r.bottom()), Color32::TRANSPARENT);
        }
        for i in 0..(pts.len() as u32 - 1) {
            let b = i * 2;
            m.add_triangle(b, b + 1, b + 2);
            m.add_triangle(b + 1, b + 2, b + 3);
        }
        clip.add(Shape::mesh(m));
        clip.add(Shape::line(pts, Stroke::new(1.6, p.green)));
    }
    pt.text(r.right_top(), Align2::RIGHT_TOP, speed((max / 1.15) as f64), font(10.5), p.weak);
}

/// Writes `crash.log` on panic and `freeze.log` when the UI thread is stuck in one stage.
fn install_diagnostics(dir: std::path::PathBuf) {
    fn append(path: &Path, line: &str) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(line.as_bytes());
        }
    }
    let crash = dir.join("crash.log");
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let bt = std::backtrace::Backtrace::force_capture();
        append(&crash, &format!("[{}] thread '{thread}' {info}\n{bt}\n\n", now_secs()));
        prev(info);
    }));
    let freeze = dir.join("freeze.log");
    let _ = std::thread::Builder::new().name("watchdog".into()).spawn(move || {
        let mut seen = (0u32, 0u64);
        let mut tier = 0usize;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let (stage, since) = eframe::stall::current();
            if stage == 0 || since == 0 {
                continue;
            }
            if (stage, since) != seen {
                seen = (stage, since);
                tier = 0;
            }
            let secs = eframe::stall::now_ms().saturating_sub(since) / 1000;
            let t = [2, 10, 30].iter().filter(|&&x| secs >= x).count();
            if t > tier {
                tier = t;
                append(&freeze, &format!("[{}] 界面线程卡在「{}」已 {secs} 秒\n", now_secs(), eframe::stall::name(stage)));
            }
        }
    });
}

fn chrome_btn(ui: &mut Ui, r: Rect, g: Glyph, p: &Pal) -> bool {
    let resp = ui.interact(r, ui.id().with(("chrome", g as u8)), Sense::click());
    let close = matches!(g, Glyph::Close);
    let pt = ui.painter();
    if resp.hovered() {
        pt.rect_filled(r, 0.0, if close { rgb(232, 17, 35) } else { p.hover });
    }
    let c = r.center().round() + vec2(0.5, 0.5);
    let s = Stroke::new(1.0, if close && resp.hovered() { Color32::WHITE } else { p.text });
    match g {
        Glyph::Min => {
            pt.line_segment([c + vec2(-5.0, 0.0), c + vec2(5.0, 0.0)], s);
        }
        Glyph::Max => {
            pt.rect_stroke(Rect::from_center_size(c, vec2(10.0, 10.0)), 0.0, s, StrokeKind::Middle);
        }
        Glyph::Restore => {
            pt.rect_stroke(Rect::from_center_size(c + vec2(-1.0, 1.0), vec2(8.0, 8.0)), 0.0, s, StrokeKind::Middle);
            pt.line_segment([c + vec2(-3.0, -4.0), c + vec2(5.0, -4.0)], s);
            pt.line_segment([c + vec2(5.0, -4.0), c + vec2(5.0, 4.0)], s);
        }
        Glyph::Close => {
            pt.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], s);
            pt.line_segment([c + vec2(-5.0, 5.0), c + vec2(5.0, -5.0)], s);
        }
    }
    resp.clicked()
}

trait Tip {
    fn tip(self, text: impl Into<String>) -> Self;
}

impl Tip for egui::Response {
    /// Hover text with a little extra side padding.
    fn tip(self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.on_hover_ui(|ui| {
            egui::Frame::new().inner_margin(Margin::symmetric(4, 0)).show(ui, |ui| ui.label(text));
        })
    }
}

fn icon_at(ui: &mut Ui, r: Rect, id: Id, icon: &str, tip: &str, p: &Pal) -> bool {
    icon_btn(ui, r, id, icon, tip, p).clicked()
}

fn icon_btn(ui: &mut Ui, r: Rect, id: Id, icon: &str, tip: &str, p: &Pal) -> egui::Response {
    let resp = ui.interact(r, id, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), if resp.is_pointer_button_down_on() { p.press } else { p.text.gamma_multiply(0.09) });
    }
    ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, ifont(15.0), p.text);
    resp.tip(tip)
}

fn menu_item(ui: &mut Ui, label: &str, danger: bool, p: &Pal) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(140.0), 30.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(6), p.text.gamma_multiply(0.09));
    }
    ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, label, font(12.5), if danger { p.red } else { p.text });
    resp.clicked()
}

/// Dropdown entry styled like `menu_item`, with a check on the selected one.
fn menu_choice(ui: &mut Ui, label: &str, selected: bool, p: &Pal) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let pt = ui.painter();
    if resp.hovered() {
        pt.rect_filled(r, CornerRadius::same(6), p.text.gamma_multiply(0.09));
    }
    pt.text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, label, font(12.5), p.text);
    if selected {
        pt.text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, ic::CHECK, ifont(14.0), p.accent);
    }
    resp.clicked()
}

fn btn_at(ui: &mut Ui, r: Rect, id: Id, text: &str, primary: bool, p: &Pal) -> bool {
    let resp = ui.interact(r, id, Sense::click());
    let base = if primary { p.accent } else if p.dark { rgb(49, 49, 49) } else { rgb(229, 229, 229) };
    let fill = if resp.is_pointer_button_down_on() {
        blend(base, Color32::BLACK, 0.82)
    } else if resp.hovered() {
        blend(base, if primary { Color32::BLACK } else if p.dark { Color32::WHITE } else { Color32::BLACK }, 0.9)
    } else {
        base
    };
    ui.painter().rect_filled(r, CornerRadius::same(6), fill);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, text, font(12.5), if primary { Color32::WHITE } else { p.text });
    resp.clicked()
}

fn btn(ui: &mut Ui, text: &str, primary: bool, p: &Pal) -> bool {
    let w = ui.painter().layout_no_wrap(text.to_string(), font(12.5), p.text).size().x + 32.0;
    let (r, resp) = ui.allocate_exact_size(vec2(w.max(72.0), 30.0), Sense::hover());
    btn_at(ui, r, resp.id.with("b"), text, primary, p)
}

/// Section header in the VS Code explorer style. Returns (header clicked, action index clicked, action buttons).
fn header(ui: &mut Ui, title: &str, n: Option<usize>, caret: Option<bool>, actions: &[(&str, &str)], p: &Pal) -> (bool, Option<usize>, Vec<egui::Response>) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let pt = ui.painter().clone();
    if resp.hovered() && caret.is_some() {
        pt.rect_filled(r, 0.0, p.hover);
    }
    // Aligned with list rows: file icons are 26 px centered at left + 30, row buttons end at right - 14.
    let mut x = r.left() + 17.0;
    if let Some(open) = caret {
        pt.text(pos2(r.left() + 30.0, r.center().y), Align2::CENTER_CENTER, if open { ic::CARET_DOWN } else { ic::CARET_RIGHT }, ifont(12.0), p.weak);
        x = r.left() + 52.0;
    }
    let t = text_line(&pt, pos2(x, r.center().y), Align2::LEFT_CENTER, title, bold(12.0), p.text, r.width() - 120.0);
    if let Some(n) = n {
        let g = pt.layout_no_wrap(n.to_string(), font(10.5), p.text);
        let b = Rect::from_min_size(pos2(t.right() + 8.0, r.center().y - 8.0), vec2((g.size().x + 10.0).max(18.0), 16.0));
        pt.rect_filled(b, CornerRadius::same(8), if p.dark { rgb(56, 56, 56) } else { rgb(226, 226, 226) });
        pt.galley(b.center() - g.size() / 2.0, g, p.text);
    }
    let mut hit = None;
    let mut btns = Vec::new();
    let mut bx = r.right() - 14.0;
    for (i, (icon, tip)) in actions.iter().enumerate().rev() {
        let br = Rect::from_min_size(pos2(bx - 26.0, r.center().y - 13.0), vec2(26.0, 26.0));
        let b = icon_btn(ui, br, resp.id.with(i), icon, tip, p);
        if b.clicked() {
            hit = Some(i);
        }
        btns.push(b);
        bx -= 28.0;
    }
    btns.reverse();
    (resp.clicked() && hit.is_none(), hit, btns)
}

fn toggle(ui: &mut Ui, on: &mut bool, p: &Pal) {
    let (r, resp) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let off = if p.dark { rgb(72, 72, 74) } else { rgb(200, 200, 204) };
    let pt = ui.painter();
    pt.rect_filled(r, CornerRadius::same(10), blend(p.accent, off, t));
    let x = egui::lerp(r.left() + 10.0..=r.right() - 10.0, t);
    pt.circle_filled(pos2(x, r.center().y), 7.0, Color32::WHITE);
}

fn group(ui: &mut Ui, p: &Pal, title: &str, body: impl FnOnce(&mut Ui, &mut bool)) {
    ui.add_space(28.0);
    ui.label(RichText::new(title).font(bold(14.5)).color(p.text));
    ui.add_space(8.0);
    let card = if p.dark { rgb(37, 37, 38) } else { rgb(250, 250, 250) };
    egui::Frame::new().fill(card).stroke(Stroke::new(1.0, p.line)).corner_radius(CornerRadius::same(8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut first = true;
        body(ui, &mut first);
    });
}

/// Settings row: title and optional description on the left, the control right-aligned.
fn srow(ui: &mut Ui, p: &Pal, first: &mut bool, title: &str, desc: &str, h: f32, control: impl FnOnce(&mut Ui)) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    if !*first {
        ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, p.line));
    }
    *first = false;
    let pt = ui.painter().clone();
    let lw = r.width() * 0.5 - 20.0;
    let cy = if h > 64.0 { r.top() + 28.0 } else { r.center().y };
    if desc.is_empty() {
        text_line(&pt, pos2(r.left() + 16.0, cy), Align2::LEFT_CENTER, title, font(13.0), p.text, lw);
    } else {
        text_line(&pt, pos2(r.left() + 16.0, cy - 9.0), Align2::LEFT_CENTER, title, font(13.0), p.text, lw);
        text_line(&pt, pos2(r.left() + 16.0, cy + 10.0), Align2::LEFT_CENTER, desc, font(11.5), p.weak, lw);
    }
    let cr = Rect::from_min_max(pos2(r.left() + r.width() * 0.5, r.top() + 8.0), pos2(r.right() - 16.0, r.bottom() - 8.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(cr).layout(egui::Layout::right_to_left(egui::Align::Center)));
    control(&mut child);
}

#[cfg(windows)]
mod win {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub fn tray(ctx: eframe::egui::Context, rgba: Vec<u8>, quit: Arc<AtomicBool>, hidden: Arc<AtomicBool>) -> Option<Box<dyn std::any::Any>> {
        use eframe::egui::ViewportCommand as V;
        use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
        use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
        let menu = Menu::new();
        let show = MenuItem::new("显示主界面", true, None);
        let exit = MenuItem::new("退出", true, None);
        menu.append_items(&[&show, &PredefinedMenuItem::separator(), &exit]).ok()?;
        let (show_id, exit_id) = (show.id().clone(), exit.id().clone());
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("GaGaDown")
            .with_icon(Icon::from_rgba(rgba, 32, 32).ok()?)
            .build()
            .ok()?;
        let reveal = {
            let ctx = ctx.clone();
            move || {
                hidden.store(false, Ordering::SeqCst);
                ctx.send_viewport_cmd(V::Visible(true));
                ctx.send_viewport_cmd(V::Minimized(false));
                ctx.send_viewport_cmd(V::Focus);
                ctx.request_repaint();
            }
        };
        let on_click = reveal.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                on_click();
            }
        }));
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if e.id == show_id {
                reveal();
            } else if e.id == exit_id {
                quit.store(true, Ordering::SeqCst);
                ctx.send_viewport_cmd(V::Close);
                ctx.request_repaint();
            }
        }));
        Some(Box::new(tray))
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// Shell icon for a file (its own icon for existing exe/ico/lnk, otherwise the extension's icon).
    pub fn file_icon(path: &str, exists: bool, px: i32) -> Option<eframe::egui::ColorImage> {
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
        use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
        use windows_sys::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_ICONLOCATION, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW};
        use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, PrivateExtractIconsW};
        let w = wide(path);
        let base = if exists { 0 } else { SHGFI_USEFILEATTRIBUTES };
        let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
        let sz = std::mem::size_of::<SHFILEINFOW>() as u32;
        if unsafe { SHGetFileInfoW(w.as_ptr(), FILE_ATTRIBUTE_NORMAL, &mut info, sz, base | SHGFI_ICONLOCATION) } != 0 && info.szDisplayName[0] != 0 {
            let mut loc = [0u16; 1024];
            let n = unsafe { ExpandEnvironmentStringsW(info.szDisplayName.as_ptr(), loc.as_mut_ptr(), loc.len() as u32) };
            let src = if n > 0 && (n as usize) <= loc.len() { loc.as_ptr() } else { info.szDisplayName.as_ptr() };
            let mut h = std::ptr::null_mut();
            let mut id = 0u32;
            let got = unsafe { PrivateExtractIconsW(src, info.iIcon, px, px, &mut h, &mut id, 1, 0) };
            if got == 1 && !h.is_null() {
                let img = icon_rgba(h);
                unsafe { DestroyIcon(h) };
                if img.is_some() {
                    return img;
                }
            }
        }
        let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
        if unsafe { SHGetFileInfoW(w.as_ptr(), FILE_ATTRIBUTE_NORMAL, &mut info, sz, base | SHGFI_ICON | SHGFI_LARGEICON) } == 0 || info.hIcon.is_null() {
            return None;
        }
        let img = icon_rgba(info.hIcon);
        unsafe { DestroyIcon(info.hIcon) };
        img
    }

    fn icon_rgba(h: windows_sys::Win32::UI::WindowsAndMessaging::HICON) -> Option<eframe::egui::ColorImage> {
        use windows_sys::Win32::Graphics::Gdi::{BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, GetObjectW};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};
        let mut ii: ICONINFO = unsafe { std::mem::zeroed() };
        if unsafe { GetIconInfo(h, &mut ii) } == 0 {
            return None;
        }
        let cleanup = |ii: &ICONINFO| unsafe {
            if !ii.hbmColor.is_null() {
                DeleteObject(ii.hbmColor);
            }
            if !ii.hbmMask.is_null() {
                DeleteObject(ii.hbmMask);
            }
        };
        if ii.hbmColor.is_null() {
            cleanup(&ii);
            return None;
        }
        let mut bm: BITMAP = unsafe { std::mem::zeroed() };
        unsafe { GetObjectW(ii.hbmColor, std::mem::size_of::<BITMAP>() as i32, &mut bm as *mut _ as *mut _) };
        let (w, hgt) = (bm.bmWidth, bm.bmHeight);
        if w <= 0 || hgt <= 0 {
            cleanup(&ii);
            return None;
        }
        let mut bi: BITMAPINFO = unsafe { std::mem::zeroed() };
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -hgt;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        let mut buf = vec![0u8; (w * hgt * 4) as usize];
        let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        let lines = unsafe { GetDIBits(dc, ii.hbmColor, 0, hgt as u32, buf.as_mut_ptr() as *mut _, &mut bi, DIB_RGB_COLORS) };
        unsafe { DeleteDC(dc) };
        cleanup(&ii);
        if lines == 0 {
            return None;
        }
        let no_alpha = buf.chunks(4).all(|c| c[3] == 0);
        for c in buf.chunks_mut(4) {
            c.swap(0, 2);
            if no_alpha {
                c[3] = 255;
            }
        }
        Some(eframe::egui::ColorImage::from_rgba_unmultiplied([w as usize, hgt as usize], &buf))
    }

    /// Rasterize with GDI: it runs the font's own hinting on both axes (egui's rasterizer
    /// only snaps vertically, see emilk/egui#8034, #8079), giving Windows-native crisp text.
    pub fn gdi_glyph(r: &eframe::epaint::text::GlyphRequest<'_>) -> Option<eframe::epaint::text::RasterGlyph> {
        use eframe::epaint::text::RasterGlyph;
        use std::cell::RefCell;
        use std::collections::HashMap;
        use windows_sys::Win32::Graphics::Gdi::*;
        type Cache = (HDC, HashMap<(String, u16, i32, bool), Option<HFONT>>);
        thread_local! {
            static GDI: RefCell<Cache> = RefCell::new((std::ptr::null_mut(), HashMap::new()));
        }
        let ppem = r.ppem.round() as i32;
        if !(4..=160).contains(&ppem) {
            return None;
        }
        let ct = r.subpixel && cleartype_enabled();
        GDI.with(|c| unsafe {
            let mut c = c.borrow_mut();
            if c.0.is_null() {
                c.0 = CreateCompatibleDC(std::ptr::null_mut());
            }
            let dc = c.0;
            if dc.is_null() {
                return None;
            }
            let font = *c.1.entry((r.family.to_string(), r.weight, ppem, ct)).or_insert_with(|| {
                let name: Vec<u16> = r.family.encode_utf16().collect();
                if name.len() >= 32 {
                    return None;
                }
                let mut lf: LOGFONTW = std::mem::zeroed();
                lf.lfHeight = -ppem;
                lf.lfWeight = r.weight as i32;
                lf.lfCharSet = DEFAULT_CHARSET as _;
                lf.lfOutPrecision = OUT_TT_ONLY_PRECIS as _;
                lf.lfQuality = if ct { CLEARTYPE_QUALITY } else { ANTIALIASED_QUALITY } as _;
                lf.lfFaceName[..name.len()].copy_from_slice(&name);
                let f = CreateFontIndirectW(&lf);
                if f.is_null() {
                    return None;
                }
                let previous = SelectObject(dc, f);
                // Glyph ids are only meaningful if GDI picked the very same face (it may report a
                // localized family name, so compare table data instead).
                let table = |tag: &[u8; 4], off: u32, n: usize| {
                    let mut b = [0u8; 4];
                    (GetFontData(dc, u32::from_le_bytes(*tag), off, b.as_mut_ptr().cast(), n as u32) == n as u32).then_some(b)
                };
                let glyphs = table(b"maxp", 4, 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as u32);
                let sum = table(b"head", 8, 4).map(u32::from_be_bytes);
                let same = glyphs == Some(r.num_glyphs) && sum == Some(r.checksum);
                // GDI cannot delete a selected font. Restore the previous object
                // on both paths; the validated font is selected below for drawing.
                SelectObject(dc, previous);
                if !same {
                    DeleteObject(f);
                    return None;
                }
                Some(f)
            });
            let font = font?;
            SelectObject(dc, font);
            let one = FIXED { fract: 0, value: 1 };
            let zero = FIXED { fract: 0, value: 0 };
            let mat = MAT2 { eM11: one, eM12: zero, eM21: zero, eM22: one };
            let fmt = GGO_GRAY8_BITMAP | GGO_GLYPH_INDEX;
            let mut gm: GLYPHMETRICS = std::mem::zeroed();
            let n = GetGlyphOutlineW(dc, r.glyph_id, fmt, &mut gm, 0, std::ptr::null_mut(), &mat);
            if n == u32::MAX {
                return None;
            }
            if n == 0 {
                return Some(RasterGlyph { width: 0, height: 0, left: 0, top: 0, coverage: Vec::new(), rgb: false });
            }
            if ct && let Some(g) = cleartype_glyph(dc, r.glyph_id, &gm) {
                return Some(g);
            }
            let mut buf = vec![0u8; n as usize];
            if GetGlyphOutlineW(dc, r.glyph_id, fmt, &mut gm, n, buf.as_mut_ptr().cast(), &mat) == u32::MAX {
                return None;
            }
            let (w, h) = (gm.gmBlackBoxX as usize, gm.gmBlackBoxY as usize);
            let pitch = (w + 3) & !3;
            let mut coverage = vec![0u8; w * h];
            for y in 0..h {
                for x in 0..w {
                    let v = buf.get(y * pitch + x).copied().unwrap_or(0).min(64) as u32;
                    coverage[y * w + x] = ((v * 255 + 32) / 64) as u8;
                }
            }
            Some(RasterGlyph { width: w as u32, height: h as u32, left: gm.gmptGlyphOrigin.x, top: -gm.gmptGlyphOrigin.y, coverage, rgb: false })
        })
    }

    /// Whether the user has ClearType on (Windows "Adjust ClearType text").
    fn cleartype_enabled() -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| unsafe {
            let mut on = 0i32;
            let mut kind = 0u32;
            SystemParametersInfoW(SPI_GETFONTSMOOTHING, 0, (&mut on as *mut i32).cast(), 0) != 0
                && on != 0
                && SystemParametersInfoW(SPI_GETFONTSMOOTHINGTYPE, 0, (&mut kind as *mut u32).cast(), 0) != 0
                && kind == FE_FONTSMOOTHINGCLEARTYPE
        })
    }

    /// GDI ClearType gamma depends on polarity, so glyphs are drawn the way the theme shows them.
    pub static DARK_TEXT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// Draw the glyph with the selected ClearType font (white-on-black in dark theme, black-on-white
    /// inverted otherwise); each channel is that subpixel's coverage with GDI's own gamma and contrast.
    unsafe fn cleartype_glyph(
        dc: windows_sys::Win32::Graphics::Gdi::HDC,
        glyph: u32,
        gm: &windows_sys::Win32::Graphics::Gdi::GLYPHMETRICS,
    ) -> Option<eframe::epaint::text::RasterGlyph> {
        use windows_sys::Win32::Graphics::Gdi::*;
        const PAD: i32 = 2;
        let glyph = u16::try_from(glyph).ok()?;
        let (w, h) = (gm.gmBlackBoxX as i32 + 2 * PAD, gm.gmBlackBoxY as i32 + 2 * PAD);
        unsafe {
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -h;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                if !bmp.is_null() {
                    DeleteObject(bmp);
                }
                return None;
            }
            let n = (w * h) as usize;
            let dark = DARK_TEXT.load(std::sync::atomic::Ordering::Relaxed);
            std::ptr::write_bytes(bits.cast::<u8>(), if dark { 0 } else { 0xFF }, n * 4);
            let old = SelectObject(dc, bmp);
            SetBkMode(dc, TRANSPARENT as _);
            SetTextColor(dc, if dark { 0x00FF_FFFF } else { 0 });
            SetTextAlign(dc, TA_BASELINE | TA_LEFT);
            let ok = ExtTextOutW(dc, PAD - gm.gmptGlyphOrigin.x, PAD + gm.gmptGlyphOrigin.y, ETO_GLYPH_INDEX, std::ptr::null(), &glyph, 1, std::ptr::null()) != 0;
            GdiFlush();
            let px = std::slice::from_raw_parts(bits.cast::<u8>(), n * 4);
            let inv = if dark { 0 } else { 0xFF };
            let coverage: Vec<u8> = px.chunks_exact(4).flat_map(|p| [p[2] ^ inv, p[1] ^ inv, p[0] ^ inv]).collect();
            SelectObject(dc, old);
            DeleteObject(bmp);
            ok.then_some(eframe::epaint::text::RasterGlyph {
                width: w as u32,
                height: h as u32,
                left: gm.gmptGlyphOrigin.x - PAD,
                top: -gm.gmptGlyphOrigin.y - PAD,
                coverage,
                rgb: true,
            })
        }
    }

    pub fn app_path(exe: &str) -> Option<std::path::PathBuf> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        let sub = format!(r"Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
        [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
            .into_iter()
            .find_map(|h| RegKey::predef(h).open_subkey(&sub).ok()?.get_value::<String, _>("").ok())
            .map(|s| std::path::PathBuf::from(s.trim_matches('"')))
            .filter(|p| p.exists())
    }

    /// Per-user external-extension entry; the browser asks the user to enable it on next start.
    pub fn register_extension(reg: &str, id: &str, update_url: &str) -> bool {
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .create_subkey(format!(r"{reg}\{id}"))
            .and_then(|(k, _)| k.set_value("update_url", &update_url))
            .is_ok()
    }

    pub fn set_autostart(on: bool) {
        let Ok((k, _)) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run") else {
            return;
        };
        if on {
            if let Ok(exe) = std::env::current_exe() {
                let _ = k.set_value("GaGaDown", &format!("\"{}\" --autostart", exe.display()));
            }
        } else {
            let _ = k.delete_value("GaGaDown");
        }
    }

    pub fn accent() -> Option<eframe::egui::Color32> {
        let k = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(r"Software\Microsoft\Windows\DWM").ok()?;
        let v: u32 = k.get_value("AccentColor").ok()?;
        Some(eframe::egui::Color32::from_rgb((v & 0xff) as u8, (v >> 8 & 0xff) as u8, (v >> 16 & 0xff) as u8))
    }

    pub fn decorate(frame: &eframe::Frame) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
        use windows_sys::Win32::UI::Controls::MARGINS;
        let Ok(h) = frame.window_handle() else { return };
        let RawWindowHandle::Win32(w) = h.as_raw() else { return };
        let hwnd = w.hwnd.get() as _;
        let pref = DWMWCP_ROUND;
        let m = MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 };
        unsafe {
            DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as _, &pref as *const _ as *const _, std::mem::size_of_val(&pref) as u32);
            DwmExtendFrameIntoClientArea(hwnd, &m);
        }
    }
}

#[cfg(not(windows))]
mod win {
    pub fn set_autostart(_on: bool) {}

    pub fn app_path(_exe: &str) -> Option<std::path::PathBuf> {
        None
    }

    pub fn register_extension(_reg: &str, _id: &str, _update_url: &str) -> bool {
        false
    }

    pub fn accent() -> Option<eframe::egui::Color32> {
        None
    }
    pub fn decorate(_frame: &eframe::Frame) {}
    pub fn tray(
        _ctx: eframe::egui::Context,
        _rgba: Vec<u8>,
        _quit: std::sync::Arc<std::sync::atomic::AtomicBool>,
        _hidden: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Option<Box<dyn std::any::Any>> {
        None
    }
    pub fn file_icon(_path: &str, _exists: bool, _px: i32) -> Option<eframe::egui::ColorImage> {
        None
    }
}

fn apply_style(ctx: &egui::Context, accent: Color32) {
    for (theme, dark) in [(egui::Theme::Light, false), (egui::Theme::Dark, true)] {
        let p = Pal::new(dark, accent);
        ctx.style_mut_of(theme, |s| {
            s.spacing.item_spacing = vec2(8.0, 6.0);
            s.spacing.button_padding = vec2(12.0, 5.0);
            s.spacing.interact_size.y = 30.0;
            s.spacing.menu_margin = Margin::same(4);
            let v = &mut s.visuals;
            v.text_options.font_hinting = true;
            v.text_options.subpixel_binning = false;
            v.text_options.color_transfer_function =
                if dark { egui::epaint::FontColorTransferFunction::TwoCoverageMinusCoverageSq } else { egui::epaint::FontColorTransferFunction::Gamma(0.75) };
            v.panel_fill = p.editor;
            v.window_fill = if dark { rgb(37, 37, 38) } else { rgb(255, 255, 255) };
            v.window_stroke = Stroke::new(1.0, if dark { rgb(54, 54, 54) } else { rgb(212, 212, 212) });
            v.window_corner_radius = CornerRadius::same(8);
            v.menu_corner_radius = CornerRadius::same(10);
            let shadow = egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 36 }) };
            v.window_shadow = shadow;
            v.popup_shadow = shadow;
            v.extreme_bg_color = p.input;
            v.faint_bg_color = p.hover;
            v.code_bg_color = p.input;
            v.selection.bg_fill = blend(accent, p.editor, 0.4);
            v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
            v.hyperlink_color = accent;
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
            v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
            let (b0, b1, b2) = if dark { (rgb(49, 49, 49), rgb(60, 60, 60), rgb(70, 70, 70)) } else { (rgb(236, 236, 236), rgb(226, 226, 226), rgb(214, 214, 214)) };
            for (w, f) in [(&mut v.widgets.inactive, b0), (&mut v.widgets.hovered, b1), (&mut v.widgets.active, b2), (&mut v.widgets.open, b1)] {
                w.corner_radius = CornerRadius::same(6);
                w.bg_stroke = Stroke::NONE;
                w.expansion = 0.0;
                w.bg_fill = f;
                w.weak_bg_fill = f;
                w.fg_stroke = Stroke::new(1.0, p.text);
            }
            v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.line);
        });
    }
}

fn sharp() -> egui::FontTweak {
    egui::FontTweak {
        hinting: Some(true),
        hinting_target: egui::epaint::text::HintingTarget::Mono,
        subpixel_binning: Some(false),
        ..Default::default()
    }
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let read = |c: &[&str]| c.iter().find_map(|p| std::fs::read(p).ok());
    let mut base = Vec::new();
    if let Some(d) = read(&[
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyh.ttf",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "/System/Library/Fonts/PingFang.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    ]) {
        fonts.font_data.insert("ui".into(), Arc::new(egui::FontData::from_owned(d).tweak(sharp())));
        base.push("ui".to_string());
    }
    if let Some(d) = read(&["C:\\Windows\\Fonts\\consola.ttf"]) {
        fonts.font_data.insert("mono".into(), Arc::new(egui::FontData::from_owned(d).tweak(sharp())));
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "mono".into());
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.retain(|f| f != "phosphor");
    for (i, f) in base.iter().enumerate() {
        prop.insert(i, f.clone());
    }
    let mut icons = vec!["phosphor".to_string()];
    icons.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(FontFamily::Name("icons".into()), icons);
    if !base.is_empty() {
        fonts.families.entry(FontFamily::Monospace).or_default().push("ui".into());
    }
    let mut bold_fam: Vec<String> = Vec::new();
    if let Some(d) = read(&["C:\\Windows\\Fonts\\msyhbd.ttc", "C:\\Windows\\Fonts\\msyhbd.ttf", "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc"]) {
        fonts.font_data.insert("bold".into(), Arc::new(egui::FontData::from_owned(d).tweak(sharp())));
        bold_fam.push("bold".into());
    }
    bold_fam.extend(fonts.families[&FontFamily::Proportional].iter().filter(|f| *f != "phosphor").cloned());
    fonts.families.insert(FontFamily::Name("bold".into()), bold_fam);
    ctx.set_fonts(fonts);
}

impl App {
    fn file_tex(&mut self, ctx: &egui::Context, name: &str, path: Option<&std::path::Path>) -> Option<egui::TextureHandle> {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        let real = path.filter(|p| matches!(ext.as_str(), "exe" | "ico" | "lnk") && p.exists());
        let key = real.map(|p| p.display().to_string()).unwrap_or_else(|| format!(".{ext}"));
        if let Some(t) = self.ficons.get(&key) {
            return t.clone();
        }
        let px = (32.0 * ctx.pixels_per_point()).round().clamp(32.0, 256.0) as i32;
        let query = real.map(|p| p.display().to_string()).unwrap_or_else(|| format!("x.{}", if ext.is_empty() { "bin" } else { &ext }));
        let tex = win::file_icon(&query, real.is_some(), px).map(|img| ctx.load_texture(format!("fi{key}"), img, egui::TextureOptions::LINEAR));
        self.ficons.insert(key, tex.clone());
        tex
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_file(&mut self, ctx: &egui::Context, pt: &Painter, c: Pos2, size: f32, name: &str, path: Option<&std::path::Path>, p: &Pal, alpha: f32) {
        if let Some(t) = self.file_tex(ctx, name, path) {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            pt.image(t.id(), Rect::from_center_size(c, vec2(size, size)), uv, Color32::WHITE.gamma_multiply(alpha));
        } else {
            let (g, gc) = file_glyph(name, p);
            pt.text(c, Align2::CENTER_CENTER, g, ifont(size * 0.78), gc.gamma_multiply(alpha));
        }
    }

    fn new(rt: tokio::runtime::Runtime, engine: Engine, api_error: Arc<Mutex<Option<String>>>, icon: Arc<egui::IconData>, accent: Color32) -> Self {
        let draft = engine.settings();
        let draft_proxies = draft.proxy.proxies.join("\n");
        Self {
            rt,
            engine,
            page: Page::Downloads,
            input: String::new(),
            views: Vec::new(),
            deleted: Vec::new(),
            last_refresh: None,
            selected: None,
            removing: None,
            err_report: None,
            rm_fade: Fade::default(),
            er_fade: Fade::default(),
            browsers: browsers(),
            ext_guide: None,
            ext_tab: 0,
            remove_mode: RemoveMode::TrashFiles,
            draft,
            draft_proxies,
            cache: None,
            toasts: Vec::new(),
            handoff_fails: Vec::new(),
            api_error,
            popups: Vec::new(),
            dismissed: Default::default(),
            tex: None,
            icon,
            accent,
            accent_checked: Instant::now(),
            decorated: false,
            settings_dirty: None,
            ficons: Default::default(),
            geo: Default::default(),
            flags: Default::default(),
            tray: None,
            quit: Arc::new(AtomicBool::new(false)),
            hidden: Arc::new(AtomicBool::new(false)),
        }
    }

    fn install_extension(&mut self, ctx: &egui::Context, i: usize) {
        let b = self.browsers.get(i).cloned();
        if let Some((b, id)) = b.as_ref().and_then(|b| Some((b, b.store_id?))) {
            if win::register_extension(b.reg, id, b.update_url) {
                let _ = std::process::Command::new(&b.exe).arg(format!("{}{id}", b.store_page)).spawn();
                self.toast(format!("已添加到 {}，重启浏览器后点“启用”", b.name), false);
                return;
            }
        }
        let dir = match unpack_extension(self.engine.data_dir()) {
            Ok(d) => d,
            Err(e) => return self.toast(format!("解压插件失败：{e}"), true),
        };
        match b {
            Some(b) => {
                ctx.copy_text(b.page.to_string());
                let _ = std::process::Command::new(&b.exe).spawn();
                self.ext_guide = Some(ExtGuide { browser: i, since: gagadown_core::task::now_secs(), dir });
            }
            None => {
                ctx.copy_text(dir.display().to_string());
                open_path(&dir);
                self.toast("插件文件夹路径已复制", false);
            }
        }
    }

    fn ext_connected(&self, key: &str) -> bool {
        self.engine.browser_last_seen(key).is_some_and(|t| gagadown_core::task::now_secs().saturating_sub(t) < 150)
    }

    fn toast(&mut self, s: impl Into<String>, bad: bool) {
        self.toasts.push((Instant::now(), s.into(), bad));
    }

    fn refresh(&mut self) {
        if self.last_refresh.is_some_and(|t| t.elapsed() < Duration::from_millis(200)) {
            return;
        }
        self.last_refresh = Some(Instant::now());
        self.views = self.engine.views(400);
        if self.page == Page::Trash {
            self.deleted = self.engine.deleted();
            self.deleted.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
        }
        for n in self.engine.take_notices() {
            self.toast(n.text, false);
        }
        let api_error = self.api_error.lock().take();
        if let Some(e) = api_error {
            self.toast(e, true);
        }
    }

    fn go(&mut self, page: Page) {
        if self.page == page {
            return;
        }
        self.page = page;
        self.last_refresh = None;
        match page {
            Page::Cache => self.cache = Some(self.engine.cache_report()),
            Page::Settings => {
                self.draft = self.engine.settings();
                self.draft_proxies = self.draft.proxy.proxies.join("\n");
            }
            _ => {}
        }
    }

    fn add_input(&mut self) {
        let text = std::mem::take(&mut self.input);
        let mut added = 0;
        for url in text.split_whitespace().filter(|u| u.contains("://")) {
            let req = AddRequest { url: url.to_string(), source: Some("manual".into()), ..Default::default() };
            match self.engine.add(req, None) {
                Ok(o) if o.existed => self.toast(format!("已在列表中：{}", o.filename), false),
                Ok(_) => added += 1,
                Err(e) => self.toast(format!("{e}"), true),
            }
        }
        if added == 0 && !text.contains("://") {
            self.input = text;
        }
        if added > 0 {
            self.go(Page::Downloads);
        }
        self.last_refresh = None;
    }

    fn server_row(&mut self, ui: &mut Ui, v: &TaskView, p: &Pal) {
        let host = v.host.clone();
        let geo = self.geo.lock().get(&host).cloned();
        if geo.is_none() {
            self.geo.lock().insert(host.clone(), None);
            let (map, ctx, h) = (self.geo.clone(), ui.ctx().clone(), host.clone());
            self.rt.spawn(async move {
                let g = lookup_geo(&h).await.unwrap_or_default();
                map.lock().insert(h, Some(g));
                ctx.request_repaint();
            });
        }
        let g = geo.flatten().unwrap_or_default();
        if let Some(img) = &g.flag {
            if !self.flags.contains_key(&g.cc) {
                let t = ui.ctx().load_texture(format!("flag-{}", g.cc), img.clone(), egui::TextureOptions::LINEAR);
                self.flags.insert(g.cc.clone(), t);
            }
        }
        let (route, ms) = route_info(v);
        let mut sub = vec![];
        if !g.ip.is_empty() {
            sub.push(g.ip.clone());
        }
        if let Some(r) = route {
            sub.push(r);
        }
        if let Some(ms) = ms {
            sub.push(format!("{ms} ms"));
        }
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
        let pt = ui.painter();
        let fr = Rect::from_min_size(pos2(r.left(), r.center().y - 7.0), vec2(20.0, 14.0));
        if let Some(t) = self.flags.get(&g.cc) {
            pt.image(t.id(), fr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            pt.rect_filled(fr, CornerRadius::same(2), p.text.gamma_multiply(0.08));
        }
        let x = r.left() + 30.0;
        let w = r.right() - x;
        text_line(pt, pos2(x, r.top() + 10.0), Align2::LEFT_CENTER, host, font(12.5), p.text, w);
        text_line(pt, pos2(x, r.top() + 28.0), Align2::LEFT_CENTER, sub.join("   "), font(11.5), p.weak, w);
    }

    fn act(&mut self, ctx: &egui::Context, v: &TaskView, a: Act) {
        let id = v.id;
        match a {
            Act::Pause => self.engine.pause(id),
            Act::Resume => self.engine.resume(id),
            Act::Redownload => {
                let e = self.engine.clone();
                self.rt.spawn(async move { e.redownload(id).await });
            }
            Act::Open => open_path(v.path.as_deref().unwrap_or(&v.dir)),
            Act::Reveal => reveal(v.path.as_deref().unwrap_or(&v.dir)),
            Act::CopyLink => {
                ctx.copy_text(v.url.clone());
                self.toast("链接已复制", false);
            }
            Act::Remove => self.removing = Some(id),
            Act::ErrorInfo => self.err_report = Some(id),
        }
        self.last_refresh = None;
    }

    fn icon_tex(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        self.tex
            .get_or_insert_with(|| ctx.load_texture("icon", egui::ColorImage::from_rgba_unmultiplied([256, 256], ICON), egui::TextureOptions::LINEAR))
            .clone()
    }

    fn title_bar(&mut self, ui: &mut Ui, p: &Pal) {
        let ctx = ui.ctx().clone();
        let r = ui.max_rect();
        let bg = ui.interact(r, Id::new("titlebar"), Sense::click_and_drag());
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if bg.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if bg.double_clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }
        let tex = self.icon_tex(&ctx);
        let ir = Rect::from_center_size(pos2(r.left() + 24.0, r.center().y), vec2(16.0, 16.0));
        egui::Image::new(egui::load::SizedTexture::new(tex.id(), ir.size())).paint_at(ui, ir);
        ui.painter().text(pos2(r.left() + 44.0, r.center().y), Align2::LEFT_CENTER, "GaGaDown", font(12.5), p.weak);

        let bw = 46.0;
        let close = Rect::from_min_size(pos2(r.right() - bw, r.top()), vec2(bw, r.height()));
        let maxr = close.translate(vec2(-bw, 0.0));
        let minr = maxr.translate(vec2(-bw, 0.0));
        if chrome_btn(ui, close, Glyph::Close, p) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if chrome_btn(ui, maxr, if maximized { Glyph::Restore } else { Glyph::Max }, p) {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }
        if chrome_btn(ui, minr, Glyph::Min, p) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }

        let w = (r.width() * 0.42).clamp(280.0, 620.0);
        let cb = Rect::from_center_size(r.center(), vec2(w, 24.0));
        let id = Id::new("url");
        let focus = ctx.memory(|m| m.has_focus(id));
        ui.painter().rect(cb, CornerRadius::same(6), p.input, Stroke::new(1.0, if focus { p.accent } else { p.line }), StrokeKind::Inside);
        ui.painter().text(pos2(cb.left() + 14.0, cb.center().y), Align2::CENTER_CENTER, ic::LINK, ifont(13.0), p.weak);
        let go = Rect::from_min_size(pos2(cb.right() - 24.0, cb.top() + 2.0), vec2(22.0, 20.0));
        let te = Rect::from_min_max(pos2(cb.left() + 28.0, cb.top()), pos2(go.left() - 4.0, cb.bottom()));
        let resp = ui.put(
            te,
            egui::TextEdit::singleline(&mut self.input)
                .id(id)
                .frame(egui::Frame::NONE)
                .font(font(12.5))
                .vertical_align(egui::Align::Center)
                .hint_text(RichText::new("粘贴下载链接或搜索").color(p.weak)),
        );
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let clicked = icon_at(ui, go, Id::new("go"), ic::ARROW_DOWN, "下载", p);
        if enter || clicked {
            self.add_input();
        }
    }

    fn activity_bar(&mut self, ui: &mut Ui, p: &Pal) {
        let r = ui.max_rect();
        let active = self.views.iter().filter(|v| matches!(v.status, Status::Running | Status::Queued)).count();
        let items = [(Page::Downloads, ic::DOWNLOAD_SIMPLE, "下载"), (Page::Trash, ic::TRASH, "回收站"), (Page::Cache, ic::HARD_DRIVES, "缓存"), (Page::Extension, ic::PUZZLE_PIECE, "浏览器插件")];
        for (i, (page, icon, tip)) in items.into_iter().enumerate() {
            let ir = Rect::from_min_size(pos2(r.left(), r.top() + 2.0 + i as f32 * 48.0), vec2(r.width(), 48.0));
            self.activity_item(ui, ir, page, icon, tip, if page == Page::Downloads { active } else { 0 }, p);
        }
        let sr = Rect::from_min_size(pos2(r.left(), r.bottom() - 50.0), vec2(r.width(), 48.0));
        self.activity_item(ui, sr, Page::Settings, ic::GEAR_SIX, "设置", 0, p);
    }

    #[allow(clippy::too_many_arguments)]
    fn activity_item(&mut self, ui: &mut Ui, r: Rect, page: Page, icon: &str, tip: &str, badge: usize, p: &Pal) {
        let resp = ui.interact(r, Id::new(("activity", tip)), Sense::click()).tip(tip);
        let on = self.page == page;
        let pt = ui.painter();
        if on {
            pt.rect_filled(Rect::from_min_size(r.min, vec2(2.0, r.height())), 0.0, p.accent);
        }
        pt.text(r.center(), Align2::CENTER_CENTER, icon, ifont(22.0), if on || resp.hovered() { p.text } else { p.weak });
        if badge > 0 {
            let c = r.center() + vec2(9.0, 9.0);
            pt.circle_filled(c, 7.5, p.accent);
            pt.text(c, Align2::CENTER_CENTER, badge.min(99).to_string(), font(9.5), Color32::WHITE);
        }
        if resp.clicked() {
            self.go(page);
        }
    }

    fn status_bar(&mut self, ui: &mut Ui, p: &Pal) {
        let r = ui.max_rect();
        let pt = ui.painter();
        let running: Vec<&TaskView> = self.views.iter().filter(|v| v.status == Status::Running).collect();
        let queued = self.views.iter().filter(|v| v.status == Status::Queued).count();
        let busy = !running.is_empty();
        let fg = if busy { Color32::WHITE } else { p.weak };
        if busy {
            pt.rect_filled(r, 0.0, p.accent);
        }
        let y = r.center().y;
        let mut x = r.left() + 10.0;
        pt.text(pos2(x + 6.0, y), Align2::CENTER_CENTER, ic::ARROW_DOWN, ifont(12.0), fg);
        x += 18.0;
        let mut item = |s: String| {
            let t = text_line(pt, pos2(x, y), Align2::LEFT_CENTER, s, font(12.0), fg, 400.0);
            x = t.right() + 18.0;
        };
        item(speed(self.engine.total_speed()));
        if busy {
            item(format!("{} 个下载中", running.len()));
        }
        if queued > 0 {
            item(format!("{queued} 个排队"));
        }
        let (done, total) = running.iter().filter_map(|v| v.size.map(|s| (v.downloaded, s))).fold((0u64, 0u64), |a, b| (a.0 + b.0, a.1 + b.1));
        if total > 0 {
            let pct = done as f64 / total as f64;
            let bar = Rect::from_min_size(pos2(r.right() - 130.0, y - 2.0), vec2(120.0, 4.0));
            pt.rect_filled(bar, CornerRadius::same(2), Color32::from_white_alpha(70));
            let mut f = bar;
            f.set_width(bar.width() * pct as f32);
            pt.rect_filled(f, CornerRadius::same(2), Color32::WHITE);
            pt.text(pos2(bar.left() - 8.0, y), Align2::RIGHT_CENTER, format!("{:.0}%", pct * 100.0), font(12.0), fg);
        }
    }

    fn hero(&mut self, ui: &mut Ui, v: &TaskView, p: &Pal) {
        let ctx = ui.ctx().clone();
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 156.0), Sense::hover());
        let pt = ui.painter().clone();
        pt.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.line));
        let lw = (r.width() * 0.46).clamp(300.0, 520.0);
        let left = Rect::from_min_max(r.min + vec2(20.0, 18.0), pos2(r.left() + 20.0 + lw, r.bottom() - 18.0));
        let graph = Rect::from_min_max(pos2(left.right() + 28.0, r.top() + 20.0), pos2(r.right() - 20.0, r.bottom() - 18.0));

        self.paint_file(&ui.ctx().clone(), &pt, pos2(left.left() + 9.0, left.top() + 10.0), 20.0, &v.filename, None, p, 1.0);
        text_line(&pt, pos2(left.left() + 26.0, left.top() + 10.0), Align2::LEFT_CENTER, &v.filename, bold(15.0), p.text, left.width() - 26.0);

        let peak = v.history.iter().cloned().fold(0.0f32, f32::max) as f64;
        let key = Id::new(("hero", v.id));
        let net = spring(&ctx, key.with("spd"), v.speed as f32, 5.0, 0.0) as f64;
        let stats = [
            ("网络", speed(net)),
            ("峰值", speed(peak.max(v.speed))),
            ("连接", format!("{} / {}", v.connections, v.target)),
        ];
        for (i, (k, val)) in stats.iter().enumerate() {
            let x = left.left() + i as f32 * 118.0;
            pt.text(pos2(x, left.top() + 34.0), Align2::LEFT_TOP, *k, font(11.5), p.weak);
            pt.text(pos2(x, left.top() + 50.0), Align2::LEFT_TOP, val, bold(14.0), p.text);
        }

        let y = left.top() + 84.0;
        pt.text(pos2(left.left(), y), Align2::LEFT_CENTER, if v.status == Status::Paused { "已暂停" } else { "正在下载" }, font(12.0), p.text);
        let f = seg_bar(&ctx, &pt, Rect::from_min_size(pos2(left.left(), y + 10.0), vec2(left.width(), 5.0)), v, p, key);
        let right = match v.size {
            Some(_) => format!("{}   {:.0}%", of(v), f * 100.0),
            None => bytes(v.downloaded),
        };
        pt.text(pos2(left.right(), y), Align2::RIGHT_CENTER, right, font(12.0), p.weak);

        let eta = match (v.status, v.eta_secs) {
            (Status::Running, Some(e)) => format!("剩余时间 {}", dur(e)),
            (Status::Running, None) => meta_line(v, p).0,
            _ => String::new(),
        };
        pt.text(pos2(left.left(), y + 38.0), Align2::LEFT_CENTER, eta, font(12.0), p.weak);
        let br = Rect::from_min_size(pos2(left.right() - 26.0, y + 25.0), vec2(26.0, 26.0));
        let (icon, tip, a) = if v.status == Status::Paused { (ic::PLAY, "继续", Act::Resume) } else { (ic::PAUSE, "暂停", Act::Pause) };
        if icon_at(ui, br, Id::new("hero_btn"), icon, tip, p) {
            self.act(&ctx, v, a);
        }
        if graph.width() > 80.0 {
            speed_graph(&ctx, &pt, graph, &v.history, p, key.with("graph"));
        }
    }

    fn downloads(&mut self, ui: &mut Ui, p: &Pal) {
        let hero = self
            .selected
            .and_then(|id| self.views.iter().find(|v| v.id == id && matches!(v.status, Status::Running | Status::Paused)))
            .or_else(|| self.views.iter().filter(|v| v.status == Status::Running).max_by_key(|v| v.created_at))
            .cloned();
        if let Some(h) = hero {
            self.hero(ui, &h, p);
        }
        let q = self.input.trim().to_lowercase();
        let q = if q.contains("://") { String::new() } else { q };
        let keep = |v: &&TaskView| q.is_empty() || v.filename.to_lowercase().contains(&q);
        let mut active: Vec<TaskView> = self.views.iter().filter(|v| matches!(v.status, Status::Running | Status::Queued | Status::Paused)).filter(keep).cloned().collect();
        active.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let mut failed: Vec<TaskView> = self.views.iter().filter(|v| v.status == Status::Failed).filter(keep).cloned().collect();
        failed.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let mut done: Vec<TaskView> = self.views.iter().filter(|v| v.status == Status::Completed).filter(keep).cloned().collect();
        done.sort_by(|a, b| b.finished_at.unwrap_or(b.created_at).cmp(&a.finished_at.unwrap_or(a.created_at)));

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.add_space(4.0);
            self.section(ui, Sec::Active, &active, p);
            if !failed.is_empty() {
                self.section(ui, Sec::Failed, &failed, p);
            }
            if !self.handoff_fails.is_empty() {
                self.handoff_section(ui, p);
            }
            self.section(ui, Sec::Done, &done, p);
            ui.add_space(12.0);
        });
    }

    fn section(&mut self, ui: &mut Ui, sec: Sec, items: &[TaskView], p: &Pal) {
        let ctx = ui.ctx().clone();
        let key = Id::new(("section", sec as u8));
        let open = ctx.data_mut(|d| *d.get_persisted_mut_or(key, true));
        let (title, actions): (&str, &[(&str, &str)]) = match sec {
            Sec::Active => ("下载中", &[(ic::PLAY, "全部开始"), (ic::PAUSE, "全部暂停")]),
            Sec::Failed => ("失败", &[(ic::ARROW_CLOCKWISE, "全部重试")]),
            Sec::Done => ("已完成", &[(ic::BROOM, "清除全部")]),
        };
        let actions = if items.is_empty() { &[][..] } else { actions };
        let (toggle, hit, btns) = header(ui, title, Some(items.len()), Some(open), actions, p);
        if toggle {
            ctx.data_mut(|d| d.insert_persisted(key, !open));
        }
        match (sec, hit) {
            (Sec::Active, Some(0)) => self.engine.resume_all(),
            (Sec::Active, Some(1)) => self.engine.pause_all(),
            (Sec::Failed, Some(0)) => self.engine.retry_failed(),
            _ => {}
        }
        if matches!(sec, Sec::Done)
            && let Some(b) = btns.first()
        {
            let mut pick = None;
            egui::Popup::menu(b).show(|ui| {
                ui.set_width(190.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                for (mode, label, danger) in [
                    (RemoveMode::KeepFiles, "清除全部，保留文件", false),
                    (RemoveMode::TrashFiles, "清除全部，文件移到回收站", false),
                    (RemoveMode::DeleteFiles, "清除全部并彻底删除文件", true),
                ] {
                    if menu_item(ui, label, danger, p) {
                        pick = Some(mode);
                        ui.close();
                    }
                }
            });
            if let Some(mode) = pick {
                let ids: Vec<Uuid> = items.iter().map(|v| v.id).collect();
                let e = self.engine.clone();
                self.rt.spawn(async move {
                    for id in ids {
                        e.remove(id, mode).await;
                    }
                });
                self.selected = None;
                self.last_refresh = None;
            }
        }
        if hit.is_some() {
            self.last_refresh = None;
        }
        if !open {
            return;
        }
        if items.is_empty() {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
            let t = match sec {
                Sec::Active => "队列中无下载",
                _ => "暂无",
            };
            ui.painter().text(pos2(r.left() + 52.0, r.center().y), Align2::LEFT_CENTER, t, font(12.5), p.weak);
            return;
        }
        for v in items {
            self.row(ui, v, p);
        }
        ui.add_space(6.0);
    }

    /// Browser hand-offs that failed; the browser kept those downloads.
    fn handoff_section(&mut self, ui: &mut Ui, p: &Pal) {
        let ctx = ui.ctx().clone();
        let key = Id::new("section_handoff");
        let open = ctx.data_mut(|d| *d.get_persisted_mut_or(key, true));
        let (toggle, hit, _) = header(ui, "未接管", Some(self.handoff_fails.len()), Some(open), &[(ic::BROOM, "清除记录")], p);
        if toggle {
            ctx.data_mut(|d| d.insert_persisted(key, !open));
        }
        if hit == Some(0) {
            self.handoff_fails.clear();
            return;
        }
        if !open {
            return;
        }
        let mut drop = None;
        for (i, h) in self.handoff_fails.clone().iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
            let hovered = ui.rect_contains_pointer(r);
            let pt = ui.painter().clone();
            if hovered {
                pt.rect_filled(r, 0.0, p.hover);
            }
            self.paint_file(&ctx, &pt, pos2(r.left() + 30.0, r.center().y), 26.0, &h.name, None, p, 0.7);
            let w = (r.width() - 52.0 - 56.0).max(80.0);
            text_line(&pt, pos2(r.left() + 52.0, r.top() + 15.0), Align2::LEFT_CENTER, &h.name, font(13.5), p.text, w);
            text_line(&pt, pos2(r.left() + 52.0, r.top() + 32.0), Align2::LEFT_CENTER, format!("{}   已交给浏览器下载   {}", h.reason, ago(h.at)), font(12.0), p.red, w);
            if hovered {
                let b = Rect::from_min_size(pos2(r.right() - 40.0, r.center().y - 13.0), vec2(26.0, 26.0));
                if icon_at(ui, b, resp.id.with(("handoff_x", i)), ic::X, "移除记录", p) {
                    drop = Some(i);
                }
            }
        }
        if let Some(i) = drop {
            self.handoff_fails.remove(i);
        }
        ui.add_space(6.0);
    }

    fn row_actions(v: &TaskView) -> Vec<(&'static str, &'static str, Act)> {
        let mut a = match v.status {
            Status::Running | Status::Queued => vec![(ic::PAUSE, "暂停", Act::Pause)],
            Status::Paused => vec![(ic::PLAY, "继续", Act::Resume), (ic::ARROW_CLOCKWISE, "重新下载", Act::Redownload)],
            Status::Failed => vec![(ic::PLAY, "重试", Act::Resume), (ic::ARROW_CLOCKWISE, "重新下载", Act::Redownload)],
            Status::Completed if v.file_missing => vec![(ic::ARROW_CLOCKWISE, "重新下载", Act::Redownload)],
            Status::Completed => vec![(ic::FOLDER_OPEN, "打开所在文件夹", Act::Reveal)],
        };
        if v.status == Status::Failed && v.error.is_some() {
            a.insert(0, (ic::INFO, "错误详情", Act::ErrorInfo));
        }
        a.push((ic::TRASH, "删除", Act::Remove));
        a
    }

    fn row(&mut self, ui: &mut Ui, v: &TaskView, p: &Pal) {
        let ctx = ui.ctx().clone();
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
        let sel = self.selected == Some(v.id);
        let hovered = ui.rect_contains_pointer(r);
        let pt = ui.painter().clone();
        if sel {
            pt.rect_filled(r, 0.0, p.sel);
        } else if hovered {
            pt.rect_filled(r, 0.0, p.hover);
        }
        self.paint_file(&ctx, &pt, pos2(r.left() + 30.0, r.center().y), 26.0, &v.filename, done_path(v), p, 1.0);
        let x0 = r.left() + 52.0;
        let right_w = 190.0;
        let name_w = (r.width() - 52.0 - right_w - 24.0).max(80.0);
        let q = self.input.trim().to_owned();
        text_line_hl(&pt, pos2(x0, r.top() + 15.0), Align2::LEFT_CENTER, &v.filename, &q, font(13.5), p.text, p, name_w);
        let (meta, mc) = meta_line(v, p);
        text_line(&pt, pos2(x0, r.top() + 32.0), Align2::LEFT_CENTER, meta, font(12.0), mc, name_w);

        let rx = r.right() - 14.0;
        let mut act = None;
        if hovered {
            let mut bx = rx;
            for (i, (icon, tip, a)) in Self::row_actions(v).into_iter().enumerate().rev() {
                let br = Rect::from_min_size(pos2(bx - 26.0, r.center().y - 13.0), vec2(26.0, 26.0));
                if icon_at(ui, br, resp.id.with(i), icon, tip, p) {
                    act = Some(a);
                }
                bx -= 28.0;
            }
        } else {
            match v.status {
                Status::Running | Status::Queued | Status::Paused if v.size.is_some() => {
                    let f = seg_bar(&ctx, &pt, Rect::from_min_size(pos2(rx - 160.0, r.top() + 29.0), vec2(160.0, 4.0)), v, p, Id::new(("row", v.id)));
                    pt.text(pos2(rx, r.top() + 15.0), Align2::RIGHT_CENTER, format!("{:.0}%", f * 100.0), font(12.0), p.weak);
                }
                Status::Completed => {
                    let key = Id::new(("row", v.id));
                    // Just finished: keep the bar for its completion animation, then cross-fade to the time.
                    let t = since_on(&ctx, key.with("done"), !v.file_missing).unwrap_or(f32::INFINITY);
                    if t < 1.3 {
                        seg_bar(&ctx, &pt, Rect::from_min_size(pos2(rx - 160.0, r.top() + 29.0), vec2(160.0, 4.0)), v, p, key);
                        pt.text(pos2(rx, r.top() + 15.0), Align2::RIGHT_CENTER, "完成", font(12.0), p.green.gamma_multiply(ease_out_cubic(t / 0.4)));
                    } else if let Some(f) = v.finished_at {
                        let a = ease_out_cubic((t - 1.3) / 0.35);
                        pt.text(pos2(rx, r.center().y), Align2::RIGHT_CENTER, format!("完成于 {}", when(f)), font(12.0), p.weak.gamma_multiply(a));
                    }
                }
                _ => {}
            }
        }
        if resp.double_clicked() {
            self.selected = None;
            if v.status == Status::Completed && !v.file_missing {
                act = Some(Act::Open);
            }
        } else if resp.clicked() {
            self.selected = Some(v.id);
        }
        let mut menu = None;
        resp.context_menu(|ui| {
            ui.set_width(140.0);
            let mut items: Vec<(&str, &str, Act)> = Vec::new();
            if v.status == Status::Completed && !v.file_missing {
                items.push((ic::FILE, "打开", Act::Open));
                items.push((ic::FOLDER_OPEN, "打开所在文件夹", Act::Reveal));
            } else {
                items.extend(Self::row_actions(v).into_iter().filter(|a| !matches!(a.2, Act::Remove)));
            }
            items.push((ic::LINK, "复制链接", Act::CopyLink));
            items.push((ic::TRASH, "删除", Act::Remove));
            ui.spacing_mut().item_spacing.y = 1.0;
            for (_, label, a) in items {
                if menu_item(ui, label, matches!(a, Act::Remove), p) {
                    menu = Some(a);
                    ui.close();
                }
            }
        });
        if let Some(a) = act.or(menu) {
            self.act(&ctx, v, a);
        }
    }

    fn detail(&mut self, ui: &mut Ui, v: &TaskView, p: &Pal) {
        let ctx = ui.ctx().clone();
        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 35.0), Sense::hover());
        ui.painter().text(pos2(hr.left() + 16.0, hr.center().y), Align2::LEFT_CENTER, "详情", bold(12.0), p.text);
        if icon_at(ui, Rect::from_min_size(pos2(hr.right() - 34.0, hr.center().y - 12.0), vec2(24.0, 24.0)), Id::new("detail_close"), ic::X, "关闭", p) {
            self.selected = None;
        }
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 2, bottom: 16 }).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(&v.filename).font(bold(14.0)).color(p.text)).wrap());
                let (m, c) = meta_line(v, p);
                ui.add(egui::Label::new(RichText::new(m).size(12.0).color(c)).wrap());
                if v.status == Status::Failed && v.error.is_some() {
                    ui.add_space(4.0);
                    let l = ui.add(egui::Label::new(RichText::new("查看错误报告").size(12.0).color(p.accent)).sense(Sense::click()));
                    if l.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.err_report = Some(v.id);
                    }
                }
                ui.add_space(8.0);
                let (br, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
                seg_bar(&ctx, ui.painter(), br, v, p, Id::new(("detail", v.id)));
                ui.add_space(14.0);
                let mut act = None;
                let vw = (ui.available_width() - 76.0).max(60.0);
                egui::Grid::new("props").num_columns(2).spacing(vec2(14.0, 8.0)).min_col_width(56.0).show(ui, |ui| {
                    let k = |ui: &mut Ui, s: &str| {
                        ui.label(RichText::new(s).size(12.0).color(p.weak));
                    };
                    let val = |ui: &mut Ui, s: String| {
                        ui.allocate_ui(vec2(vw, 18.0), |ui| {
                            ui.add(egui::Label::new(RichText::new(s).size(12.0).color(p.text)).truncate());
                        });
                    };
                    k(ui, "大小");
                    val(ui, v.size.map(bytes).unwrap_or_else(|| "未知".into()));
                    ui.end_row();
                    if v.status != Status::Completed {
                        k(ui, "已下载");
                        val(ui, format!("{}   {:.1}%", bytes(v.downloaded), frac(v) * 100.0));
                        ui.end_row();
                    }
                    if v.status == Status::Running {
                        k(ui, "速度");
                        val(ui, speed(v.speed));
                        ui.end_row();
                        k(ui, "连接");
                        val(ui, format!("{} / {}", v.connections, v.target));
                        ui.end_row();
                    }
                    if v.avg_speed > 0.0 {
                        k(ui, "平均速度");
                        val(ui, speed(v.avg_speed));
                        ui.end_row();
                    }
                    k(ui, "分段");
                    val(ui, if v.ranges { format!("支持，切分 {} 次", v.splits) } else { "不支持".into() });
                    ui.end_row();
                    if let Some(r) = &v.route {
                        k(ui, "线路");
                        val(ui, r.clone());
                        ui.end_row();
                    }
                    k(ui, "添加于");
                    val(ui, when(v.created_at));
                    ui.end_row();
                    k(ui, "保存到");
                    ui.allocate_ui_with_layout(vec2(vw, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.spacing_mut().button_padding = vec2(2.0, 0.0);
                        if ui.add(egui::Button::new(RichText::new(ic::FOLDER_OPEN).font(ifont(13.0))).frame_when_inactive(false).small()).tip("打开所在文件夹").clicked() {
                            act = Some(Act::Reveal);
                        }
                        ui.add(egui::Label::new(RichText::new(v.path.as_ref().unwrap_or(&v.dir).display().to_string()).size(12.0).color(p.text)).truncate());
                    });
                    ui.end_row();
                    k(ui, "链接");
                    ui.allocate_ui_with_layout(vec2(vw, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.spacing_mut().button_padding = vec2(2.0, 0.0);
                        if ui.add(egui::Button::new(RichText::new(ic::COPY).font(ifont(13.0))).frame_when_inactive(false).small()).tip("复制链接").clicked() {
                            act = Some(Act::CopyLink);
                        }
                        ui.add(egui::Label::new(RichText::new(&v.url).size(12.0).color(p.text)).truncate()).tip(&v.url);
                    });
                    ui.end_row();
                });
                if let Some(a) = act {
                    self.act(&ctx, v, a);
                }
                if !v.host.is_empty() {
                    ui.add_space(16.0);
                    ui.label(RichText::new("下载服务器").font(bold(12.0)).color(p.text));
                    ui.add_space(6.0);
                    self.server_row(ui, v, p);
                }
            });
        });
    }

    fn page_title(ui: &mut Ui, title: &str, n: Option<usize>, actions: &[(&str, &str)], p: &Pal) -> Option<usize> {
        ui.add_space(6.0);
        header(ui, title, n, None, actions, p).1
    }

    fn trash(&mut self, ui: &mut Ui, p: &Pal) {
        let ctx = ui.ctx().clone();
        let actions: &[(&str, &str)] = if self.deleted.is_empty() { &[] } else { &[(ic::BROOM, "清空回收站")] };
        ui.add_space(6.0);
        let (_, _, btns) = header(ui, "回收站", Some(self.deleted.len()), None, actions, p);
        if let Some(b) = btns.first() {
            let mut pick = None;
            egui::Popup::menu(b).show(|ui| {
                ui.set_width(190.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                for (mode, label, danger) in [
                    (RemoveMode::KeepFiles, "只清空记录，保留文件", false),
                    (RemoveMode::TrashFiles, "文件移到系统回收站", false),
                    (RemoveMode::DeleteFiles, "彻底删除文件", true),
                ] {
                    if menu_item(ui, label, danger, p) {
                        pick = Some(mode);
                        ui.close();
                    }
                }
            });
            if let Some(mode) = pick {
                self.engine.clear_deleted(mode);
                self.last_refresh = None;
            }
        }
        if self.deleted.is_empty() {
            ui.centered_and_justified(|ui| ui.label(RichText::new("回收站是空的").color(p.weak)));
            return;
        }
        let list = self.deleted.clone();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for d in &list {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
                let hovered = ui.rect_contains_pointer(r);
                let pt = ui.painter().clone();
                if hovered {
                    pt.rect_filled(r, 0.0, p.hover);
                }
                self.paint_file(&ui.ctx().clone(), &pt, pos2(r.left() + 30.0, r.center().y), 26.0, &d.rec.filename, None, p, 0.7);
                let w = (r.width() - 52.0 - 100.0).max(80.0);
                text_line(&pt, pos2(r.left() + 52.0, r.top() + 15.0), Align2::LEFT_CENTER, &d.rec.filename, font(13.5), p.text, w);
                let size = d.rec.size.map(bytes).unwrap_or_default();
                let mode = match d.mode {
                    RemoveMode::KeepFiles => "保留文件",
                    RemoveMode::TrashFiles => "文件在系统回收站",
                    RemoveMode::DeleteFiles => "文件已删除",
                };
                text_line(&pt, pos2(r.left() + 52.0, r.top() + 32.0), Align2::LEFT_CENTER, format!("{size}   {mode}   {}删除", ago(d.deleted_at)), font(12.0), p.weak, w);
                if hovered {
                    let b2 = Rect::from_min_size(pos2(r.right() - 40.0, r.center().y - 13.0), vec2(26.0, 26.0));
                    let b1 = b2.translate(vec2(-28.0, 0.0));
                    if icon_at(ui, b1, resp.id.with("restore"), ic::ARROW_COUNTER_CLOCKWISE, "恢复", p) {
                        if !self.engine.restore(d.rec.id) {
                            self.toast("无法恢复", true);
                        }
                        self.last_refresh = None;
                    }
                    if icon_at(ui, b2, resp.id.with("purge"), ic::X, "彻底删除", p) {
                        self.engine.purge_deleted(Some(d.rec.id));
                        self.last_refresh = None;
                    }
                }
                let _ = &ctx;
            }
        });
    }

    fn extension_page(&mut self, ui: &mut Ui, p: &Pal) {
        Self::page_title(ui, "浏览器插件", None, &[], p);
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(500));
        let (mut act, mut copy, mut open, mut tab, mut copy_page) = (None, false, false, None, None);
        let card = if p.dark { rgb(37, 37, 38) } else { rgb(250, 250, 250) };
        let browsers = self.browsers.clone();
        let icons: Vec<_> = browsers.iter().map(|b| self.file_tex(&ctx, "b.exe", Some(&b.exe))).collect();
        let sel = self.ext_tab.min(browsers.len().saturating_sub(1));
        let on = browsers.get(sel).is_some_and(|b| self.ext_connected(b.key));
        let since = self.ext_guide.as_ref().filter(|g| g.browser == sel).map(|g| g.since);
        let linked = since.is_some_and(|s| browsers.get(sel).and_then(|b| self.engine.browser_last_seen(b.key)).is_some_and(|t| t >= s));
        let path = self.engine.data_dir().join("extension");
        let path_s = path.display().to_string();
        let icon_at = |pt: &Painter, i: usize, c: Pos2, size: f32, alpha: f32| match icons.get(i).cloned().flatten() {
            Some(t) => {
                pt.image(t.id(), Rect::from_center_size(c, vec2(size, size)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(alpha));
            }
            None => {
                pt.text(c, Align2::CENTER_CENTER, ic::GLOBE, ifont(size * 0.85), p.text.gamma_multiply(alpha));
            }
        };
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin::symmetric(17, 10)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let w = ui.available_width().min(680.0);
                let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
                text_line(ui.painter(), r.left_center(), Align2::LEFT_CENTER, "安装插件之后，下载就放心交给 GaGaDown 吧！", font(12.5), p.weak, w);
                ui.add_space(10.0);
                let Some(b) = browsers.get(sel) else {
                    let (r, _) = ui.allocate_exact_size(vec2(w, 64.0), Sense::hover());
                    let pt = ui.painter().clone();
                    pt.rect(r, CornerRadius::same(8), card, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
                    text_line(&pt, pos2(r.left() + 16.0, r.center().y), Align2::LEFT_CENTER, "没有找到 Chrome 或 Edge", font(13.5), p.text, w - 160.0);
                    let br = Rect::from_min_size(pos2(r.right() - 130.0, r.center().y - 15.0), vec2(114.0, 30.0));
                    if btn_at(ui, br, Id::new("ext-folder"), "打开插件文件夹", false, p) {
                        act = Some(usize::MAX);
                    }
                    return;
                };

                let (bar, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
                ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, p.line));
                let mut x = bar.left();
                for (i, bb) in browsers.iter().enumerate() {
                    let tw = ui.painter().layout_no_wrap(bb.name.to_string(), font(13.0), p.text).size().x;
                    let r = Rect::from_min_max(pos2(x, bar.top()), pos2(x + 26.0 + tw, bar.bottom()));
                    x = r.right() + 28.0;
                    let resp = ui.interact(r.expand2(vec2(8.0, 0.0)), Id::new(("ext-tab", i)), Sense::click());
                    let t = ui.ctx().animate_bool_with_time(resp.id, i == sel, 0.16);
                    let h = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.12);
                    let pt = ui.painter();
                    if h > 0.0 && i != sel {
                        pt.rect_filled(r.expand2(vec2(8.0, 0.0)).shrink2(vec2(0.0, 6.0)), CornerRadius::same(6), p.hover.gamma_multiply(h));
                    }
                    let cy = r.center().y - 1.0;
                    icon_at(pt, i, pos2(r.left() + 9.0, cy), 18.0, 0.55 + 0.45 * t.max(h));
                    pt.text(pos2(r.left() + 26.0, cy), Align2::LEFT_CENTER, bb.name, font(13.0), p.weak.lerp_to_gamma(p.text, t.max(h * 0.6)));
                    if t > 0.0 {
                        let u = Rect::from_center_size(pos2(r.center().x, bar.bottom() - 1.0), vec2(r.width() * t, 2.0));
                        pt.rect_filled(u, CornerRadius::same(1), p.text.gamma_multiply(t));
                    }
                    if resp.clicked() {
                        tab = Some(i);
                    }
                    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                }
                ui.add_space(16.0);

                let (r, _) = ui.allocate_exact_size(vec2(w, 72.0), Sense::hover());
                let pt = ui.painter().clone();
                pt.rect(r, CornerRadius::same(8), card, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
                icon_at(&pt, sel, pos2(r.left() + 36.0, r.center().y), 40.0, 1.0);
                let tx = r.left() + 68.0;
                text_line(&pt, pos2(tx, r.center().y - 10.0), Align2::LEFT_CENTER, b.name, bold(15.0), p.text, w - 200.0);
                let (st, sc) = if on { ("已连接", p.green) } else { ("未连接", p.weak) };
                text_line(&pt, pos2(tx, r.center().y + 11.0), Align2::LEFT_CENTER, st, font(12.0), sc, 200.0);
                let br = Rect::from_min_size(pos2(r.right() - 112.0, r.center().y - 15.0), vec2(96.0, 30.0));
                if btn_at(ui, br, Id::new(("ext-install", sel)), if on { "重新安装" } else { "安装插件" }, !on, p) {
                    act = Some(sel);
                }

                let (dev, load) = if b.key == "edge" { ("开发人员模式", "加载解压缩的扩展") } else { ("开发者模式", "加载已解压的扩展程序") };
                ui.add_space(24.0);
                let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
                text_line(ui.painter(), r.left_center(), Align2::LEFT_CENTER, "安装步骤", bold(13.5), p.text, w);
                let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
                text_line(ui.painter(), r.left_center(), Align2::LEFT_CENTER, "点“安装插件”会打开浏览器，并复制好扩展页地址", font(12.0), p.weak, w);
                ui.add_space(8.0);
                let steps = [
                    format!("在浏览器地址栏粘贴 {} 并回车", b.page),
                    format!("在扩展页打开“{dev}”开关"),
                    format!("点击“{load}”"),
                    "复制下面的插件路径，粘贴到弹出窗口的地址栏，回车后点“选择文件夹”".to_string(),
                ];
                let ind = 30.0;
                for (n, s) in steps.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
                    let pt = ui.painter();
                    let c = pos2(r.left() + 10.0, r.center().y);
                    pt.circle_filled(c, 10.0, p.accent.gamma_multiply(0.14));
                    pt.text(c, Align2::CENTER_CENTER, (n + 1).to_string(), font(11.5), p.accent);
                    let tw = if n == 0 { w - ind - 70.0 } else { w - ind };
                    text_line(pt, pos2(r.left() + ind, r.center().y), Align2::LEFT_CENTER, s, font(13.0), p.text, tw);
                    if n == 0 {
                        let cb = Rect::from_min_size(pos2(r.right() - 58.0, r.center().y - 14.0), vec2(58.0, 28.0));
                        if btn_at(ui, cb, Id::new("ext-copy-page"), "复制", false, p) {
                            copy_page = Some(b.page);
                        }
                    }
                }
                ui.add_space(4.0);
                let (row, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
                let r = Rect::from_min_max(pos2(row.left() + ind, row.top()), row.max);
                ui.painter().rect(r, CornerRadius::same(6), card, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
                let ob = Rect::from_min_size(pos2(r.right() - 98.0, r.top() + 4.0), vec2(94.0, 28.0));
                let cb = Rect::from_min_size(pos2(ob.left() - 62.0, r.top() + 4.0), vec2(58.0, 28.0));
                text_line(ui.painter(), pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, &path_s, FontId::monospace(11.5), p.text, cb.left() - r.left() - 24.0);
                copy = btn_at(ui, cb, Id::new("ext-copy"), "复制", false, p);
                open = btn_at(ui, ob, Id::new("ext-open"), "打开文件夹", false, p);
                if since.is_none() {
                    return;
                }
                ui.add_space(16.0);
                let (row, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
                let x0 = row.left() + ind;
                if linked {
                    ui.painter().text(pos2(x0, row.center().y), Align2::LEFT_CENTER, ic::CHECK_CIRCLE, ifont(15.0), p.green);
                    text_line(ui.painter(), pos2(x0 + 22.0, row.center().y), Align2::LEFT_CENTER, "安装成功，插件已连接", font(13.0), p.green, w - ind - 30.0);
                } else {
                    ui.put(Rect::from_center_size(pos2(x0 + 7.0, row.center().y), vec2(14.0, 14.0)), egui::Spinner::new().size(14.0).color(p.weak));
                    text_line(ui.painter(), pos2(x0 + 22.0, row.center().y), Align2::LEFT_CENTER, "等待插件连接…", font(12.5), p.weak, w - ind - 30.0);
                }
            });
        });
        if let Some(i) = tab {
            self.ext_tab = i;
        }
        if let Some(i) = act {
            self.install_extension(&ctx, i);
        }
        if copy {
            ctx.copy_text(path_s);
            self.toast("路径已复制", false);
        }
        if let Some(u) = copy_page {
            ctx.copy_text(u.to_string());
            self.toast("地址已复制", false);
        }
        if open {
            let _ = unpack_extension(self.engine.data_dir());
            open_path(&path);
        }
    }

    fn cache_page(&mut self, ui: &mut Ui, p: &Pal) {
        if Self::page_title(ui, "缓存", None, &[(ic::ARROW_CLOCKWISE, "刷新")], p) == Some(0) {
            self.cache = Some(self.engine.cache_report());
        }
        let Some(rep) = self.cache.clone() else { return };
        egui::Frame::new().inner_margin(Margin::symmetric(20, 10)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let stat = |ui: &mut Ui, k: &str, v: String| {
                    ui.vertical(|ui| {
                        ui.set_min_width(120.0);
                        ui.label(RichText::new(k).size(11.5).color(p.weak));
                        ui.label(RichText::new(v).font(bold(16.0)).color(p.text));
                    });
                    ui.add_space(16.0);
                };
                stat(ui, "下载中", bytes(rep.active_bytes));
                stat(ui, "回收站", bytes(rep.deleted_bytes));
                stat(ui, "孤儿文件", bytes(rep.orphan_bytes));
                if let Some(f) = rep.free_space {
                    stat(ui, "磁盘剩余", bytes(f));
                }
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if rep.orphan_bytes > 0 && btn(ui, "清理孤儿文件", true, p) {
                    let n = self.engine.clean_cache(true, false);
                    self.toast(format!("已释放 {}", bytes(n)), false);
                    self.cache = Some(self.engine.cache_report());
                }
                if rep.deleted_bytes > 0 && btn(ui, "清理回收站缓存", false, p) {
                    let n = self.engine.clean_cache(false, true);
                    self.toast(format!("已释放 {}", bytes(n)), false);
                    self.cache = Some(self.engine.cache_report());
                }
                if rep.missing_files > 0 && btn(ui, &format!("移除文件丢失的任务 ({})", rep.missing_files), false, p) {
                    self.engine.prune_missing();
                    self.cache = Some(self.engine.cache_report());
                    self.last_refresh = None;
                }
            });
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for it in &rep.items {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                let pt = ui.painter();
                if ui.rect_contains_pointer(r) {
                    pt.rect_filled(r, 0.0, p.hover);
                }
                let (k, c) = match it.kind {
                    CacheKind::Active => ("下载中", p.accent),
                    CacheKind::Deleted => ("回收站", p.orange),
                    CacheKind::Orphan => ("孤儿", p.red),
                };
                pt.text(pos2(r.left() + 20.0, r.center().y), Align2::LEFT_CENTER, k, font(12.0), c);
                pt.text(pos2(r.left() + 80.0, r.center().y), Align2::LEFT_CENTER, bytes(it.bytes), font(12.0), p.text);
                text_line(pt, pos2(r.left() + 170.0, r.center().y), Align2::LEFT_CENTER, it.path.display().to_string(), font(12.0), p.weak, r.width() - 190.0);
            }
        });
    }

    fn about(&mut self, ui: &mut Ui, p: &Pal) {
        ui.add_space(28.0);
        ui.label(RichText::new("关于").font(bold(14.5)).color(p.text));
        ui.add_space(8.0);
        let card = if p.dark { rgb(37, 37, 38) } else { rgb(250, 250, 250) };
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::hover());
        let pt = ui.painter().clone();
        pt.rect(r, CornerRadius::same(8), card, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
        let tex = self.icon_tex(ui.ctx());
        let ir = Rect::from_center_size(pos2(r.left() + 70.0, r.center().y), vec2(88.0, 88.0));
        pt.image(tex.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        let x = r.left() + 140.0;
        let w = r.right() - x - 20.0;
        let name = text_line(&pt, pos2(x, r.top() + 30.0), Align2::LEFT_CENTER, "GaGaDown", bold(20.0), p.text, w);
        text_line(&pt, pos2(name.right() + 10.0, r.top() + 32.0), Align2::LEFT_CENTER, format!("v{}", env!("CARGO_PKG_VERSION")), font(12.0), p.weak, 80.0);
        let verse = p.text.gamma_multiply(0.78);
        text_line(&pt, pos2(x, r.top() + 58.0), Align2::LEFT_CENTER, "To see a World in a Grain of Sand", font(13.0), verse, w);
        text_line(&pt, pos2(x, r.top() + 77.0), Align2::LEFT_CENTER, "And a Heaven in a Wild Flower", font(13.0), verse, w);
        text_line(&pt, pos2(x, r.top() + 97.0), Align2::LEFT_CENTER, "— William Blake", font(11.5), p.weak, w);
        let y = r.top() + 126.0;
        let mut lx = x;
        let repo = "https://github.com/shuakami/gagadown";
        let dir = self.engine.data_dir().to_path_buf();
        for (key, label, tip) in [("about_dir", "打开数据目录", "设置、任务记录和日志都在这里"), ("about_repo", "GitHub", repo)] {
            let lr = text_line(&pt, pos2(lx, y), Align2::LEFT_CENTER, label, font(12.5), p.accent, w);
            let resp = ui.interact(lr.expand(3.0), Id::new(key), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).tip(tip);
            if resp.hovered() {
                pt.hline(lr.x_range(), lr.bottom() + 1.0, Stroke::new(1.0, p.accent.gamma_multiply(0.6)));
            }
            if resp.clicked() {
                if key == "about_repo" {
                    open_path(Path::new(repo));
                } else {
                    open_path(&dir);
                }
            }
            lx = lr.right() + 18.0;
        }
        let done = self.views.iter().filter(|v| v.status == Status::Completed);
        let (n, total) = done.fold((0usize, 0u64), |(n, t), v| (n + 1, t + v.size.unwrap_or(0)));
        pt.text(pos2(r.right() - 20.0, y), Align2::RIGHT_CENTER, format!("已完成 {n} 个下载，共 {}", bytes(total)), font(12.0), p.weak);
    }

    fn settings_page(&mut self, ui: &mut Ui, p: &Pal) {
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let avail = ui.available_width();
            let w = (avail - 48.0).min(680.0);
            ui.horizontal_top(|ui| {
                ui.add_space(((avail - w) / 2.0).max(24.0));
                ui.vertical(|ui| {
                    ui.set_width(w);
                    ui.add_space(36.0);
                    ui.label(RichText::new("设置").font(bold(22.0)).color(p.text));
                    let d = &mut self.draft;
                    group(ui, p, "常规", |ui, first| {
                        srow(ui, p, first, "开机自启", "", 52.0, |ui| toggle(ui, &mut d.launch_at_login, p));
                        srow(ui, p, first, "开机时静默启动", "开机自启时只在托盘运行，不弹出主窗口", 56.0, |ui| {
                            ui.add_enabled_ui(d.launch_at_login, |ui| toggle(ui, &mut d.start_minimized, p));
                        });
                    });
                    group(ui, p, "下载", |ui, first| {
                        srow(ui, p, first, "下载目录", "", 52.0, |ui| {
                            if btn(ui, "更改", false, p) {
                                if let Some(x) = rfd::FileDialog::new().set_directory(&d.download_dir).pick_folder() {
                                    d.download_dir = x;
                                }
                            }
                            ui.add(egui::Label::new(RichText::new(d.download_dir.display().to_string()).size(13.0).color(p.text)).truncate());
                        });
                        srow(ui, p, first, "缓存目录", "", 52.0, |ui| {
                            if btn(ui, "更改", false, p) {
                                if let Some(x) = rfd::FileDialog::new().set_directory(d.cache_dir.as_ref().unwrap_or(&d.download_dir)).pick_folder() {
                                    d.cache_dir = Some(x);
                                }
                            }
                            if d.cache_dir.is_some() && btn(ui, "默认", false, p) {
                                d.cache_dir = None;
                            }
                            let (t, c) = match &d.cache_dir {
                                Some(x) => (x.display().to_string(), p.text),
                                None => ("下载目录".into(), p.weak),
                            };
                            ui.add(egui::Label::new(RichText::new(t).size(13.0).color(c)).truncate());
                        });
                        srow(ui, p, first, "同时下载任务", "超出的任务会排队", 56.0, |ui| {
                            ui.add(egui::DragValue::new(&mut d.max_concurrent_tasks).range(1..=64));
                        });
                        srow(ui, p, first, "初始连接数", "每个任务开始时的连接数", 56.0, |ui| {
                            ui.add(egui::DragValue::new(&mut d.initial_connections).range(1..=256));
                        });
                        srow(ui, p, first, "单任务最大连接数", "自动调节的上限", 56.0, |ui| {
                            ui.add(egui::DragValue::new(&mut d.max_connections_per_task).range(1..=256));
                        });
                        srow(ui, p, first, "全局最大连接数", "所有任务合计", 56.0, |ui| {
                            ui.add(egui::DragValue::new(&mut d.max_connections_total).range(1..=4096));
                        });
                        srow(ui, p, first, "最小分段", "小于这个大小不再切分", 56.0, |ui| {
                            let mut kb = d.min_split_size / 1024;
                            if ui.add(egui::DragValue::new(&mut kb).range(64..=65536).suffix(" KB")).changed() {
                                d.min_split_size = kb * 1024;
                            }
                        });
                        srow(ui, p, first, "限速", "0 为不限", 56.0, |ui| {
                            let mut kb = d.speed_limit / 1024;
                            if ui.add(egui::DragValue::new(&mut kb).range(0..=10_000_000).suffix(" KB/s")).changed() {
                                d.speed_limit = kb * 1024;
                            }
                        });
                        srow(ui, p, first, "分段重试次数", "", 52.0, |ui| {
                            ui.add(egui::DragValue::new(&mut d.segment_retries).range(0..=1000));
                        });
                        srow(ui, p, first, "任务自动重试次数", "", 52.0, |ui| {
                            ui.add(egui::DragValue::new(&mut d.task_auto_retries).range(0..=100));
                        });
                    });
                    group(ui, p, "网络", |ui, first| {
                        srow(ui, p, first, "代理模式", "", 52.0, |ui| {
                            egui::ComboBox::from_id_salt("pmode").width(230.0).selected_text(d.proxy.mode.label()).show_ui(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                for m in ProxyMode::ALL {
                                    if menu_choice(ui, m.label(), d.proxy.mode == m, p) {
                                        d.proxy.mode = m;
                                        ui.close();
                                    }
                                }
                            });
                        });
                        srow(ui, p, first, "代理地址", "每行一个", 92.0, |ui| {
                            ui.add(egui::TextEdit::multiline(&mut self.draft_proxies).hint_text("http://127.0.0.1:7890").desired_rows(3).desired_width(260.0));
                        });
                        srow(ui, p, first, "自动探测本机代理", "扫描 Clash、v2rayN 等常用端口", 56.0, |ui| toggle(ui, &mut d.proxy.auto_detect, p));
                        srow(ui, p, first, "使用系统代理", "", 52.0, |ui| toggle(ui, &mut d.proxy.use_system, p));
                        srow(ui, p, first, "直连等待", "超时后同时尝试代理", 56.0, |ui| {
                            ui.add(egui::DragValue::new(&mut d.proxy.hedge_delay_ms).range(0..=30000).suffix(" ms"));
                        });
                        let found = self.engine.detected_proxies();
                        let desc = if found.is_empty() { "无".to_string() } else { found.join("  ") };
                        srow(ui, p, first, "已发现代理", &desc, 56.0, |ui| {
                            if self.engine.is_detecting() {
                                ui.spinner();
                            }
                        });
                        srow(ui, p, first, "User-Agent", "", 52.0, |ui| {
                            ui.add(egui::TextEdit::singleline(&mut d.user_agent).desired_width(300.0));
                        });
                    });
                    group(ui, p, "浏览器", |ui, first| {
                        srow(ui, p, first, "接管最小文件", "小于这个大小交给浏览器下载", 56.0, |ui| {
                            let mut kb = d.takeover_min_size / 1024;
                            if ui.add(egui::DragValue::new(&mut kb).range(0..=10_000_000).suffix(" KB")).changed() {
                                d.takeover_min_size = kb * 1024;
                            }
                        });
                        srow(ui, p, first, "本地端口", "重启后生效", 56.0, |ui| {
                            ui.add(egui::DragValue::new(&mut d.api_port).range(1024..=65535));
                        });
                    });
                    group(ui, p, "清理", |ui, first| {
                        srow(ui, p, first, "回收站保留", "", 52.0, |ui| {
                            ui.add(egui::DragValue::new(&mut d.deleted_retention_days).range(0..=3650).suffix(" 天"));
                        });
                        srow(ui, p, first, "孤儿文件自动清理", "0 为不清理", 56.0, |ui| {
                            ui.add(egui::DragValue::new(&mut d.orphan_auto_clean_days).range(0..=3650).suffix(" 天"));
                        });
                        srow(ui, p, first, "完成后刷盘", "确保文件完整写入磁盘", 56.0, |ui| toggle(ui, &mut d.sync_on_complete, p));
                    });
                    self.about(ui, p);
                    ui.add_space(48.0);
                });
            });
        });

        let mut cand = self.draft.clone();
        cand.proxy.proxies = self.draft_proxies.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        let next = serde_json::to_string(&cand).unwrap_or_default();
        if next == serde_json::to_string(&self.engine.settings()).unwrap_or_default() {
            self.settings_dirty = None;
            return;
        }
        if self.settings_dirty.as_ref().is_none_or(|(s, _)| *s != next) {
            self.settings_dirty = Some((next, Instant::now()));
        }
        if self.settings_dirty.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_millis(600)) {
            self.settings_dirty = None;
            win::set_autostart(cand.launch_at_login);
            if let Err(e) = self.engine.set_settings(cand) {
                self.toast(format!("{e}"), true);
                self.draft = self.engine.settings();
                self.draft_proxies = self.draft.proxy.proxies.join("\n");
            }
        }
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    fn error_modal(&mut self, ctx: &egui::Context, p: &Pal) {
        let Some(id) = self.err_report else { return };
        let found = self.views.iter().find(|v| v.id == id).and_then(|v| v.error.as_ref().map(|e| (v.filename.clone(), e.summary(), error_report(v, e))));
        let Some((name, summary, report)) = found else {
            self.err_report = None;
            self.er_fade = Fade::default();
            return;
        };
        let Some(a) = self.er_fade.step(ctx) else {
            self.err_report = None;
            return;
        };
        let closing = self.er_fade.closing;
        let mut close = false;
        let fill = if p.dark { rgb(37, 37, 38) } else { rgb(255, 255, 255) };
        let frame = egui::Frame::new().fill(fill).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(20)).stroke(Stroke::new(1.0, p.line));
        let mid = Id::new("err_report");
        let m = egui::Modal::new(mid).area(egui::Modal::default_area(mid).fade_in(false)).backdrop_color(Color32::from_black_alpha((100.0 * a) as u8)).frame(frame).show(ctx, |ui| {
            ui.set_opacity(a);
            ui.set_width(520.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new("错误报告").font(bold(15.0)).color(p.text));
            ui.add_space(2.0);
            ui.add(egui::Label::new(RichText::new(name).size(12.0).color(p.weak)).truncate());
            ui.add_space(12.0);
            ui.add(egui::Label::new(RichText::new(summary).size(13.5).color(p.red)).wrap());
            ui.add_space(10.0);
            egui::Frame::new().fill(p.text.gamma_multiply(0.04)).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(280.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(&report).monospace().size(11.5).color(p.text)).wrap().selectable(true));
                });
            });
            ui.add_space(16.0);
            let (row, _) = ui.allocate_exact_size(vec2(520.0, 30.0), Sense::hover());
            let copy = Rect::from_min_size(pos2(row.right() - 88.0, row.top()), vec2(88.0, 30.0));
            let shut = Rect::from_min_size(pos2(copy.left() - 84.0, row.top()), vec2(76.0, 30.0));
            let cr = ui.interact(copy, Id::new("er-copy"), Sense::click());
            ui.painter().rect_filled(copy, CornerRadius::same(6), if cr.hovered() { blend(p.accent, Color32::BLACK, 0.88) } else { p.accent });
            ui.painter().text(copy.center(), Align2::CENTER_CENTER, "复制报告", font(13.0), Color32::WHITE);
            let sr = ui.interact(shut, Id::new("er-close"), Sense::click());
            ui.painter().rect_filled(shut, CornerRadius::same(6), p.text.gamma_multiply(if sr.hovered() { 0.1 } else { 0.06 }));
            ui.painter().text(shut.center(), Align2::CENTER_CENTER, "关闭", font(13.0), p.text);
            if cr.clicked() && !closing {
                ui.ctx().copy_text(report.clone());
            }
            close = sr.clicked();
        });
        if !closing && (close || m.should_close()) {
            self.er_fade.close(ctx);
        }
    }

    fn remove_modal(&mut self, ctx: &egui::Context, p: &Pal) {
        let Some(id) = self.removing else { return };
        let Some(a) = self.rm_fade.step(ctx) else {
            self.removing = None;
            return;
        };
        let closing = self.rm_fade.closing;
        let name = self.views.iter().find(|v| v.id == id).map(|v| v.filename.clone()).unwrap_or_default();
        let mut close = false;
        let fill = if p.dark { rgb(37, 37, 38) } else { rgb(255, 255, 255) };
        let frame = egui::Frame::new().fill(fill).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(20)).stroke(Stroke::new(1.0, p.line));
        let mid = Id::new("remove");
        let m = egui::Modal::new(mid).area(egui::Modal::default_area(mid).fade_in(false)).backdrop_color(Color32::from_black_alpha((100.0 * a) as u8)).frame(frame).show(ctx, |ui| {
            ui.set_opacity(a);
            ui.set_width(340.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new("删除任务").font(bold(15.0)).color(p.text));
            ui.add_space(2.0);
            ui.add(egui::Label::new(RichText::new(name).size(12.0).color(p.weak)).truncate());
            ui.add_space(14.0);
            for (mode, label) in [(RemoveMode::KeepFiles, "只删除任务，保留文件"), (RemoveMode::TrashFiles, "文件移到回收站"), (RemoveMode::DeleteFiles, "彻底删除文件")] {
                let (r, resp) = ui.allocate_exact_size(vec2(340.0, 34.0), Sense::click());
                let on = self.remove_mode == mode;
                if on {
                    ui.painter().rect_filled(r, CornerRadius::same(6), p.accent.gamma_multiply(0.12));
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(6), p.text.gamma_multiply(0.06));
                }
                ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, label, font(13.0), p.text);
                if on {
                    ui.painter().text(pos2(r.right() - 12.0, r.center().y), Align2::RIGHT_CENTER, ic::CHECK, ifont(14.0), p.accent);
                }
                if resp.clicked() && !closing {
                    self.remove_mode = mode;
                }
            }
            ui.add_space(18.0);
            let (row, _) = ui.allocate_exact_size(vec2(340.0, 30.0), Sense::hover());
            let ok = Rect::from_min_size(pos2(row.right() - 76.0, row.top()), vec2(76.0, 30.0));
            let cancel = Rect::from_min_size(pos2(ok.left() - 84.0, row.top()), vec2(76.0, 30.0));
            let danger = self.remove_mode == RemoveMode::DeleteFiles;
            let okr = ui.interact(ok, Id::new("rm-ok"), Sense::click());
            let base = if danger { p.red } else { p.accent };
            ui.painter().rect_filled(ok, CornerRadius::same(6), if okr.hovered() { blend(base, Color32::BLACK, 0.88) } else { base });
            ui.painter().text(ok.center(), Align2::CENTER_CENTER, "删除", font(13.0), Color32::WHITE);
            let cr = ui.interact(cancel, Id::new("rm-cancel"), Sense::click());
            ui.painter().rect_filled(cancel, CornerRadius::same(6), p.text.gamma_multiply(if cr.hovered() { 0.1 } else { 0.06 }));
            ui.painter().text(cancel.center(), Align2::CENTER_CENTER, "取消", font(13.0), p.text);
            if !closing && (okr.clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                let e = self.engine.clone();
                let mode = self.remove_mode;
                self.rt.spawn(async move { e.remove(id, mode).await });
                if self.selected == Some(id) {
                    self.selected = None;
                }
                close = true;
            }
            if cr.clicked() {
                close = true;
            }
        });
        if !closing && (close || m.should_close()) {
            self.rm_fade.close(ctx);
            self.last_refresh = None;
        }
    }

    fn show_toasts(&mut self, ctx: &egui::Context, p: &Pal) {
        const LIFE: f32 = 4.0;
        self.toasts.retain(|t| t.0.elapsed().as_secs_f32() < LIFE);
        if self.toasts.len() > 4 {
            let n = self.toasts.len() - 4;
            self.toasts.drain(..n);
        }
        if self.toasts.is_empty() {
            return;
        }
        let fill = if p.dark { rgb(44, 44, 46) } else { rgb(255, 255, 255) };
        let shadow = egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(if p.dark { 60 } else { 18 }) };
        let mut y = 30.0;
        let mut animating = false;
        for (t0, t, bad) in self.toasts.iter().rev() {
            let id = Id::new(("toast", t0, t.as_str()));
            let age = t0.elapsed().as_secs_f32();
            let a = ease_out_cubic((age / 0.22).min(1.0)).min(ease_in_out_cubic(((LIFE - age) / 0.22).clamp(0.0, 1.0)));
            animating |= a < 1.0;
            let oy = ctx.animate_value_with_time(id.with("y"), y, 0.2);
            animating |= (oy - y).abs() > 0.1;
            let slide = (1.0 - a) * 8.0;
            let r = egui::Area::new(id)
                .order(egui::Order::Foreground)
                .interactable(false)
                .anchor(Align2::RIGHT_BOTTOM, vec2(-14.0, -oy + slide))
                .show(ctx, |ui| {
                    ui.set_opacity(a);
                    egui::Frame::new()
                        .fill(fill)
                        .stroke(Stroke::new(1.0, p.line))
                        .shadow(shadow)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(11, 7))
                        .show(ui, |ui| {
                            ui.set_max_width(300.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 7.0;
                                let (icon, c) = if *bad { (ic::WARNING_CIRCLE, p.red) } else { (ic::CHECK_CIRCLE, p.accent) };
                                ui.label(RichText::new(icon).font(ifont(14.0)).color(c));
                                ui.add(egui::Label::new(RichText::new(t).size(12.5).color(p.text)).wrap());
                            });
                        });
                })
                .response
                .rect;
            y += r.height() + 8.0;
        }
        ctx.request_repaint_after(Duration::from_millis(if animating { 16 } else { 100 }));
    }

    fn popups(&mut self, ctx: &egui::Context, p: &Pal) {
        let events = self.engine.take_popups();
        if !events.is_empty() {
            let screen = ctx.input(|i| i.viewport().monitor_size).unwrap_or(vec2(1920.0, 1080.0));
            let next_pos = |n: usize| {
                let k = n as f32 * 28.0;
                pos2((screen.x - 460.0) / 2.0 + k, (screen.y - 210.0) / 2.0 - 80.0 + k)
            };
            for ev in events {
                match ev {
                    PopupEvent::Pending { key, filename, size, host } => {
                        if !self.popups.iter().any(|x| x.key == key) {
                            let pos = next_pos(self.popups.len());
                            self.popups.push(Pop { key, task: None, name: filename, size, host, pos, error: None });
                        }
                    }
                    PopupEvent::Bound { key, id } => {
                        self.last_refresh = None;
                        self.refresh();
                        if self.dismissed.remove(&key) {
                            let e = self.engine.clone();
                            self.rt.spawn(async move { e.remove(id, RemoveMode::DeleteFiles).await });
                        } else if let Some(x) = self.popups.iter_mut().find(|x| x.key == key) {
                            x.task = Some(id);
                        } else if !self.popups.iter().any(|x| x.task == Some(id)) {
                            let pos = next_pos(self.popups.len());
                            self.popups.push(Pop { key, task: Some(id), name: String::new(), size: None, host: String::new(), pos, error: None });
                        }
                    }
                    PopupEvent::Failed { key, reason } => {
                        self.dismissed.remove(&key);
                        if let Some(x) = self.popups.iter_mut().find(|x| x.key == key) {
                            x.error = Some(reason.clone());
                            self.handoff_fails.insert(0, Handoff { at: now_secs(), name: x.name.clone(), reason });
                            self.handoff_fails.truncate(50);
                        }
                    }
                    PopupEvent::Dropped { key } => {
                        self.dismissed.remove(&key);
                        self.popups.retain(|x| x.key != key);
                    }
                }
            }
        }
        let list = self.popups.clone();
        for pop in list {
            let view = match pop.task {
                Some(id) => match self.views.iter().find(|v| v.id == id).cloned() {
                    Some(v) => Some(v),
                    None => {
                        self.popups.retain(|x| x.key != pop.key);
                        continue;
                    }
                },
                None => None,
            };
            let title = view.as_ref().map(|v| v.filename.clone()).unwrap_or_else(|| pop.name.clone());
            let builder = egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([440.0, 196.0])
                .with_position(pop.pos)
                .with_decorations(false)
                .with_resizable(false)
                .with_always_on_top()
                .with_icon(self.icon.clone());
            let mut close = false;
            ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("popup", pop.key)), builder, |ui, _| {
                close = match &view {
                    Some(v) => self.popup_ui(ui, v, pop.key, p),
                    None => self.pending_ui(ui, &pop, p),
                };
            });
            if close {
                if view.is_none() && pop.error.is_none() {
                    self.dismissed.insert(pop.key);
                }
                self.popups.retain(|x| x.key != pop.key);
            }
        }
    }

    /// Popup chrome shared by the connecting and task popups. Returns (body rect, close clicked).
    fn popup_frame(&mut self, ui: &mut Ui, drag_id: Id, p: &Pal) -> (Rect, bool) {
        let ctx = ui.ctx().clone();
        let tex = self.icon_tex(&ctx);
        let full = ui.max_rect();
        let pt = ui.painter().clone();
        let strip = Rect::from_min_size(full.min, vec2(full.width(), 32.0));
        let drag = ui.interact(strip, drag_id, Sense::drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let ir = Rect::from_center_size(pos2(strip.left() + 18.0, strip.center().y), vec2(14.0, 14.0));
        egui::Image::new(egui::load::SizedTexture::new(tex.id(), ir.size())).paint_at(ui, ir);
        pt.text(pos2(strip.left() + 34.0, strip.center().y), Align2::LEFT_CENTER, "GaGaDown", font(12.0), p.weak);
        let close = chrome_btn(ui, Rect::from_min_size(pos2(strip.right() - 46.0, strip.top()), vec2(46.0, 32.0)), Glyph::Close, p);
        pt.rect_stroke(full, 0.0, Stroke::new(1.0, p.line), StrokeKind::Inside);
        (Rect::from_min_max(pos2(full.left() + 20.0, strip.bottom() + 14.0), pos2(full.right() - 20.0, full.bottom() - 16.0)), close)
    }

    fn pending_ui(&mut self, ui: &mut Ui, pop: &Pop, p: &Pal) -> bool {
        let ctx = ui.ctx().clone();
        let mut close = ctx.input(|i| i.viewport().close_requested());
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.editor)).show(ui, |ui| {
            let (b, x) = self.popup_frame(ui, Id::new(("popup_drag", pop.key)), p);
            close |= x;
            let pt = ui.painter().clone();
            self.paint_file(&ctx, &pt, pos2(b.left() + 18.0, b.top() + 18.0), 36.0, &pop.name, None, p, 1.0);
            text_line(&pt, pos2(b.left() + 48.0, b.top() + 9.0), Align2::LEFT_CENTER, &pop.name, bold(14.0), p.text, b.width() - 48.0);
            let size = pop.size.map(bytes).unwrap_or_else(|| "大小未知".into());
            text_line(&pt, pos2(b.left() + 48.0, b.top() + 28.0), Align2::LEFT_CENTER, format!("{size}   {}", pop.host), font(12.0), p.weak, b.width() - 48.0);
            let by = b.top() + 56.0;
            let bar = Rect::from_min_size(pos2(b.left(), by), vec2(b.width(), 4.0));
            pt.rect_filled(bar, CornerRadius::same(2), p.line);
            let label = if let Some(err) = &pop.error {
                let t = since_on(&ctx, Id::new(("pending_err", pop.key)), true).unwrap_or(1.0);
                let k = ease_out_cubic((t / 0.5).min(1.0));
                pt.rect_filled(bar, CornerRadius::same(2), blend(p.red, p.accent, k));
                text_line(&pt, pos2(b.left(), by + 22.0), Align2::LEFT_CENTER, format!("无法接管：{err}"), font(12.0), p.red, b.width());
                text_line(&pt, pos2(b.left(), by + 40.0), Align2::LEFT_CENTER, "已交给浏览器继续下载", font(12.0), p.weak, b.width() - 90.0);
                "关闭"
            } else {
                let now = ctx.input(|i| i.time);
                ctx.data_mut(|d| d.insert_temp(Id::new(("pend", pop.key)), now));
                indeterminate(&pt, bar, ctx.input(|i| i.time), p.accent);
                text_line(&pt, pos2(b.left(), by + 22.0), Align2::LEFT_CENTER, "正在连接…", font(12.0), p.weak, b.width());
                "取消"
            };
            let r = Rect::from_min_size(pos2(b.right() - 76.0, b.bottom() - 28.0), vec2(76.0, 28.0));
            if btn_at(ui, r, Id::new(("pending_btn", pop.key)), label, false, p) {
                close = true;
            }
        });
        ctx.request_repaint_after(Duration::from_millis(33));
        close
    }

    fn show_in_main(&mut self, ctx: &egui::Context, id: Uuid) {
        self.selected = Some(id);
        self.hidden.store(false, Ordering::SeqCst);
        for c in [ViewportCommand::Visible(true), ViewportCommand::Minimized(false), ViewportCommand::Focus] {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, c);
        }
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }

    fn popup_ui(&mut self, ui: &mut Ui, v: &TaskView, pop_key: Uuid, p: &Pal) -> bool {
        let ctx = ui.ctx().clone();
        let mut close = ctx.input(|i| i.viewport().close_requested());
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.editor)).show(ui, |ui| {
            let (b, x) = self.popup_frame(ui, Id::new(("popup_drag", v.id)), p);
            close |= x;
            let pt = ui.painter().clone();
            let key = Id::new(("pop", v.id));
            let ok = v.status == Status::Completed && !v.file_missing;
            let since = since_on(&ctx, key.with("done"), ok);
            let c = pos2(b.left() + 18.0, b.top() + 18.0);
            if let Some(t) = since {
                if t < 1.5 {
                    ctx.request_repaint();
                }
            }
            self.paint_file(&ctx, &pt, c, 36.0, &v.filename, done_path(v), p, 1.0);
            let badge = match since {
                Some(t) => ease_out_back((t - 0.3) / 0.45),
                None if ok => 1.0,
                None => 0.0,
            };
            if badge > 0.01 {
                let bc = c + vec2(13.0, 13.0);
                pt.circle_filled(bc, 10.0 * badge, p.editor);
                pt.circle_filled(bc, 8.0 * badge, p.green);
                let m = |x: f32, y: f32| bc + vec2(x, y) * badge;
                pt.add(Shape::line(vec![m(-3.4, 0.2), m(-1.1, 2.5), m(3.5, -2.3)], Stroke::new(1.6 * badge.min(1.0), Color32::WHITE)));
            }
            text_line(&pt, pos2(b.left() + 48.0, b.top() + 9.0), Align2::LEFT_CENTER, &v.filename, bold(14.0), p.text, b.width() - 48.0);
            let size = v.size.map(bytes).unwrap_or_else(|| "大小未知".into());
            text_line(&pt, pos2(b.left() + 48.0, b.top() + 28.0), Align2::LEFT_CENTER, format!("{size}   {}", v.host), font(12.0), p.weak, b.width() - 48.0);

            let by = b.top() + 56.0;
            let bar = Rect::from_min_size(pos2(b.left(), by), vec2(b.width(), 4.0));
            let f = seg_bar(&ctx, &pt, bar, v, p, key);
            // Coming from "正在连接…": fade the connecting sweep out while the real fill eases in.
            let now = ctx.input(|i| i.time);
            if ctx.data(|d| d.get_temp::<f64>(Id::new(("pend", pop_key)))).is_some() {
                let t = (now - ctx.data_mut(|d| *d.get_temp_mut_or(key.with("shown"), now))) as f32;
                if t < 0.45 {
                    indeterminate(&pt, bar, now, p.accent.gamma_multiply(1.0 - ease_out_cubic(t / 0.45)));
                    ctx.request_repaint();
                }
            }
            if ok {
                let a = since.map_or(1.0, |t| ease_out_cubic((t - 0.35) / 0.4));
                let y = by + 22.0 + (1.0 - a) * 6.0;
                let r1 = text_line(&pt, pos2(b.left(), y), Align2::LEFT_CENTER, "下载完成", font(12.5), p.green.gamma_multiply(a), 120.0);
                let mut extra = Vec::new();
                if let Some(fin) = v.finished_at {
                    extra.push(format!("用时 {}", dur(fin.saturating_sub(v.created_at).max(1))));
                }
                if v.avg_speed > 0.0 {
                    extra.push(format!("平均 {}", speed(v.avg_speed)));
                }
                text_line(&pt, pos2(r1.right() + 12.0, y), Align2::LEFT_CENTER, extra.join("   "), font(12.0), p.weak.gamma_multiply(a), b.right() - r1.right() - 12.0);
            } else {
                let sp = speed(spring(&ctx, key.with("spd"), v.speed as f32, 5.0, 0.0) as f64);
                let (line, lc) = match v.status {
                    Status::Running => match v.eta_secs {
                        Some(e) => (format!("{sp}   剩余 {}   {} 连接", dur(e), v.connections), p.weak),
                        None => (format!("{sp}   {} 连接", v.connections), p.weak),
                    },
                    _ => meta_line(v, p),
                };
                text_line(&pt, pos2(b.left(), by + 22.0), Align2::LEFT_CENTER, line, font(12.0), lc, b.width() - 60.0);
                pt.text(pos2(b.right(), by + 22.0), Align2::RIGHT_CENTER, format!("{:.0}%", f * 100.0), font(12.0), p.weak);
            }

            let buttons: Vec<(&str, bool, Option<Act>)> = match v.status {
                Status::Completed if !v.file_missing => vec![("打开所在文件夹", false, Some(Act::Reveal)), ("打开", true, Some(Act::Open))],
                Status::Running | Status::Queued => vec![("取消", false, None), ("暂停", true, Some(Act::Pause))],
                Status::Paused => vec![("取消", false, None), ("继续", true, Some(Act::Resume))],
                _ => vec![("取消", false, None), ("重试", true, Some(Act::Resume))],
            };
            let show = "在主界面中显示";
            let sw = pt.layout_no_wrap(show.to_string(), font(12.5), p.text).size().x + 28.0;
            let sr = Rect::from_min_size(pos2(b.left(), b.bottom() - 28.0), vec2(sw, 28.0));
            if btn_at(ui, sr, Id::new(("popup_show", v.id)), show, false, p) {
                self.show_in_main(&ctx, v.id);
                close = true;
            }
            let mut x = b.right();
            for (i, (label, primary, a)) in buttons.into_iter().enumerate().rev() {
                let w = pt.layout_no_wrap(label.to_string(), font(12.5), p.text).size().x + 28.0;
                let r = Rect::from_min_size(pos2(x - w.max(76.0), b.bottom() - 28.0), vec2(w.max(76.0), 28.0));
                if btn_at(ui, r, Id::new(("popup_btn", v.id, i)), label, primary, p) {
                    match a {
                        Some(a) => {
                            self.act(&ctx, v, a);
                            if matches!(a, Act::Open | Act::Reveal) {
                                close = true;
                            }
                        }
                        None => {
                            let e = self.engine.clone();
                            let id = v.id;
                            self.rt.spawn(async move { e.remove(id, RemoveMode::DeleteFiles).await });
                            close = true;
                        }
                    }
                }
                x = r.left() - 8.0;
            }
        });
        close
    }
}

fn resize_edges(ctx: &egui::Context, r: Rect) {
    use egui::{CursorIcon, ResizeDirection as D};
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let m = 5.0;
    let (l, rt, t, b) = (pos.x < r.left() + m, pos.x > r.right() - m, pos.y < r.top() + m, pos.y > r.bottom() - m);
    let dir = match (l, rt, t, b) {
        (true, _, true, _) => D::NorthWest,
        (_, true, true, _) => D::NorthEast,
        (true, _, _, true) => D::SouthWest,
        (_, true, _, true) => D::SouthEast,
        (true, ..) => D::West,
        (_, true, ..) => D::East,
        (_, _, true, _) => D::North,
        (_, _, _, true) => D::South,
        _ => return,
    };
    ctx.set_cursor_icon(match dir {
        D::NorthWest | D::SouthEast => CursorIcon::ResizeNwSe,
        D::NorthEast | D::SouthWest => CursorIcon::ResizeNeSw,
        D::West | D::East => CursorIcon::ResizeHorizontal,
        _ => CursorIcon::ResizeVertical,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.decorated {
            self.decorated = true;
            win::decorate(frame);
        }
        if self.accent_checked.elapsed() > Duration::from_secs(5) {
            self.accent_checked = Instant::now();
            let a = win::accent().unwrap_or(DEFAULT_ACCENT);
            if a != self.accent {
                self.accent = a;
                apply_style(&ctx, a);
            }
        }
        if self.tray.is_some() && !self.quit.load(Ordering::SeqCst) && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.hidden.store(true, Ordering::SeqCst);
        }
        self.refresh();
        #[cfg(windows)]
        win::DARK_TEXT.store(ui.visuals().dark_mode, std::sync::atomic::Ordering::Relaxed);
        let p = Pal::new(ui.visuals().dark_mode, self.accent);
        let full = ui.max_rect();

        if !ctx.egui_wants_keyboard_input() {
            let pasted = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Paste(s) = e { Some(s.clone()) } else { None }));
            if let Some(s) = pasted.filter(|s| s.contains("://")) {
                self.input = s;
                self.add_input();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Delete)) && self.page == Page::Downloads && self.removing.is_none() {
                self.removing = self.selected;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.selected = None;
            }
        }

        egui::Panel::top("title").exact_size(36.0).frame(egui::Frame::new().fill(p.chrome)).show(ui, |ui| self.title_bar(ui, &p));
        egui::Panel::bottom("status").exact_size(22.0).frame(egui::Frame::new().fill(p.chrome)).show(ui, |ui| self.status_bar(ui, &p));
        egui::Panel::left("activity").exact_size(48.0).resizable(false).frame(egui::Frame::new().fill(p.editor)).show(ui, |ui| self.activity_bar(ui, &p));
        if self.page == Page::Downloads {
            if let Some(v) = self.selected.and_then(|id| self.views.iter().find(|v| v.id == id)).cloned() {
                egui::Panel::right("detail")
                    .default_size(320.0)
                    .size_range(260.0..=560.0)
                    .resizable(true)
                    .frame(egui::Frame::new().fill(p.editor))
                    .show(ui, |ui| self.detail(ui, &v, &p));
            }
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.editor)).show(ui, |ui| match self.page {
            Page::Downloads => self.downloads(ui, &p),
            Page::Trash => self.trash(ui, &p),
            Page::Cache => self.cache_page(ui, &p),
            Page::Extension => self.extension_page(ui, &p),
            Page::Settings => self.settings_page(ui, &p),
        });
        self.remove_modal(&ctx, &p);
        self.error_modal(&ctx, &p);
        self.show_toasts(&ctx, &p);
        self.popups(&ctx, &p);
        resize_edges(&ctx, full);
        let busy = self.views.iter().any(|v| matches!(v.status, Status::Running | Status::Queued)) || self.engine.is_detecting();
        ctx.request_repaint_after(Duration::from_millis(if busy { 250 } else { 1000 }));
    }

    fn on_exit(&mut self) {
        let e = self.engine.clone();
        self.rt.block_on(async move {
            let _ = tokio::time::timeout(Duration::from_secs(3), e.shutdown()).await;
        });
    }
}

/// 32x32 box-filtered copy of the 256px app icon for the tray.
fn tray_rgba() -> Vec<u8> {
    let mut out = vec![0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let mut acc = [0u32; 4];
            for dy in 0..8 {
                for dx in 0..8 {
                    let i = (((y * 8 + dy) * 256) + x * 8 + dx) * 4;
                    let a = ICON[i + 3] as u32;
                    for c in 0..3 {
                        acc[c] += ICON[i + c] as u32 * a;
                    }
                    acc[3] += a;
                }
            }
            let o = (y * 32 + x) * 4;
            if acc[3] > 0 {
                for c in 0..3 {
                    out[o + c] = (acc[c] / acc[3]) as u8;
                }
            }
            out[o + 3] = (acc[3] / 64) as u8;
        }
    }
    out
}

fn main() -> eframe::Result {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    // UI-thread calls into the engine may spawn tokio tasks.
    let rt_handle = rt.handle().clone();
    let _rt_guard = rt_handle.enter();
    let engine = {
        let _g = rt.enter();
        Engine::new(None)
    };
    let engine = match engine {
        Ok(e) => e,
        Err(e) => {
            let _ = rfd::MessageDialog::new().set_title("GaGaDown").set_description(format!("{e}")).show();
            std::process::exit(1);
        }
    };
    install_diagnostics(engine.data_dir().to_path_buf());
    let st = engine.settings();
    win::set_autostart(st.launch_at_login);
    let silent = st.launch_at_login && st.start_minimized && std::env::args().any(|a| a == "--autostart");
    eframe::START_HIDDEN.store(silent, Ordering::Relaxed);
    let api_error = Arc::new(Mutex::new(None));
    {
        let e = engine.clone();
        let port = e.settings().api_port;
        let ae = api_error.clone();
        rt.spawn(async move {
            if let Err(x) = gagadown_core::api::serve(e, port).await {
                *ae.lock() = Some(format!("浏览器插件端口 {port} 不可用：{x}"));
            }
        });
    }
    let icon = Arc::new(egui::IconData { rgba: ICON.to_vec(), width: 256, height: 256 });
    let accent = win::accent().unwrap_or(DEFAULT_ACCENT);
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("GaGaDown")
            .with_app_id("gagadown")
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([760.0, 480.0])
            .with_decorations(false)
            .with_icon(icon.clone()),
        centered: true,
        ..Default::default()
    };
    #[cfg(windows)]
    eframe::epaint::text::set_glyph_rasterizer(win::gdi_glyph);
    eframe::run_native(
        "GaGaDown",
        opts,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            apply_style(&cc.egui_ctx, accent);
            let mut app = App::new(rt, engine.clone(), api_error, icon, accent);
            app.tray = win::tray(cc.egui_ctx.clone(), tray_rgba(), app.quit.clone(), app.hidden.clone());
            app.hidden.store(silent, Ordering::SeqCst);
            let _ = unpack_extension(engine.data_dir());
            // Keep the UI pass running while minimized or in the tray so hand-off popups still open.
            eframe::KEEP_ROOT_UI.store(true, Ordering::Relaxed);
            let ctx = cc.egui_ctx.clone();
            engine.set_waker(move || ctx.request_repaint());
            Ok(Box::new(app))
        }),
    )
}
