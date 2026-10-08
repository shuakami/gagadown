use crate::config::Settings;
use crate::download::{self, part_path, Shared, PART_EXT};
use crate::error::{DlError, DlResult, ErrorKind};
use crate::limiter::Limiter;
use crate::probe::{sanitize_filename, ProbeInfo};
use crate::request::RequestSpec;
use crate::route::{detect_local_proxies, HostStat, RouteManager};
use crate::task::*;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use uuid::Uuid;

/// Browser hand-off popups: shown as soon as a download arrives, bound to a task once the
/// probe accepts it, or dropped when the browser keeps the download.
#[derive(Clone, Debug)]
pub enum PopupEvent {
    Pending { key: Uuid, filename: String, size: Option<u64>, host: String },
    Bound { key: Uuid, id: Uuid },
    Dropped { key: Uuid },
    /// Hand-off failed; the browser keeps the download. `reason` is shown to the user.
    Failed { key: Uuid, reason: String },
}

pub struct RunHandle {
    pub shared: Arc<Shared>,
    pub join: Option<tokio::task::JoinHandle<()>>,
}

pub struct TaskEntry {
    pub rec: Mutex<TaskRecord>,
    pub run: Mutex<Option<RunHandle>>,
    operation: tokio::sync::Mutex<()>,
}

impl TaskEntry {
    pub fn with<R>(&self, f: impl FnOnce(&mut TaskRecord) -> R) -> R {
        f(&mut self.rec.lock())
    }

    pub fn snapshot_record(&self) -> TaskRecord {
        self.rec.lock().clone()
    }

    pub fn log(&self, s: impl Into<String>) {
        let mut r = self.rec.lock();
        r.log.push(LogLine { at: now_secs(), text: s.into() });
        if r.log.len() > 60 {
            let n = r.log.len() - 60;
            r.log.drain(..n);
        }
    }

    fn shared(&self) -> Option<Arc<Shared>> {
        self.run.lock().as_ref().map(|h| h.shared.clone())
    }

    /// Record with live progress folded in, ready to persist.
    fn persistable(&self) -> TaskRecord {
        let mut r = self.rec.lock().clone();
        if let Some(sh) = self.shared() {
            if let Some(t) = sh.table.lock().as_ref() {
                if r.ranges && !sh.single.load(Ordering::Relaxed) {
                    r.remaining = Some(t.remaining());
                }
                r.downloaded = t.downloaded();
            }
        }
        r
    }
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub at: u64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Default)]
struct State {
    tasks: Vec<TaskRecord>,
    deleted: Vec<DeletedRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheKind {
    Active,
    Deleted,
    Orphan,
}

#[derive(Clone, Debug)]
pub struct CacheItem {
    pub path: PathBuf,
    pub bytes: u64,
    pub kind: CacheKind,
    pub modified: u64,
    pub task: Option<Uuid>,
}

#[derive(Clone, Debug, Default)]
pub struct CacheReport {
    pub items: Vec<CacheItem>,
    pub active_bytes: u64,
    pub deleted_bytes: u64,
    pub orphan_bytes: u64,
    pub missing_files: usize,
    pub free_space: Option<u64>,
}

pub struct Inner {
    pub settings: RwLock<Settings>,
    pub data_dir: PathBuf,
    pub tasks: RwLock<Vec<Arc<TaskEntry>>>,
    pub deleted: Mutex<Vec<DeletedRecord>>,
    pub routes: RouteManager,
    pub limiter: Limiter,
    pub conn_budget: Arc<Semaphore>,
    budget_size: Mutex<usize>,
    dirty: AtomicBool,
    pub notices: Mutex<VecDeque<Notice>>,
    popups: Mutex<VecDeque<PopupEvent>>,
    waker: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
    detecting: AtomicBool,
    browsers_seen: Mutex<HashMap<String, u64>>,
    /// Tasks are spawned here, so sync calls from non-runtime threads (the UI) work.
    rt: tokio::runtime::Handle,
}

impl Inner {
    pub fn taken_paths(&self, except: Uuid) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self
            .tasks
            .read()
            .iter()
            .filter_map(|t| {
                let r = t.rec.lock();
                if r.id == except { None } else { r.path.clone() }
            })
            .collect();
        v.extend(self.deleted.lock().iter().filter(|d| d.rec.status != Status::Completed).filter_map(|d| d.rec.path.clone()));
        v
    }

