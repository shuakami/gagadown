//! One task's download run: route resolution, preallocation, worker pool with
//! work-stealing segments, hill-climbing concurrency control, straggler eviction,
//! mid-flight route failover, finalisation.
use crate::engine::{Inner, TaskEntry};
use crate::error::{DlError, DlResult, ErrorKind};
use crate::probe::{parse_content_range, retry_after, sanitize_filename};
use crate::route::Route;
use crate::segments::Table;
use crate::task::{Phase, StopReason};
use futures_util::StreamExt;
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

pub const PART_EXT: &str = "fdpart";
const FLUSH_AT: usize = 1024 * 1024;
const CHUNK_TIMEOUT: Duration = Duration::from_secs(15);
const STALL_TIMEOUT: Duration = Duration::from_secs(20);

/// Live state of a running task, shared by workers, controller and UI snapshots.
pub struct Shared {
    pub table: Mutex<Option<Table>>,
    pub target: AtomicUsize,
    pub active: AtomicUsize,
    pub cancel: CancellationToken,
    pub stop: AtomicU8,
    pub phase: AtomicU8,
    pub fatal: Mutex<Option<DlError>>,
    pub route: RwLock<Option<Arc<Route>>>,
    pub route_errors: AtomicU32,
    pub throttles: AtomicU32,
    pub received: AtomicU64,
    pub speed: AtomicU64,
    pub history: Mutex<VecDeque<f32>>,
    pub single: AtomicBool,
    /// Bytes written in single-stream mode (no table).
    pub stream_done: AtomicU64,
    pub retries: AtomicU32,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            table: Mutex::new(None),
            target: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            cancel: CancellationToken::new(),
            stop: AtomicU8::new(StopReason::None as u8),
            phase: AtomicU8::new(Phase::Connecting as u8),
            fatal: Mutex::new(None),
            route: RwLock::new(None),
            route_errors: AtomicU32::new(0),
            throttles: AtomicU32::new(0),
            received: AtomicU64::new(0),
            speed: AtomicU64::new(0),
            history: Mutex::new(VecDeque::with_capacity(120)),
            single: AtomicBool::new(false),
            stream_done: AtomicU64::new(0),
            retries: AtomicU32::new(0),
        }
    }

    pub fn set_phase(&self, p: Phase) {
        self.phase.store(p as u8, Ordering::Relaxed);
    }

    pub fn speed(&self) -> f64 {
        f64::from_bits(self.speed.load(Ordering::Relaxed))
    }

    fn fail(&self, e: DlError) {
        let mut f = self.fatal.lock();
        if f.is_none() {
            *f = Some(e);
        }
        self.cancel.cancel();
    }
}

pub struct Outcome {
    pub final_path: PathBuf,
    pub size: u64,
}

fn write_all_at(f: &File, mut buf: &[u8], mut off: u64) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        while !buf.is_empty() {
            let n = f.write_at(buf, off)?;
            if n == 0 {
                return Err(std::io::ErrorKind::WriteZero.into());
            }
            buf = &buf[n..];
            off += n as u64;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            let n = f.seek_write(buf, off)?;
            if n == 0 {
                return Err(std::io::ErrorKind::WriteZero.into());
            }
            buf = &buf[n..];
            off += n as u64;
        }
    }
    Ok(())
}

async fn write_at(f: &Arc<File>, buf: Vec<u8>, off: u64) -> DlResult<Vec<u8>> {
    let f = f.clone();
    tokio::task::spawn_blocking(move || write_all_at(&f, &buf, off).map(|_| buf))
        .await
        .map_err(|e| DlError::new(ErrorKind::Io, e.to_string()))?
        .map_err(|e| DlError::from_io(&e))
}

static CACHE_DIR: parking_lot::RwLock<Option<PathBuf>> = parking_lot::RwLock::new(None);

pub fn set_cache_dir(d: Option<PathBuf>) {
    *CACHE_DIR.write() = d;
}

pub fn cache_dir() -> Option<PathBuf> {
    CACHE_DIR.read().clone()
}

pub fn part_path(final_path: &Path) -> PathBuf {
    if let Some(d) = cache_dir() {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        final_path.hash(&mut h);
        let name = final_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        return d.join(format!("{name}-{:016x}.{PART_EXT}", h.finish()));
    }
    let mut s = final_path.as_os_str().to_owned();
    s.push(".");
    s.push(PART_EXT);
    PathBuf::from(s)
}

