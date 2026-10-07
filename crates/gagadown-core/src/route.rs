//! Route manager: direct first, proxies race in after a hedge delay (happy-eyeballs style),
//! the winner is cached per host, and running tasks can hop routes when one goes bad.
use crate::config::{ProxyMode, ProxySettings};
use crate::error::{DlError, DlResult, ErrorKind};
use crate::probe::{probe, ProbeInfo};
use crate::request::RequestSpec;
use futures_util::stream::{FuturesUnordered, StreamExt};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Route {
    pub name: String,
    pub proxy: Option<String>,
    pub client: reqwest::Client,
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route").field("name", &self.name).finish()
    }
}

pub const DIRECT: &str = "直连";

/// Well-known local proxy ports: Clash/Mihomo, Clash Verge, v2rayN, Shadowsocks, Surge, sing-box, Qv2ray.
pub const LOCAL_PROXY_PORTS: &[u16] = &[7890, 7897, 7891, 7893, 10809, 10808, 1080, 1081, 2080, 2081, 6152, 6153, 8889, 20171, 20170, 8080, 8888, 9090, 12334, 33210];

fn display_proxy(p: &str) -> String {
    match url::Url::parse(p) {
        Ok(u) => format!("{}://{}:{}", u.scheme(), u.host_str().unwrap_or(""), u.port_or_known_default().unwrap_or(0)),
        Err(_) => p.to_string(),
    }
}

pub fn build_client(proxy: Option<&str>, system: bool) -> DlResult<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .http1_only()
        .connect_timeout(Duration::from_secs(10))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(30))
        .pool_max_idle_per_host(512)
        .pool_idle_timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::limited(20));
    match proxy {
        Some(p) => {
            let px = reqwest::Proxy::all(p).map_err(|e| DlError::new(ErrorKind::Other, format!("代理地址无效 {p}: {e}")))?;
            b = b.proxy(px);
        }
        None if !system => b = b.no_proxy(),
        None => {}
    }
    b.build().map_err(|e| DlError::new(ErrorKind::Other, e.to_string()))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HostStat {
    /// Connection count that produced the best throughput last time.
    pub best_conns: usize,
    /// Learned upper bound after the server throttled us.
    pub ceiling: Option<usize>,
    pub best_speed: f64,
}

pub struct RouteManager {
    settings: RwLock<ProxySettings>,
    routes: RwLock<Vec<Arc<Route>>>,
    detected: RwLock<Vec<String>>,
    cache: Mutex<HashMap<String, (String, Instant)>>,
    pub hosts: Mutex<HashMap<String, HostStat>>,
    probes: Mutex<HashMap<String, (String, ProbeInfo, Instant)>>,
}

const CACHE_TTL: Duration = Duration::from_secs(30 * 60);