    fn notify(&self, text: impl Into<String>) {
        let mut n = self.notices.lock();
        n.push_back(Notice { at: now_secs(), text: text.into() });
        while n.len() > 20 {
            n.pop_front();
        }
    }

    fn find(&self, id: Uuid) -> Option<Arc<TaskEntry>> {
        self.tasks.read().iter().find(|t| t.rec.lock().id == id).cloned()
    }

    fn finish(&self, entry: &Arc<TaskEntry>, sh: &Arc<Shared>, res: DlResult<download::Outcome>, started: std::time::Instant) {
        let reason = sh.stop.load(Ordering::Relaxed);
        let live = {
            let t = sh.table.lock();
            t.as_ref().map(|t| (t.remaining(), t.downloaded()))
        };
        let mut r = entry.rec.lock();
        r.elapsed_secs += started.elapsed().as_secs_f64();
        if let Some((rem, done)) = &live {
            if r.ranges && !sh.single.load(Ordering::Relaxed) {
                r.remaining = Some(rem.clone());
            } else {
                r.remaining = None;
            }
            r.downloaded = *done;
        }
        match res {
            Ok(out) => {
                r.status = Status::Completed;
                r.error = None;
                r.remaining = None;
                r.downloaded = out.size;
                r.size = Some(out.size);
                r.path = Some(out.final_path);
                r.finished_at = Some(now_secs());
                r.next_retry_at = None;
                let name = r.filename.clone();
                drop(r);
                self.notify(format!("下载完成 {name}"));
            }
            Err(e) if e.kind == ErrorKind::Cancelled || reason != StopReason::None as u8 => {
                r.status = if reason == StopReason::Shutdown as u8 { Status::Queued } else { Status::Paused };
            }
            Err(e) => {
                let settings = self.settings.read();
                r.status = Status::Failed;
                r.error = Some(TaskError { kind: e.kind, message: e.message.clone(), at: now_secs(), status: e.status, routes: e.routes.clone() });
                if e.kind.retryable() && r.auto_retries < settings.task_auto_retries {
                    let wait = [15u64, 60, 180, 600, 1800][(r.auto_retries as usize).min(4)];
                    r.next_retry_at = Some(now_secs() + wait);
                } else {
                    r.next_retry_at = None;
                }
                let reason = crate::error::summary(e.kind, e.status, &e.message);
                let line = format!("失败: {reason}");
                r.log.push(LogLine { at: now_secs(), text: line });
                let name = r.filename.clone();
                drop(r);
                self.notify(format!("下载失败 {name}: {reason}"));
            }
        }
        *entry.run.lock() = None;
        self.dirty.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub struct Engine {
    pub inner: Arc<Inner>,
}

fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> Option<T> {
    let s = std::fs::read(p).ok()?;
    serde_json::from_slice(&s).ok()
}

pub fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "gagadown", "GagaDown")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("gagadown"))
}

impl Engine {
    /// Must be called inside a tokio runtime.
    pub fn new(data_dir: Option<PathBuf>) -> DlResult<Self> {
        let data_dir = data_dir.unwrap_or_else(default_data_dir);
        std::fs::create_dir_all(&data_dir)?;
        let settings: Settings = read_json::<Settings>(&data_dir.join("settings.json")).unwrap_or_default().sanitized();
        let state: State = read_json(&data_dir.join("state.json")).unwrap_or_default();
        let hosts: HashMap<String, HostStat> = read_json(&data_dir.join("hosts.json")).unwrap_or_default();
        let routes = RouteManager::new(settings.proxy.clone())?;
        *routes.hosts.lock() = hosts;
        let tasks = state
            .tasks
            .into_iter()
            .map(|mut r| {
                if r.status == Status::Running {
                    r.status = Status::Queued;
                }
                Arc::new(TaskEntry { rec: Mutex::new(r), run: Mutex::new(None), operation: tokio::sync::Mutex::new(()) })
            })
            .collect();
        download::set_cache_dir(settings.cache_dir.clone());
        let total = settings.max_connections_total;
        let inner = Arc::new(Inner {
            limiter: Limiter::new(settings.speed_limit),
            conn_budget: Arc::new(Semaphore::new(total)),
            budget_size: Mutex::new(total),
            settings: RwLock::new(settings),
            data_dir,
            tasks: RwLock::new(tasks),
            deleted: Mutex::new(state.deleted),
            routes,
            dirty: AtomicBool::new(false),
            notices: Mutex::new(VecDeque::new()),
            popups: Mutex::new(VecDeque::new()),
            waker: RwLock::new(None),
            detecting: AtomicBool::new(false),
            browsers_seen: Mutex::new(HashMap::new()),
            rt: tokio::runtime::Handle::current(),
        });
        let engine = Engine { inner };
        engine.spawn_scheduler();
        if engine.settings().proxy.auto_detect {
            engine.detect_proxies();
        }
        Ok(engine)
    }

    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    /// Records a ping from the browser extension running in `kind` (`chrome`, `edge`).
    pub fn browser_seen(&self, kind: &str) {
        self.inner.browsers_seen.lock().insert(kind.to_string(), now_secs());
    }