/// `name.ext` -> `name (1).ext` until nothing on disk or in `taken` collides.
pub fn unique_path(dir: &Path, name: &str, taken: &[PathBuf]) -> PathBuf {
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string());
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut i = 0;
    loop {
        let cand = if i == 0 { dir.join(name) } else { dir.join(format!("{stem} ({i}){ext}")) };
        if !cand.exists() && !part_path(&cand).exists() && !taken.contains(&cand) {
            return cand;
        }
        i += 1;
    }
}

fn backoff(attempt: u32, hint: Option<Duration>) -> Duration {
    if let Some(h) = hint {
        return h.max(Duration::from_millis(200));
    }
    let base = 400u64 * (1u64 << attempt.min(6));
    Duration::from_millis(fastrand::u64(base / 2..=base).min(30_000))
}

struct Ctx {
    inner: Arc<Inner>,
    entry: Arc<TaskEntry>,
    sh: Arc<Shared>,
    file: Arc<File>,
    urls: Vec<String>,
    spec: crate::request::RequestSpec,
    ua: String,
    validator: Option<String>,
    min_split: u64,
    retries: u32,
    /// Largest byte range one request may ask for; the rest of the segment is re-requested.
    max_req: Option<u64>,
}

impl Ctx {
    fn headers(&self) -> reqwest::header::HeaderMap {
        self.spec.header_map(&self.ua)
    }

    fn log(&self, s: impl Into<String>) {
        self.entry.log(s);
    }

    fn eff_min_split(&self) -> u64 {
        // Endgame: as the tail shrinks, allow finer splits so no connection idles.
        let left = self.sh.table.lock().as_ref().map(|t| t.left_total()).unwrap_or(0);
        let target = self.sh.target.load(Ordering::Relaxed).max(1) as u64;
        (left / (target * 4)).clamp(64 * 1024, self.min_split)
    }
}

pub async fn run(inner: Arc<Inner>, entry: Arc<TaskEntry>, sh: Arc<Shared>) -> DlResult<Outcome> {
    let mut force_single = false;
    loop {
        match run_once(&inner, &entry, &sh, force_single).await {
            Err(e) if e.kind == ErrorKind::RangeUnsupported && !force_single && !sh.cancel.is_cancelled() => {
                entry.log("服务器分段异常，切换单线程");
                force_single = true;
                *sh.fatal.lock() = None;
                entry.with(|r| {
                    r.ranges = false;
                    r.remaining = None;
                });
            }
            Err(e) if e.kind == ErrorKind::ResourceChanged && !sh.cancel.is_cancelled() && sh.retries.fetch_add(1, Ordering::Relaxed) < 2 => {
                entry.log("远端文件已变化，重新下载");
                *sh.fatal.lock() = None;
                entry.with(|r| {
                    r.remaining = None;
                    r.etag = None;
                    r.last_modified = None;
                });
            }
            other => return other,
        }
    }
}