fn env_proxy() -> Option<String> {
    for k in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy", "HTTP_PROXY", "http_proxy"] {
        if let Ok(v) = std::env::var(k) {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

impl RouteManager {
    pub fn new(settings: ProxySettings) -> DlResult<Self> {
        let m = Self {
            settings: RwLock::new(settings),
            routes: RwLock::new(Vec::new()),
            detected: RwLock::new(Vec::new()),
            cache: Mutex::new(HashMap::new()),
            hosts: Mutex::new(HashMap::new()),
            probes: Mutex::new(HashMap::new()),
        };
        m.rebuild()?;
        Ok(m)
    }

    pub fn update_settings(&self, s: ProxySettings) -> DlResult<()> {
        *self.settings.write() = s;
        self.cache.lock().clear();
        self.rebuild()
    }

    pub fn set_detected(&self, list: Vec<String>) -> DlResult<()> {
        *self.detected.write() = list;
        self.rebuild()
    }

    pub fn detected(&self) -> Vec<String> {
        self.detected.read().clone()
    }

    fn rebuild(&self) -> DlResult<()> {
        let s = self.settings.read().clone();
        let mut out = vec![Arc::new(Route { name: DIRECT.into(), proxy: None, client: build_client(None, false)? })];
        let mut seen: Vec<String> = Vec::new();
        let mut push = |name: String, p: Option<String>, system: bool, out: &mut Vec<Arc<Route>>| {
            let key = p.clone().unwrap_or_else(|| name.clone());
            if seen.contains(&key) {
                return;
            }
            match build_client(p.as_deref(), system) {
                Ok(client) => {
                    seen.push(key);
                    out.push(Arc::new(Route { name, proxy: p, client }));
                }
                Err(e) => tracing::warn!("skip proxy: {e}"),
            }
        };
        for p in s.proxies.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
            let p = if p.contains("://") { p.to_string() } else { format!("http://{p}") };
            push(format!("代理 {}", display_proxy(&p)), Some(p), false, &mut out);
        }
        if s.auto_detect {
            for p in self.detected.read().iter() {
                push(format!("本机代理 {}", display_proxy(p)), Some(p.clone()), false, &mut out);
            }
        }
        if s.use_system {
            if let Some(p) = env_proxy() {
                push(format!("环境变量代理 {}", display_proxy(&p)), Some(p), false, &mut out);
            }
            if cfg!(any(windows, target_os = "macos")) {
                push("系统代理".into(), None, true, &mut out);
            }
        }
        *self.routes.write() = out;
        Ok(())
    }

    pub fn all_routes(&self) -> Vec<Arc<Route>> {
        self.routes.read().clone()
    }

    pub fn ordered(&self, exclude: Option<&str>) -> Vec<Arc<Route>> {
        let mode = self.settings.read().mode;
        let all = self.routes.read().clone();
        let (direct, proxies): (Vec<_>, Vec<_>) = all.into_iter().partition(|r| r.name == DIRECT);
        let mut v = match mode {
            ProxyMode::DirectThenProxy => direct.into_iter().chain(proxies).collect::<Vec<_>>(),
            ProxyMode::DirectOnly => direct,
            ProxyMode::ProxyThenDirect => proxies.into_iter().chain(direct).collect(),
            ProxyMode::ProxyOnly => proxies,
        };
        if let Some(x) = exclude {
            if v.len() > 1 {
                v.retain(|r| r.name != x);
            }
        }
        v
    }

    pub fn by_name(&self, name: &str) -> Option<Arc<Route>> {
        self.routes.read().iter().find(|r| r.name == name).cloned()
    }

    pub fn cached_route(&self, host: &str) -> Option<Arc<Route>> {
        let name = {
            let c = self.cache.lock();
            let (n, t) = c.get(host)?;
            if t.elapsed() > CACHE_TTL {
                return None;
            }
            n.clone()
        };
        self.by_name(&name)
    }

    pub fn remember(&self, host: &str, route: &str) {
        self.cache.lock().insert(host.to_string(), (route.to_string(), Instant::now()));
    }

    /// Keep a fresh probe so the download that follows skips a second round trip.
    pub fn stash(&self, url: &str, route: &str, info: &ProbeInfo) {
        let mut p = self.probes.lock();
        p.retain(|_, v| v.2.elapsed() < Duration::from_secs(30));
        p.insert(url.to_string(), (route.to_string(), info.clone(), Instant::now()));
    }

    pub fn forget(&self, host: &str) {
        self.cache.lock().remove(host);
    }

    /// Find a working route for `url` and probe it. Routes race with staggered starts; the
    /// first success wins, a fast failure immediately releases the next candidate.
    pub async fn resolve(&self, spec: &RequestSpec, url: &str, ua: &str, exclude: Option<&str>) -> DlResult<(Arc<Route>, ProbeInfo)> {
        let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(|s| s.to_string())).unwrap_or_default();
        let timeout = Duration::from_secs(20);
        if exclude.is_none() {
            let hit = self.probes.lock().remove(url).filter(|v| v.2.elapsed() < Duration::from_secs(20));
            if let Some((name, info, _)) = hit {
                let r = self.routes.read().iter().find(|r| r.name == name).cloned();
                if let Some(r) = r {
                    return Ok((r, info));
                }
            }
            if let Some(r) = self.cached_route(&host) {
                match probe(&r.client, spec, url, ua, timeout).await {
                    Ok(mut info) => {
                        info.route = r.name.clone();
                        return Ok((r, info));
                    }
                    Err(e) if !e.kind.route_related() && e.kind != ErrorKind::Auth => return Err(e),
                    Err(_) => self.forget(&host),
                }
            }
        }
        let routes = self.ordered(exclude);
        if routes.is_empty() {
            return Err(DlError::new(ErrorKind::Network, "没有可用线路（仅代理模式但未配置或探测到代理）"));
        }
        let hedge = Duration::from_millis(self.settings.read().hedge_delay_ms.max(100));
        let mut pending = FuturesUnordered::new();
        let mut next = 0usize;
        let mut errors: Vec<(String, DlError)> = Vec::new();
        let launch = |r: Arc<Route>| {
            let spec = spec.clone();
            let url = url.to_string();
            let ua = ua.to_string();
            async move {
                let res = probe(&r.client, &spec, &url, &ua, timeout).await;
                (r, res)
            }
        };
        pending.push(launch(routes[0].clone()));
        next += 1;
        let mut deadline = tokio::time::Instant::now() + hedge;
        loop {
            if pending.is_empty() && next >= routes.len() {
                break;
            }
            tokio::select! {
                Some((r, res)) = pending.next(), if !pending.is_empty() => {
                    match res {
                        Ok(mut info) => {
                            info.route = r.name.clone();
                            self.remember(&host, &r.name);
                            return Ok((r, info));
                        }
                        Err(e) => {
                            tracing::debug!("route {} failed: {e}", r.name);
                            errors.push((r.name.clone(), e));
                            if next < routes.len() {
                                pending.push(launch(routes[next].clone()));
                                next += 1;
                                deadline = tokio::time::Instant::now() + Duration::from_millis(400);
                            }
                        }
                    }
                }
                _ = tokio::time::sleep_until(deadline), if next < routes.len() => {
                    pending.push(launch(routes[next].clone()));
                    next += 1;
                    deadline = tokio::time::Instant::now() + Duration::from_millis(400);
                }
            }
        }
        // Prefer an answer from a server (HTTP status) over a transport failure.
        errors.sort_by_key(|(_, e)| if e.status.is_some() { 0 } else { 1 });
        let routes = errors
            .iter()
            .map(|(n, e)| crate::error::RouteFailure { route: n.clone(), status: e.status, message: e.message.clone() })
            .collect();
        let mut e = errors.into_iter().next().map(|(_, e)| e).unwrap_or_else(|| DlError::new(ErrorKind::Network, "没有可用线路"));
        e.routes = routes;
        Err(e)
    }
}