    /// Unix time of the extension's last ping from `kind`.
    pub fn browser_last_seen(&self, kind: &str) -> Option<u64> {
        self.inner.browsers_seen.lock().get(kind).copied()
    }

    pub fn settings(&self) -> Settings {
        self.inner.settings.read().clone()
    }

    pub fn set_settings(&self, s: Settings) -> DlResult<()> {
        let s = s.sanitized();
        download::set_cache_dir(s.cache_dir.clone());
        let old = self.settings();
        self.inner.limiter.set_rate(s.speed_limit);
        if old.proxy.proxies != s.proxy.proxies || old.proxy.mode != s.proxy.mode || old.proxy.use_system != s.proxy.use_system || old.proxy.auto_detect != s.proxy.auto_detect || old.proxy.hedge_delay_ms != s.proxy.hedge_delay_ms {
            self.inner.routes.update_settings(s.proxy.clone())?;
        }
        {
            let mut size = self.inner.budget_size.lock();
            if s.max_connections_total > *size {
                self.inner.conn_budget.add_permits(s.max_connections_total - *size);
            } else if s.max_connections_total < *size {
                self.inner.conn_budget.forget_permits(*size - s.max_connections_total);
            }
            *size = s.max_connections_total;
        }
        *self.inner.settings.write() = s.clone();
        let data = serde_json::to_vec_pretty(&s).unwrap_or_default();
        atomic_write(&self.inner.data_dir.join("settings.json"), &data)?;
        Ok(())
    }

    pub fn detect_proxies(&self) {
        if self.inner.detecting.swap(true, Ordering::AcqRel) {
            return;
        }
        let inner = self.inner.clone();
        self.inner.rt.spawn(async move {
            let found = detect_local_proxies().await;
            let _ = inner.routes.set_detected(found);
            inner.detecting.store(false, Ordering::Release);
        });
    }

    pub fn is_detecting(&self) -> bool {
        self.inner.detecting.load(Ordering::Relaxed)
    }

    pub fn detected_proxies(&self) -> Vec<String> {
        self.inner.routes.detected()
    }

    pub fn route_names(&self) -> Vec<String> {
        self.inner.routes.ordered(None).iter().map(|r| r.name.clone()).collect()
    }

    /// Called from any thread whenever the UI has something new to show.
    pub fn set_waker(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.inner.waker.write() = Some(Arc::new(f));
    }

    pub fn push_popup(&self, ev: PopupEvent) {
        self.inner.popups.lock().push_back(ev);
        if let Some(w) = self.inner.waker.read().clone() {
            w();
        }
    }

    pub fn take_popups(&self) -> Vec<PopupEvent> {
        self.inner.popups.lock().drain(..).collect()
    }

    pub fn take_notices(&self) -> Vec<Notice> {
        self.inner.notices.lock().drain(..).collect()
    }