async fn run_once(inner: &Arc<Inner>, entry: &Arc<TaskEntry>, sh: &Arc<Shared>, force_single: bool) -> DlResult<Outcome> {
    sh.set_phase(Phase::Connecting);
    let settings = inner.settings.read().clone();
    let rec = entry.snapshot_record();
    let spec = rec.spec.clone();
    let ua = settings.user_agent.clone();

    let (route, info) = tokio::select! {
        r = inner.routes.resolve(&spec, &spec.url, &ua, None) => r?,
        _ = sh.cancel.cancelled() => return Err(DlError::cancelled()),
    };
    *sh.route.write() = Some(route.clone());
    entry.log(format!("线路 {}，响应 {} ms", route.name, info.latency_ms));
    if rec.size.is_none() && is_web_page(&info) {
        return Err(DlError { status: Some(info.status), ..DlError::new(ErrorKind::NotDownload, "服务器返回的是网页，不是文件") });
    }

    let ranges = info.ranges && !force_single && info.size.unwrap_or(0) > 0;
    let changed = rec.size.is_some()
        && (rec.size != info.size
            || (rec.etag.is_some() && info.etag.is_some() && rec.etag != info.etag)
            || (rec.etag.is_none() && rec.last_modified.is_some() && info.last_modified.is_some() && rec.last_modified != info.last_modified));

    // Decide paths.
    let taken = inner.taken_paths(rec.id);
    let (final_path, part) = {
        let mut r = entry.rec.lock();
        if r.filename.is_empty() {
            r.filename = info.filename.clone().unwrap_or_else(|| "download".into());
        }
        r.filename = sanitize_filename(&r.filename);
        let path = match &r.path {
            Some(p) => p.clone(),
            None => {
                let p = unique_path(&r.dir, &r.filename, &taken);
                r.filename = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                r.path = Some(p.clone());
                p
            }
        };
        let part = part_path(&path);
        if let Some(d) = part.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if changed || !ranges || !part.exists() {
            if changed {
                tracing::info!("resource changed, restarting");
            }
            r.remaining = None;
        }
        r.size = info.size;
        r.ranges = ranges;
        r.etag = info.etag.clone();
        r.last_modified = info.last_modified.clone();
        r.content_type = info.content_type.clone();
        r.final_url = Some(info.final_url.clone());
        r.route = Some(route.name.clone());
        (path, part)
    };
    if changed {
        entry.log("远端文件已变化，从头下载");
    }
    if let Some(dir) = final_path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let remaining = entry.rec.lock().remaining.clone();
    let need = match (&remaining, info.size) {
        (Some(rem), _) => rem.iter().map(|(a, b)| b - a).sum(),
        (None, Some(s)) => s,
        _ => 0,
    };
    if let Some(dir) = final_path.parent() {
        if let Ok(avail) = fs4::available_space(dir) {
            if avail < need + 16 * 1024 * 1024 {
                return Err(DlError::new(ErrorKind::DiskFull, format!("需要 {} MB，可用 {} MB", need >> 20, avail >> 20)));
            }
        }
    }

    let fresh = remaining.is_none();
    // Large preallocations can block for minutes. Keep them off Tokio's worker
    // threads, and await their completion even after cancellation so remove()
    // cannot unlink a file that is still being allocated.
    let part_for_open = part.clone();
    let allocation_size = info.size;
    let file = tokio::task::spawn_blocking(move || -> std::io::Result<File> {
        let file = std::fs::OpenOptions::new().create(true).read(true).write(true).truncate(fresh).open(&part_for_open)?;
        if let Some(size) = allocation_size {
            if file.metadata()?.len() != size && ranges {
                file.set_len(size)?;
            }
            if fresh && ranges && size > 0 {
                fs4::FileExt::allocate(&file, size)?;
            }
        }
        Ok(file)
    })
    .await
    .map_err(|e| DlError::new(ErrorKind::Io, e.to_string()))??;
    if sh.cancel.is_cancelled() {
        return Err(DlError::new(ErrorKind::Cancelled, "下载已停止"));
    }
    let file = Arc::new(file);
    let validator = info.etag.clone().or(info.last_modified.clone());

    let ctx = Arc::new(Ctx {
        inner: inner.clone(),
        entry: entry.clone(),
        sh: sh.clone(),
        file: file.clone(),
        urls: {
            let mut u = vec![info.final_url.clone()];
            for m in spec.mirrors.iter() {
                if !u.contains(m) {
                    u.push(m.clone());
                }
            }
            u
        },
        spec,
        ua,
        validator,
        min_split: settings.min_split_size,
        retries: settings.segment_retries,
        // Baidu PCS answers 403 (31326) to ranges above ~8 MB for non-member clients.
        max_req: [info.final_url.as_str(), rec.spec.url.as_str()]
            .iter()
            .any(|u| baidu_pcs_host(u))
            .then_some(4 << 20),
    });

    let size = if ranges {
        let size = info.size.unwrap_or(0);
        let host = rec.spec.host();
        let learned = inner.routes.hosts.lock().get(&host).cloned().unwrap_or_default();
        let mut cap = settings.max_connections_per_task;
        if let Some(c) = learned.ceiling {
            cap = cap.min(c.max(1));
        }
        // Baidu PCS blacklists accounts that open many parallel ranges on one file.
        if host.ends_with("baidupcs.com") || host.ends_with("pcs.baidu.com") {
            cap = cap.min(8);
        }
        // Open as many connections as the file can use right away; slow start takes it from there.
        let by_size = ((size / (512 * 1024)) as usize).max(1);
        let fresh_start = by_size.min(settings.initial_connections.max(32));
        let start = if learned.best_conns > 0 { learned.best_conns.max(fresh_start.min(settings.initial_connections)) } else { fresh_start }.min(cap);
        sh.target.store(start.max(1), Ordering::Relaxed);
        sh.single.store(false, Ordering::Relaxed);
        {
            let t = match &remaining {
                Some(rem) => Table::from_remaining(size, rem),
                None => Table::fresh(size, start, settings.min_split_size),
            };
            *sh.table.lock() = Some(t);
        }
        sh.set_phase(Phase::Downloading);
        segmented(ctx.clone(), cap, host).await?;
        size
    } else {
        sh.single.store(true, Ordering::Relaxed);
        sh.target.store(1, Ordering::Relaxed);
        sh.set_phase(Phase::Downloading);
        if info.size.is_some() {
            *sh.table.lock() = Some(Table::fresh(info.size.unwrap(), 1, u64::MAX / 4));
        }
        single_stream(ctx.clone(), info.size).await?
    };

    sh.set_phase(Phase::Finalizing);
    if settings.sync_on_complete {
        let f = file.clone();
        let _ = tokio::task::spawn_blocking(move || f.sync_data()).await;
    }
    drop(ctx);
    drop(file);
    let mut final_path = final_path;
    if final_path.exists() {
        let dir = final_path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let name = final_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let taken = inner.taken_paths(rec.id);
        final_path = unique_path(&dir, &name, &taken);
        entry.with(|r| {
            r.filename = final_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            r.path = Some(final_path.clone());
        });
    }
    if std::fs::rename(&part, &final_path).is_err() {
        std::fs::copy(&part, &final_path)?;
        let _ = std::fs::remove_file(&part);
    }

    if let Some(expect) = rec.sha256.clone().filter(|s| !s.is_empty()) {
        sh.set_phase(Phase::Verifying);
        let p = final_path.clone();
        let got = tokio::task::spawn_blocking(move || sha256_file(&p)).await.map_err(|e| DlError::new(ErrorKind::Io, e.to_string()))??;
        if !got.eq_ignore_ascii_case(expect.trim()) {
            entry.log(format!("SHA-256 不一致: {got}"));
            let _ = std::fs::remove_file(&final_path);
            return Err(DlError::new(ErrorKind::ResourceChanged, "校验失败"));
        }
        entry.log("SHA-256 校验通过");
    }
    Ok(Outcome { final_path, size })
}