/// Scan well-known local proxy ports and keep only the ones that actually forward traffic.
pub async fn detect_local_proxies() -> Vec<String> {
    let mut open = Vec::new();
    let mut scans = FuturesUnordered::new();
    for &port in LOCAL_PROXY_PORTS {
        scans.push(async move {
            let ok = tokio::time::timeout(Duration::from_millis(250), tokio::net::TcpStream::connect(("127.0.0.1", port)))
                .await
                .map(|r| r.is_ok())
                .unwrap_or(false);
            (port, ok)
        });
    }
    while let Some((port, ok)) = scans.next().await {
        if ok {
            open.push(port);
        }
    }
    open.sort_by_key(|p| LOCAL_PROXY_PORTS.iter().position(|x| x == p));
    let mut checks = FuturesUnordered::new();
    for (i, port) in open.into_iter().enumerate() {
        checks.push(async move {
            for scheme in ["http", "socks5h"] {
                let p = format!("{scheme}://127.0.0.1:{port}");
                if verify_proxy(&p).await {
                    return Some((i, p));
                }
            }
            None
        });
    }
    let mut found = Vec::new();
    while let Some(r) = checks.next().await {
        if let Some(x) = r {
            found.push(x);
        }
    }
    found.sort();
    found.into_iter().map(|(_, p)| p).collect()
}

pub async fn verify_proxy(p: &str) -> bool {
    let Ok(client) = build_client(Some(p), false) else { return false };
    for u in ["http://www.gstatic.com/generate_204", "http://cp.cloudflare.com/generate_204", "http://connectivitycheck.platform.hicloud.com/generate_204"] {
        if let Ok(Ok(r)) = tokio::time::timeout(Duration::from_secs(5), client.get(u).send()).await {
            if r.status().is_success() || r.status().as_u16() == 204 {
                return true;
            }
        }
    }
    false
}