    fn spawn_scheduler(&self) {
        let eng = self.clone();
        self.inner.rt.spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(400));
            let mut n: u64 = 0;
            loop {
                tick.tick().await;
                n += 1;
                eng.schedule();
                if n % 5 == 0 {
                    let running = eng.inner.tasks.read().iter().any(|t| t.run.lock().is_some());
                    if running || eng.inner.dirty.swap(false, Ordering::Relaxed) {
                        let _ = eng.save();
                    }
                }
                if n % 150 == 1 {
                    eng.housekeeping();
                }
            }
        });
    }

    fn schedule(&self) {
        let max = self.inner.settings.read().max_concurrent_tasks;
        let now = now_secs();
        let tasks = self.inner.tasks.read().clone();
        let mut running = tasks.iter().filter(|t| t.run.lock().is_some()).count();
        for t in &tasks {
            let mut r = t.rec.lock();
            if r.status == Status::Failed {
                if let Some(at) = r.next_retry_at {
                    if at <= now {
                        r.next_retry_at = None;
                        r.auto_retries += 1;
                        r.status = Status::Queued;
                        let line = format!("自动重试（第 {} 次）", r.auto_retries);
                        r.log.push(LogLine { at: now, text: line });
                    }
                }
            }
        }
        for t in &tasks {
            if running >= max {
                break;
            }
            let queued = t.rec.lock().status == Status::Queued && t.run.lock().is_none();
            if queued {
                self.start(t.clone());
                running += 1;
            }
        }
    }

    fn start(&self, entry: Arc<TaskEntry>) {
        let Ok(_operation) = entry.operation.try_lock() else { return };
        if entry.rec.lock().status != Status::Queued || entry.run.lock().is_some() {
            return;
        }
        let sh = Arc::new(Shared::new());
        entry.with(|r| {
            r.status = Status::Running;
            r.error = None;
        });
        let inner = self.inner.clone();
        let e = entry.clone();
        let s = sh.clone();
        let mut run = entry.run.lock();
        *run = Some(RunHandle { shared: sh, join: None });
        let join = self.inner.rt.spawn(async move {
            let started = std::time::Instant::now();
            let res = download::run(inner.clone(), e.clone(), s.clone()).await;
            inner.finish(&e, &s, res, started);
        });
        if let Some(h) = run.as_mut() {
            h.join = Some(join);
        }
    }

    pub fn save(&self) -> DlResult<()> {
        let tasks: Vec<TaskRecord> = self.inner.tasks.read().iter().map(|t| t.persistable()).collect();
        let deleted = self.inner.deleted.lock().clone();
        let st = State { tasks, deleted };
        let data = serde_json::to_vec(&st).map_err(|e| DlError::new(ErrorKind::Io, e.to_string()))?;
        atomic_write(&self.inner.data_dir.join("state.json"), &data)?;
        let hosts = self.inner.routes.hosts.lock().clone();
        let data = serde_json::to_vec(&hosts).unwrap_or_default();
        atomic_write(&self.inner.data_dir.join("hosts.json"), &data)?;
        Ok(())
    }

    fn housekeeping(&self) {
        let s = self.settings();
        let now = now_secs();
        let expired: Vec<Uuid> = self
            .inner
            .deleted
            .lock()
            .iter()
            .filter(|d| now.saturating_sub(d.deleted_at) > s.deleted_retention_days * 86400)
            .map(|d| d.rec.id)
            .collect();
        for id in expired {
            self.purge_deleted(Some(id));
        }
        if s.orphan_auto_clean_days > 0 {
            let rep = self.cache_report();
            for it in rep.items.iter().filter(|i| i.kind == CacheKind::Orphan) {
                if now.saturating_sub(it.modified) > s.orphan_auto_clean_days * 86400 {
                    let _ = std::fs::remove_file(&it.path);
                }
            }
        }
    }

    // ---- task operations ----

    pub async fn probe(&self, spec: &RequestSpec) -> DlResult<ProbeInfo> {
        let ua = self.inner.settings.read().user_agent.clone();
        let (r, info) = self.inner.routes.resolve(spec, &spec.url, &ua, None).await?;
        self.inner.routes.stash(&spec.url, &r.name, &info);
        Ok(info)
    }

    pub fn add(&self, req: AddRequest, probe: Option<&ProbeInfo>) -> DlResult<AddOutcome> {
        let url = req.url.trim().to_string();
        let parsed = url::Url::parse(&url).map_err(|e| DlError::new(ErrorKind::Other, format!("链接无效: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(DlError::new(ErrorKind::Other, "只支持 http/https 链接"));
        }
        let filename = req
            .filename
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| probe.and_then(|p| p.filename.clone()))
            .map(|s| sanitize_filename(&s))
            .unwrap_or_default();
        let size = probe.and_then(|p| p.size).or(req.size_hint);
        let spec = RequestSpec {
            url: url.clone(),
            mirrors: req.mirrors.clone(),
            headers: req.headers.clone(),
            cookies: req.cookies.clone(),
            referrer: req.referrer.clone(),
            user_agent: req.user_agent.clone(),
        };

        let tasks = self.inner.tasks.read().clone();
        for t in &tasks {
            let mut r = t.rec.lock();
            if r.spec.url == url && r.status != Status::Completed {
                if matches!(r.status, Status::Failed | Status::Paused) {
                    r.spec = spec.clone();
                    r.status = Status::Queued;
                    r.error = None;
                }
                return Ok(AddOutcome { id: r.id, existed: true, refreshed: false, filename: r.filename.clone() });
            }
        }
        if req.allow_refresh && !filename.is_empty() {
            for t in &tasks {
                let mut r = t.rec.lock();
                let stuck = r.status == Status::Failed || r.status == Status::Paused;
                let same_size = match (r.size, size) {
                    (Some(a), Some(b)) => a == b,
                    _ => false,
                };
                if stuck && same_size && r.filename.eq_ignore_ascii_case(&filename) {
                    r.spec = spec.clone();
                    r.status = Status::Queued;
                    r.error = None;
                    r.auto_retries = 0;
                    r.log.push(LogLine { at: now_secs(), text: "已从浏览器续上新链接".into() });
                    self.inner.dirty.store(true, Ordering::Relaxed);
                    return Ok(AddOutcome { id: r.id, existed: true, refreshed: true, filename: r.filename.clone() });
                }
            }
        }

        let settings = self.settings();
        let rec = TaskRecord {
            id: Uuid::new_v4(),
            spec,
            dir: req.dir.clone().unwrap_or(settings.download_dir.clone()),
            filename: filename.clone(),
            path: None,
            size,
            ranges: probe.map(|p| p.ranges).unwrap_or(false),
            etag: None,
            last_modified: None,
            content_type: probe.and_then(|p| p.content_type.clone()),
            final_url: None,
            status: if req.start_paused { Status::Paused } else { Status::Queued },
            error: None,
            auto_retries: 0,
            next_retry_at: None,
            remaining: None,
            downloaded: 0,
            created_at: now_secs(),
            finished_at: None,
            sha256: req.sha256.clone(),
            route: probe.map(|p| p.route.clone()),
            elapsed_secs: 0.0,
            source: req.source.clone().unwrap_or_else(|| "manual".into()),
            log: Vec::new(),
        };
        let id = rec.id;
        let entry = Arc::new(TaskEntry { rec: Mutex::new(rec), run: Mutex::new(None), operation: tokio::sync::Mutex::new(()) });
        // Size alone is an estimate until the real probe; the run overwrites it.
        entry.with(|r| r.size = None);
        self.inner.tasks.write().insert(0, entry);
        self.inner.dirty.store(true, Ordering::Relaxed);
        self.schedule();
        Ok(AddOutcome { id, existed: false, refreshed: false, filename })
    }

    pub fn pause(&self, id: Uuid) {
        let Some(t) = self.inner.find(id) else { return };
        if let Some(sh) = t.shared() {
            sh.stop.store(StopReason::Pause as u8, Ordering::Relaxed);
            sh.cancel.cancel();
        }
        t.with(|r| {
            if matches!(r.status, Status::Queued | Status::Running | Status::Failed) {
                r.status = Status::Paused;
                r.next_retry_at = None;
            }
        });
        self.inner.dirty.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self, id: Uuid) {
        let Some(t) = self.inner.find(id) else { return };
        let Ok(operation) = t.operation.try_lock() else { return };
        t.with(|r| {
            if matches!(r.status, Status::Paused | Status::Failed) {
                r.status = Status::Queued;
                r.error = None;
                r.next_retry_at = None;
                r.auto_retries = 0;
            }
        });
        drop(operation);
        self.schedule();
    }

    pub fn pause_all(&self) {
        let ids: Vec<Uuid> = self.inner.tasks.read().iter().map(|t| t.rec.lock().id).collect();
        for id in ids {
            self.pause(id);
        }
    }

    pub fn resume_all(&self) {
        let ids: Vec<Uuid> = self.inner.tasks.read().iter().map(|t| t.rec.lock().id).collect();
        for id in ids {
            self.resume(id);
        }
    }

    /// Restart from zero, discarding partial data.
    pub async fn redownload(&self, id: Uuid) {
        let Some(t) = self.inner.find(id) else { return };
        let _operation = t.operation.lock().await;
        if !self.stop_and_wait(&t, StopReason::Pause).await {
            return;
        }
        t.with(|r| {
            if let Some(p) = &r.path {
                let _ = std::fs::remove_file(part_path(p));
                if r.status == Status::Completed {
                    r.path = None;
                }
            }
            r.remaining = None;
            r.downloaded = 0;
            r.status = Status::Queued;
            r.error = None;
            r.finished_at = None;
            r.elapsed_secs = 0.0;
        });
        self.schedule();
    }

    async fn stop_and_wait(&self, t: &Arc<TaskEntry>, reason: StopReason) -> bool {
        // Prevent queued/failed tasks from restarting while their files are removed.
        t.with(|r| {
            if r.status != Status::Completed {
                r.status = Status::Paused;
            }
            r.next_retry_at = None;
        });
        let join = {
            let mut g = t.run.lock();
            g.as_mut().map(|h| {
                h.shared.stop.store(reason as u8, Ordering::Relaxed);
                h.shared.cancel.cancel();
                h.join.take()
            })
        };
        if let Some(Some(mut j)) = join {
            // Never detach a run and then unlink its open file. Preallocation
            // and blocking writes must finish before Windows can reclaim it.
            if tokio::time::timeout(Duration::from_secs(10), &mut j).await.is_err() {
                self.inner.notify("正在等待磁盘操作结束，结束后将自动继续；无需再次点击");
                let _ = j.await;
            }
        }
        // Another operation may already be waiting for this run. Never unlink
        // its files while it still owns them.
        t.run.lock().is_none()
    }

    pub async fn remove(&self, id: Uuid, mode: RemoveMode) {
        let Some(t) = self.inner.find(id) else { return };
        let _operation = t.operation.lock().await;
        if self.inner.find(id).is_none() {
            return;
        }
        if !self.stop_and_wait(&t, StopReason::Remove).await {
            return;
        }
        let rec = t.persistable();
        let cleanup = (|| -> Result<(), String> {
            if let Some(p) = &rec.path {
                match mode {
                    RemoveMode::KeepFiles => {}
                    RemoveMode::TrashFiles => {
                        // Incomplete downloads have only a partial file. Do not
                        // leave it behind until the user deletes the record again.
                        let path = if rec.status == Status::Completed { p.clone() } else { part_path(p) };
                        if path.try_exists().map_err(|e| e.to_string())? {
                            trash::delete(&path).map_err(|e| e.to_string())?;
                        }
                    }
                    RemoveMode::DeleteFiles => {
                        for path in [part_path(p), p.clone()] {
                            match std::fs::remove_file(&path) {
                                Ok(()) => {}
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                                Err(e) => return Err(format!("{}: {e}", path.display())),
                            }
                        }
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = cleanup {
            t.log(format!("删除失败，保留任务以便重试: {e}"));
            self.inner.notify(format!("删除失败: {e}"));
            self.inner.dirty.store(true, Ordering::Relaxed);
            let _ = self.save();
            return;
        }
        self.inner.tasks.write().retain(|x| !Arc::ptr_eq(x, &t));
        if mode != RemoveMode::DeleteFiles {
            self.inner.deleted.lock().insert(0, DeletedRecord { rec, deleted_at: now_secs(), mode });
        }
        self.inner.dirty.store(true, Ordering::Relaxed);
        let _ = self.save();
    }

    pub fn restore(&self, id: Uuid) -> bool {
        let rec = {
            let mut d = self.inner.deleted.lock();
            let Some(i) = d.iter().position(|x| x.rec.id == id) else { return false };
            d.remove(i).rec
        };
        let mut rec = rec;
        let part_ok = rec.path.as_ref().map(|p| part_path(p).exists()).unwrap_or(false);
        let file_ok = rec.path.as_ref().map(|p| p.exists()).unwrap_or(false);
        if rec.status == Status::Completed {
            if !file_ok {
                rec.status = Status::Paused;
                rec.path = None;
                rec.remaining = None;
                rec.downloaded = 0;
                rec.finished_at = None;
            }
        } else {
            if !part_ok {
                rec.remaining = None;
                rec.downloaded = 0;
                rec.path = None;
            }
            rec.status = Status::Paused;
        }
        rec.log.push(LogLine { at: now_secs(), text: "已从最近删除恢复".into() });
        self.inner.tasks.write().insert(0, Arc::new(TaskEntry { rec: Mutex::new(rec), run: Mutex::new(None), operation: tokio::sync::Mutex::new(()) }));
        self.inner.dirty.store(true, Ordering::Relaxed);
        true
    }

    /// Empty the recently-deleted list; `mode` decides what happens to files still on disk.
    pub fn clear_deleted(&self, mode: RemoveMode) {
        if mode != RemoveMode::KeepFiles {
            let files: Vec<PathBuf> = self.inner.deleted.lock().iter().filter_map(|d| d.rec.path.clone()).filter(|p| p.is_file()).collect();
            for p in files {
                if self.inner.tasks.read().iter().any(|t| t.rec.lock().path.as_ref() == Some(&p)) {
                    continue;
                }
                if mode == RemoveMode::DeleteFiles || trash::delete(&p).is_err() {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
        self.purge_deleted(None);
        let _ = self.save();
    }

    /// Drop deleted records for good, removing their leftover partial files.
    pub fn purge_deleted(&self, id: Option<Uuid>) {
        let gone: Vec<DeletedRecord> = {
            let mut d = self.inner.deleted.lock();
            let (gone, keep): (Vec<_>, Vec<_>) = d.drain(..).partition(|x| id.map(|i| i == x.rec.id).unwrap_or(true));
            *d = keep;
            gone
        };
        for g in gone {
            if let Some(p) = &g.rec.path {
                let part = part_path(p);
                let still_used = self.inner.tasks.read().iter().any(|t| t.rec.lock().path.as_ref() == Some(p));
                if !still_used {
                    let _ = std::fs::remove_file(part);
                }
            }
        }
        self.inner.dirty.store(true, Ordering::Relaxed);
    }

    pub fn set_filename(&self, id: Uuid, name: &str) {
        let Some(t) = self.inner.find(id) else { return };
        t.with(|r| {
            if r.path.is_none() {
                r.filename = sanitize_filename(name);
            }
        });
    }

    pub fn retry_failed(&self) {
        let ids: Vec<Uuid> = self.inner.tasks.read().iter().filter(|t| t.rec.lock().status == Status::Failed).map(|t| t.rec.lock().id).collect();
        for id in ids {
            self.resume(id);
        }
    }

    pub async fn shutdown(&self) {
        let tasks = self.inner.tasks.read().clone();
        let mut joins = Vec::new();
        for t in &tasks {
            let mut g = t.run.lock();
            if let Some(h) = g.as_mut() {
                h.shared.stop.store(StopReason::Shutdown as u8, Ordering::Relaxed);
                h.shared.cancel.cancel();
                if let Some(j) = h.join.take() {
                    joins.push(j);
                }
            }
        }
        for j in joins {
            let _ = tokio::time::timeout(Duration::from_secs(5), j).await;
        }
        let _ = self.save();
    }

    // ---- views ----

    pub fn views(&self, seg_detail: usize) -> Vec<TaskView> {
        let tasks = self.inner.tasks.read().clone();
        tasks.iter().map(|t| view_of(t, seg_detail)).collect()
    }

    pub fn deleted(&self) -> Vec<DeletedRecord> {
        self.inner.deleted.lock().clone()
    }

    pub fn cache_report(&self) -> CacheReport {
        let settings = self.settings();
        let mut rep = CacheReport::default();
        let mut active: HashMap<PathBuf, Uuid> = HashMap::new();
        let mut dirs: Vec<PathBuf> = vec![settings.download_dir.clone()];
        dirs.extend(settings.cache_dir.clone());
        for t in self.inner.tasks.read().iter() {
            let r = t.rec.lock();
            if !dirs.contains(&r.dir) {
                dirs.push(r.dir.clone());
            }
            if let Some(p) = &r.path {
                if r.status == Status::Completed {
                    if !p.exists() {
                        rep.missing_files += 1;
                    }
                } else {
                    active.insert(part_path(p), r.id);
                }
            }
        }
        let mut deleted: HashMap<PathBuf, Uuid> = HashMap::new();
        for d in self.inner.deleted.lock().iter() {
            if !dirs.contains(&d.rec.dir) {
                dirs.push(d.rec.dir.clone());
            }
            if let Some(p) = &d.rec.path {
                deleted.insert(part_path(p), d.rec.id);
            }
        }
        for dir in dirs {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == PART_EXT).unwrap_or(false) {
                    let md = e.metadata().ok();
                    let bytes = md.as_ref().map(allocated_bytes).unwrap_or(0);
                    let modified = md
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let (kind, task) = if let Some(id) = active.get(&p) {
                        (CacheKind::Active, Some(*id))
                    } else if let Some(id) = deleted.get(&p) {
                        (CacheKind::Deleted, Some(*id))
                    } else {
                        (CacheKind::Orphan, None)
                    };
                    match kind {
                        CacheKind::Active => rep.active_bytes += bytes,
                        CacheKind::Deleted => rep.deleted_bytes += bytes,
                        CacheKind::Orphan => rep.orphan_bytes += bytes,
                    }
                    rep.items.push(CacheItem { path: p, bytes, kind, modified, task });
                }
            }
        }
        rep.free_space = fs4::available_space(&settings.download_dir).ok();
        rep.items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        rep
    }

    pub fn clean_cache(&self, orphans: bool, deleted: bool) -> u64 {
        let rep = self.cache_report();
        let mut freed = 0;
        for it in rep.items {
            if (orphans && it.kind == CacheKind::Orphan) || (deleted && it.kind == CacheKind::Deleted) {
                if std::fs::remove_file(&it.path).is_ok() {
                    freed += it.bytes;
                }
            }
        }
        if deleted {
            self.purge_deleted(None);
        }
        freed
    }

    /// Remove completed tasks whose file has been moved or deleted outside the app.
    pub fn prune_missing(&self) -> usize {
        let mut n = 0;
        self.inner.tasks.write().retain(|t| {
            let r = t.rec.lock();
            let gone = r.status == Status::Completed && r.path.as_ref().map(|p| !p.exists()).unwrap_or(true);
            if gone {
                n += 1;
            }
            !gone
        });
        self.inner.dirty.store(true, Ordering::Relaxed);
        n
    }

    pub fn clear_completed(&self) {
        self.inner.tasks.write().retain(|t| t.rec.lock().status != Status::Completed);
        self.inner.dirty.store(true, Ordering::Relaxed);
    }

    pub fn total_speed(&self) -> f64 {
        self.inner.tasks.read().iter().filter_map(|t| t.shared()).map(|s| s.speed()).sum()
    }
}

#[cfg(unix)]
fn allocated_bytes(m: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    (m.blocks() * 512).min(m.len().max(m.blocks() * 512))
}

#[cfg(not(unix))]
fn allocated_bytes(m: &std::fs::Metadata) -> u64 {
    m.len()
}

fn view_of(t: &Arc<TaskEntry>, seg_detail: usize) -> TaskView {
    let r = t.rec.lock().clone();
    let sh = t.shared();
    let mut v = TaskView {
        id: r.id,
        filename: if r.filename.is_empty() { r.spec.url.clone() } else { r.filename.clone() },
        url: r.spec.url.clone(),
        host: r.spec.host(),
        dir: r.dir.clone(),
        path: r.path.clone(),
        size: r.size,
        downloaded: r.downloaded,
        speed: 0.0,
        eta_secs: None,
        status: r.status,
        phase: None,
        error: r.error.clone(),
        connections: 0,
        target: 0,
        segments: Vec::new(),
        splits: 0,
        history: Vec::new(),
        route: r.route.clone(),
        created_at: r.created_at,
        finished_at: r.finished_at,
        next_retry_at: r.next_retry_at,
        file_missing: r.status == Status::Completed && r.path.as_ref().map(|p| !p.exists()).unwrap_or(true),
        ranges: r.ranges,
        avg_speed: if r.elapsed_secs > 0.5 { r.downloaded as f64 / r.elapsed_secs } else { 0.0 },
        log: r.log.clone(),
    };
    if let (Some(rem), Some(size)) = (&r.remaining, r.size) {
        if seg_detail > 0 && r.status != Status::Completed {
            let mut cur = 0;
            for (a, b) in rem {
                if *a > cur {
                    v.segments.push(SegView { start: cur, done: *a, end: *a, active: false });
                }
                v.segments.push(SegView { start: *a, done: *a, end: *b, active: false });
                cur = *b;
            }
            if cur < size {
                v.segments.push(SegView { start: cur, done: size, end: size, active: false });
            }
        }
    }
    if let Some(sh) = sh {
        v.phase = Some(Phase::from_u8(sh.phase.load(Ordering::Relaxed)));
        v.speed = sh.speed();
        v.connections = sh.active.load(Ordering::Relaxed);
        v.target = sh.target.load(Ordering::Relaxed);
        v.history = sh.history.lock().iter().copied().collect();
        if let Some(rt) = sh.route.read().as_ref() {
            v.route = Some(rt.name.clone());
        }
        if let Some(tb) = sh.table.lock().as_ref() {
            v.downloaded = tb.downloaded();
            v.splits = tb.splits;
            v.size = Some(tb.size);
            if seg_detail > 0 {
                v.segments = tb.view(seg_detail).into_iter().map(|(s, d, e, a)| SegView { start: s, done: d, end: e, active: a }).collect();
            }
        } else if sh.single.load(Ordering::Relaxed) {
            v.downloaded = sh.stream_done.load(Ordering::Relaxed);
        }
        if let Some(size) = v.size {
            if v.speed > 1.0 {
                v.eta_secs = Some(((size.saturating_sub(v.downloaded)) as f64 / v.speed) as u64);
            }
        }
    }
    v
}