pub fn sha256_file(p: &Path) -> DlResult<String> {
    use sha2::Digest;
    use std::io::Read;
    let mut f = File::open(p)?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

struct Controller {
    cap: usize,
    slow_start: bool,
    last: f64,
    best: f64,
    best_target: usize,
    probing: Option<usize>,
    windows: u32,
    throttles_seen: u32,
}

impl Controller {
    /// Throughput hill-climbing: double while it pays (> +8%), then probe +step every few
    /// windows and keep it only on a real gain. Throttling signals cut by 40% (AIMD).
    fn step(&mut self, sh: &Shared, speed: f64, throttles: u32) -> usize {
        let cur = sh.target.load(Ordering::Relaxed);
        self.windows += 1;
        if throttles > self.throttles_seen {
            self.throttles_seen = throttles;
            let n = ((cur as f64) * 0.6).floor().max(1.0) as usize;
            self.cap = self.cap.min(cur.max(2));
            self.slow_start = false;
            self.probing = None;
            self.best_target = n;
            return n;
        }
        if speed > self.best {
            self.best = speed;
            self.best_target = cur;
        }
        if self.slow_start {
            if self.last == 0.0 || speed > self.last * 1.08 {
                self.last = speed;
                return (cur * 2).min(self.cap);
            }
            self.slow_start = false;
            self.last = speed;
            return self.best_target.max(1);
        }
        if let Some(prev) = self.probing.take() {
            if speed > self.last * 1.04 {
                self.last = speed;
                return cur;
            }
            return prev;
        }
        self.last = self.last * 0.7 + speed * 0.3;
        if self.windows % 3 == 0 && cur < self.cap {
            self.probing = Some(cur);
            return (cur + (cur / 8).max(2)).min(self.cap);
        }
        cur
    }
}

async fn segmented(ctx: Arc<Ctx>, cap: usize, host: String) -> DlResult<()> {
    let sh = ctx.sh.clone();
    let mut workers = tokio::task::JoinSet::new();
    let mut next_wid: u32 = 0;
    let mut ctl = Controller { cap, slow_start: true, last: 0.0, best: 0.0, best_target: sh.target.load(Ordering::Relaxed), probing: None, windows: 0, throttles_seen: 0 };
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    let mut last_sample = Instant::now();
    let mut last_received = sh.received.load(Ordering::Relaxed);
    let mut window_bytes = 0u64;
    let mut window_start = Instant::now();
    let mut stalled_since: Option<Instant> = None;
    let mut ewma = 0.0f64;

    loop {
        // Spawn up to target.
        let min = ctx.eff_min_split();
        loop {
            let active = sh.active.load(Ordering::Relaxed);
            if sh.cancel.is_cancelled() || active >= sh.target.load(Ordering::Relaxed) {
                break;
            }
            let has_work = sh.table.lock().as_ref().map(|t| t.has_work(min)).unwrap_or(false);
            if !has_work {
                break;
            }
            let Ok(permit) = ctx.inner.conn_budget.clone().try_acquire_owned() else { break };
            sh.active.fetch_add(1, Ordering::Relaxed);
            let c = ctx.clone();
            let wid = next_wid;
            next_wid += 1;
            workers.spawn(async move {
                if !worker(c.clone(), wid).await {
                    c.sh.active.fetch_sub(1, Ordering::Relaxed);
                }
                drop(permit);
            });
        }

        tokio::select! {
            _ = tick.tick() => {}
            Some(_) = workers.join_next(), if !workers.is_empty() => {}
        }

        let done = sh.table.lock().as_ref().map(|t| t.complete()).unwrap_or(true);
        if done && workers.is_empty() {
            break;
        }
        if sh.cancel.is_cancelled() {
            while workers.join_next().await.is_some() {}
            break;
        }
        if workers.is_empty() && !done {
            let has_work = sh.table.lock().as_ref().map(|t| t.has_work(64 * 1024)).unwrap_or(false);
            if !has_work {
                break;
            }
        }

        // 1 s sampling: speed, history, segment speeds, straggler eviction.
        if last_sample.elapsed() >= Duration::from_secs(1) {
            let dt = last_sample.elapsed().as_secs_f64();
            last_sample = Instant::now();
            let rx = sh.received.load(Ordering::Relaxed);
            let inst = (rx - last_received) as f64 / dt;
            last_received = rx;
            window_bytes += (inst * dt) as u64;
            ewma = if ewma == 0.0 { inst } else { ewma * 0.6 + inst * 0.4 };
            sh.speed.store(ewma.to_bits(), Ordering::Relaxed);
            {
                let mut h = sh.history.lock();
                if h.len() >= 120 {
                    h.pop_front();
                }
                h.push_back(inst as f32);
            }
            evict_stragglers(&ctx, dt);

            if inst < 1.0 {
                let since = *stalled_since.get_or_insert_with(Instant::now);
                if since.elapsed() > Duration::from_secs(8) && sh.route_errors.load(Ordering::Relaxed) >= 3 {
                    stalled_since = None;
                    failover(&ctx).await;
                }
            } else {
                stalled_since = None;
                sh.route_errors.store(0, Ordering::Relaxed);
            }
        }

        // 2 s control window.
        let window = if ctl.slow_start { Duration::from_millis(500) } else { Duration::from_secs(2) };
        if window_start.elapsed() >= window {
            let speed = window_bytes as f64 / window_start.elapsed().as_secs_f64();
            window_bytes = 0;
            window_start = Instant::now();
            let saturated = sh.active.load(Ordering::Relaxed) >= sh.target.load(Ordering::Relaxed);
            let throttles = sh.throttles.load(Ordering::Relaxed);
            if saturated || throttles > ctl.throttles_seen {
                let n = ctl.step(&sh, speed, throttles);
                sh.target.store(n.max(1), Ordering::Relaxed);
            }
        }
    }

    let mut hosts = ctx.inner.routes.hosts.lock();
    let st = hosts.entry(host).or_default();
    if ctl.best > 0.0 {
        st.best_conns = ctl.best_target.max(1);
        st.best_speed = ctl.best;
    }
    if ctl.cap < ctx.inner.settings.read().max_connections_per_task {
        st.ceiling = Some(ctl.cap);
    }
    drop(hosts);

    if let Some(e) = sh.fatal.lock().clone() {
        return Err(e);
    }
    if sh.cancel.is_cancelled() {
        return Err(DlError::cancelled());
    }
    let complete = sh.table.lock().as_ref().map(|t| t.complete()).unwrap_or(false);
    if !complete {
        return Err(DlError::new(ErrorKind::Network, "部分分段未完成"));
    }
    Ok(())
}

fn evict_stragglers(ctx: &Ctx, dt: f64) {
    let min_split = ctx.min_split;
    let mut g = ctx.sh.table.lock();
    let Some(t) = g.as_mut() else { return };
    let mut speeds = Vec::new();
    for s in t.segs.iter_mut() {
        if s.worker.is_some() {
            let inst = s.window as f64 / dt;
            s.speed = if s.speed == 0.0 { inst } else { s.speed * 0.5 + inst * 0.5 };
            s.window = 0;
            speeds.push(s.speed);
        }
    }
    if speeds.len() < 2 {
        return;
    }
    speeds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = speeds[speeds.len() / 2];
    for s in t.segs.iter_mut() {
        let age = s.claimed_at.map(|c| c.elapsed()).unwrap_or_default();
        let stalled = s.speed < 1.0 && median > 64.0 * 1024.0 && age > Duration::from_secs(3);
        if s.worker.is_some() && (stalled || (age > Duration::from_secs(6) && s.speed < median * 0.15 && s.left() > 4 * min_split)) {
            s.abort = true;
        }
    }
}

async fn failover(ctx: &Ctx) {
    let cur = ctx.sh.route.read().as_ref().map(|r| r.name.clone()).unwrap_or_default();
    let url = ctx.urls[0].clone();
    if let Ok((r, _)) = ctx.inner.routes.resolve(&ctx.spec, &url, &ctx.ua, Some(&cur)).await {
        if r.name != cur {
            ctx.log(format!("线路切换 {cur} -> {}", r.name));
            ctx.entry.with(|rec| rec.route = Some(r.name.clone()));
            *ctx.sh.route.write() = Some(r);
            ctx.sh.route_errors.store(0, Ordering::Relaxed);
        }
    }
}

/// Returns true when the worker retired itself (and already decremented `active`).
async fn worker(ctx: Arc<Ctx>, wid: u32) -> bool {
    let sh = &ctx.sh;
    let mut attempt = 0u32;
    let mut buf = Vec::with_capacity(FLUSH_AT + 256 * 1024);
    loop {
        if sh.cancel.is_cancelled() {
            return false;
        }
        let a = sh.active.load(Ordering::Relaxed);
        if a > sh.target.load(Ordering::Relaxed) && sh.active.compare_exchange(a, a - 1, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
            return true;
        }
        let min = ctx.eff_min_split();
        let idx = {
            let mut g = sh.table.lock();
            match g.as_mut().and_then(|t| t.claim(wid, min)) {
                Some(i) => i,
                None => return false,
            }
        };
        let res = segment(&ctx, idx, wid, &mut buf).await;
        if let Some(t) = sh.table.lock().as_mut() {
            t.release(idx);
        }
        match res {
            Ok(()) => attempt = 0,
            Err(e) if e.kind == ErrorKind::Cancelled => return false,
            Err(e) => {
                attempt += 1;
                tracing::debug!("worker {wid} segment error: {e}");
                match e.kind {
                    ErrorKind::RangeUnsupported | ErrorKind::ResourceChanged | ErrorKind::DiskFull | ErrorKind::Io | ErrorKind::NotFound => {
                        sh.fail(e);
                        return false;
                    }
                    ErrorKind::Auth => {
                        // Some CDNs 403 on connection floods; only fatal if it keeps happening.
                        sh.throttles.fetch_add(1, Ordering::Relaxed);
                        if attempt >= 3 {
                            sh.fail(e);
                            return false;
                        }
                    }
                    ErrorKind::ServerBusy => {
                        sh.throttles.fetch_add(1, Ordering::Relaxed);
                    }
                    k if k.route_related() => {
                        sh.route_errors.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
                if attempt > ctx.retries {
                    if sh.active.load(Ordering::Relaxed) <= 1 {
                        sh.fail(e);
                    }
                    return false;
                }
                ctx.log(format!("连接 #{wid} {}，第 {attempt} 次重试", e.kind.label()));
                tokio::select! {
                    _ = tokio::time::sleep(backoff(attempt, e.retry_after)) => {}
                    _ = sh.cancel.cancelled() => return false,
                }
            }
        }
    }
}

async fn flush(ctx: &Ctx, idx: usize, buf: &mut Vec<u8>, off: &mut u64) -> DlResult<()> {
    if buf.is_empty() {
        return Ok(());
    }
    let n = buf.len() as u64;
    let data = std::mem::take(buf);
    let mut back = write_at(&ctx.file, data, *off).await?;
    back.clear();
    *buf = back;
    *off += n;
    if let Some(t) = ctx.sh.table.lock().as_mut() {
        t.commit(idx, n);
    }
    Ok(())
}

fn baidu_pcs_host(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.ends_with("baidupcs.com") || h.ends_with("pcs.baidu.com")))
        .unwrap_or(false)
}

async fn segment(ctx: &Ctx, idx: usize, wid: u32, buf: &mut Vec<u8>) -> DlResult<()> {
    let sh = &ctx.sh;
    let (start, end) = {
        let g = sh.table.lock();
        let s = &g.as_ref().unwrap().segs[idx];
        (s.pos, s.end)
    };
    if start >= end {
        return Ok(());
    }
    let route = sh.route.read().clone().ok_or_else(|| DlError::new(ErrorKind::Network, "no route"))?;
    let url = &ctx.urls[wid as usize % ctx.urls.len()];
    let req_end = ctx.max_req.map_or(end, |m| end.min(start + m));
    let mut req = route.client.get(url).headers(ctx.headers()).header(reqwest::header::RANGE, format!("bytes={start}-{}", req_end - 1));
    if let Some(v) = &ctx.validator {
        req = req.header(reqwest::header::IF_RANGE, v.as_str());
    }
    let resp = tokio::select! {
        r = tokio::time::timeout(Duration::from_secs(25), req.send()) => match r {
            Ok(r) => r?,
            Err(_) => return Err(DlError::new(ErrorKind::Timeout, "等待响应超时")),
        },
        _ = sh.cancel.cancelled() => return Err(DlError::cancelled()),
    };
    let status = resp.status().as_u16();
    match status {
        206 => {
            let cr = resp.headers().get(reqwest::header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(parse_content_range);
            if let Some((a, _, _)) = cr {
                if a != start {
                    return Err(DlError::new(ErrorKind::RangeUnsupported, format!("Content-Range 起点 {a} != {start}")));
                }
            }
        }
        200 => {
            return Err(if ctx.validator.is_some() {
                DlError::new(ErrorKind::ResourceChanged, "If-Range 未命中")
            } else {
                DlError::new(ErrorKind::RangeUnsupported, "服务器忽略 Range")
            });
        }
        s => return Err(DlError::from_status(s, retry_after(resp.headers()))),
    }

    let mut stream = resp.bytes_stream();
    let mut off = start;
    let mut idle = Duration::ZERO;
    buf.clear();
    loop {
        let next = tokio::select! {
            n = tokio::time::timeout(Duration::from_secs(1), stream.next()) => n,
            _ = sh.cancel.cancelled() => {
                flush(ctx, idx, buf, &mut off).await?;
                return Err(DlError::cancelled());
            }
        };
        let chunk = match next {
            Err(_) => {
                // No data this second: honour straggler eviction even on a dead socket.
                idle += Duration::from_secs(1);
                let abort = sh.table.lock().as_ref().map(|t| t.segs[idx].abort).unwrap_or(false);
                if abort {
                    flush(ctx, idx, buf, &mut off).await?;
                    return Ok(());
                }
                if idle >= STALL_TIMEOUT {
                    flush(ctx, idx, buf, &mut off).await?;
                    return Err(DlError::new(ErrorKind::Timeout, "读取超时"));
                }
                continue;
            }
            Ok(None) => {
                flush(ctx, idx, buf, &mut off).await?;
                let complete = sh.table.lock().as_ref().map(|t| t.segs[idx].finished()).unwrap_or(false);
                return if complete || (req_end < end && off >= req_end) { Ok(()) } else { Err(DlError::new(ErrorKind::Network, "连接提前关闭")) };
            }
            Ok(Some(Err(e))) => {
                flush(ctx, idx, buf, &mut off).await?;
                return Err(DlError::from_reqwest(&e));
            }
            Ok(Some(Ok(b))) => b,
        };
        idle = Duration::ZERO;
        ctx.inner.limiter.acquire(chunk.len()).await;
        let r = {
            let mut g = sh.table.lock();
            g.as_mut().unwrap().reserve(idx, chunk.len())
        };
        sh.received.fetch_add(r.take as u64, Ordering::Relaxed);
        buf.extend_from_slice(&chunk[..r.take]);
        if r.complete || r.abort || buf.len() >= FLUSH_AT {
            flush(ctx, idx, buf, &mut off).await?;
        }
        if r.complete || r.abort {
            return Ok(());
        }
    }
}

async fn single_stream(ctx: Arc<Ctx>, size: Option<u64>) -> DlResult<u64> {
    let sh = &ctx.sh;
    let stop = sh.cancel.child_token();
    let _guard = stop.clone().drop_guard();
    {
        let sh = sh.clone();
        tokio::spawn(async move {
            let mut last = sh.received.load(Ordering::Relaxed);
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.tick().await;
            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = stop.cancelled() => return,
                }
                let rx = sh.received.load(Ordering::Relaxed);
                let sp = rx.saturating_sub(last) as f64;
                last = rx;
                let prev = sh.speed();
                let v = if prev == 0.0 { sp } else { prev * 0.6 + sp * 0.4 };
                sh.speed.store(v.to_bits(), Ordering::Relaxed);
                let mut h = sh.history.lock();
                if h.len() >= 120 {
                    h.pop_front();
                }
                h.push_back(sp as f32);
            }
        });
    }
    let mut attempt = 0u32;
    // Bytes already on disk from earlier attempts; resumed with a Range request when possible.
    let mut have = 0u64;
    loop {
        let route = sh.route.read().clone().ok_or_else(|| DlError::new(ErrorKind::Network, "no route"))?;
        let res: DlResult<u64> = async {
            let mut req = route.client.get(&ctx.urls[0]).headers(ctx.headers());
            if have > 0 {
                req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
                if let Some(v) = &ctx.validator {
                    req = req.header(reqwest::header::IF_RANGE, v.as_str());
                }
            }
            let resp = req.send().await?;
            let st = resp.status().as_u16();
            if !(200..300).contains(&st) {
                return Err(DlError::from_status(st, retry_after(resp.headers())));
            }
            let resumed = have > 0
                && st == 206
                && resp.headers().get(reqwest::header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(parse_content_range).is_some_and(|(a, _, _)| a == have);
            if have > 0 {
                ctx.log(if resumed { format!("从 {:.1} MB 处续传", have as f64 / 1048576.0) } else { "服务器不支持续传，从头下载".into() });
            }
            if !resumed {
                have = 0;
                ctx.file.set_len(0)?;
                if let Some(t) = sh.table.lock().as_mut() {
                    *t = Table::fresh(t.size, 1, u64::MAX / 4);
                }
            }
            sh.stream_done.store(have, Ordering::Relaxed);
            if let Some(t) = sh.table.lock().as_mut() {
                t.claim(0, u64::MAX / 4);
            }
            let mut stream = resp.bytes_stream();
            let mut buf = Vec::with_capacity(FLUSH_AT + 256 * 1024);
            let end = loop {
                let next = tokio::select! {
                    n = tokio::time::timeout(CHUNK_TIMEOUT, stream.next()) => n,
                    _ = sh.cancel.cancelled() => break Err(DlError::cancelled()),
                };
                let chunk = match next {
                    Err(_) => break Err(DlError::new(ErrorKind::Timeout, "读取超时")),
                    Ok(None) => break Ok(()),
                    Ok(Some(Err(e))) => break Err(DlError::from_reqwest(&e)),
                    Ok(Some(Ok(b))) => b,
                };
                ctx.inner.limiter.acquire(chunk.len()).await;
                sh.received.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                buf.extend_from_slice(&chunk);
                if buf.len() >= FLUSH_AT {
                    stream_flush(&ctx, &mut buf, &mut have).await?;
                }
            };
            stream_flush(&ctx, &mut buf, &mut have).await?;
            end?;
            let off = have;
            if let Some(s) = size {
                if off != s {
                    return Err(DlError::new(ErrorKind::Network, format!("长度不符 {off}/{s}")));
                }
            }
            if let Some(t) = sh.table.lock().as_mut() {
                for s in t.segs.iter_mut() {
                    s.done = s.end;
                    s.pos = s.end;
                }
            }
            Ok(off)
        }
        .await;
        match res {
            Ok(n) => {
                ctx.entry.with(|r| r.size = Some(n));
                return Ok(n);
            }
            Err(e) if e.kind == ErrorKind::Cancelled => return Err(e),
            Err(e) if e.kind.retryable() && attempt < ctx.retries => {
                attempt += 1;
                if e.kind.route_related() && attempt % 2 == 0 {
                    failover(&ctx).await;
                }
                ctx.log(format!("{}，第 {attempt} 次重试", e.kind.label()));
                tokio::select! {
                    _ = tokio::time::sleep(backoff(attempt, e.retry_after)) => {}
                    _ = sh.cancel.cancelled() => return Err(DlError::cancelled()),
                }
            }
            Err(e) => return Err(e),
        }
    }
}

async fn stream_flush(ctx: &Ctx, buf: &mut Vec<u8>, off: &mut u64) -> DlResult<()> {
    if buf.is_empty() {
        return Ok(());
    }
    let n = buf.len() as u64;
    let mut b = write_at(&ctx.file, std::mem::take(buf), *off).await?;
    b.clear();
    *buf = b;
    *off += n;
    ctx.sh.stream_done.store(*off, Ordering::Relaxed);
    if let Some(t) = ctx.sh.table.lock().as_mut() {
        if t.segs.len() == 1 {
            t.reserve(0, n as usize);
            t.commit(0, n);
        }
    }
    Ok(())
}

/// A plain HTML response without an attachment filename is a web page, not a download.
pub fn is_web_page(info: &crate::probe::ProbeInfo) -> bool {
    info.filename.is_none() && info.content_type.as_deref().is_some_and(|t| t.trim_start().to_ascii_lowercase().starts_with("text/html"))
}
