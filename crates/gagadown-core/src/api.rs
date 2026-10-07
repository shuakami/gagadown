//! Local HTTP API used by the browser extension. Bound to 127.0.0.1 only; requests that
//! carry a web-page Origin are refused so random sites can't push downloads.
use crate::engine::{Engine, PopupEvent};
use crate::probe::filename_from_url;
use crate::error::ErrorKind;
use crate::request::RequestSpec;
use crate::task::{AddRequest, Status};
use axum::extract::{Query, Request, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tower_http::cors::{AllowOrigin, CorsLayer};
use uuid::Uuid;

const PROBE_BUDGET: Duration = Duration::from_secs(8);

fn origin_ok(o: &str) -> bool {
    o.starts_with("chrome-extension://") || o.starts_with("moz-extension://") || o.starts_with("safari-web-extension://") || o == "null"
}

async fn guard(req: Request, next: Next) -> Response {
    if let Some(o) = req.headers().get("origin").and_then(|v| v.to_str().ok()) {
        if !origin_ok(o) {
            return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
        }
    }
    next.run(req).await
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BrowserAdd {
    #[serde(flatten)]
    req: AddRequest,
    /// Skip the size threshold (context-menu "download with GagaDown").
    force: bool,
    mime: Option<String>,
}

#[derive(Serialize)]
struct AddResp {
    accepted: bool,
    reason: Option<String>,
    id: Option<String>,
    filename: Option<String>,
    size: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PingQuery {
    browser: Option<String>,
}

async fn ping(State(e): State<Engine>, Query(q): Query<PingQuery>) -> impl IntoResponse {
    if let Some(b) = q.browser.filter(|b| matches!(b.as_str(), "chrome" | "edge")) {
        e.browser_seen(&b);
    }
    let s = e.settings();
    Json(json!({
        "app": "gagadown",
        "version": env!("CARGO_PKG_VERSION"),
        "takeover_min_size": s.takeover_min_size,
    }))
}

async fn add(State(e): State<Engine>, Json(body): Json<BrowserAdd>) -> impl IntoResponse {
    let html_mime = body.mime.as_deref().map(|m| m.starts_with("text/html")).unwrap_or(false);
    // Show the popup right away; the probe below decides whether it becomes a task.
    let key = (!html_mime).then(|| {
        let key = Uuid::new_v4();
        let url = &body.req.url;
        let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        let filename = body.req.filename.clone().filter(|s| !s.trim().is_empty()).or_else(|| filename_from_url(url)).unwrap_or_else(|| "下载".into());
        e.push_popup(PopupEvent::Pending { key, filename, size: body.req.size_hint, host });
        key
    });
    let (resp, id, fail) = take(&e, body, html_mime).await;
    if let Some(key) = key {
        e.push_popup(match (id, fail) {
            (Some(id), _) => PopupEvent::Bound { key, id },
            (None, Some(reason)) => PopupEvent::Failed { key, reason },
            (None, None) => PopupEvent::Dropped { key },
        });
    }
    Json(resp)
}

/// Baidu PCS answers 403 to browser user agents unless it is the netdisk client.
const NETDISK_UA: &str = "netdisk;P2SP;3.0.20.63";

fn baidu_pcs(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.ends_with("baidupcs.com") || h.ends_with("pcs.baidu.com")))
        .unwrap_or(false)
}

type Taken = (AddResp, Option<Uuid>, Option<String>);

/// Expected hand-back to the browser (small file, web page, …): no error shown.
fn reject(reason: impl Into<String>, size: Option<u64>) -> Taken {
    (AddResp { accepted: false, reason: Some(reason.into()), id: None, filename: None, size }, None, None)
}

/// We wanted the download but could not take it; `shown` is displayed to the user.
fn fail(reason: impl Into<String>, shown: impl Into<String>, size: Option<u64>) -> Taken {
    let (r, id, _) = reject(reason, size);
    (r, id, Some(shown.into()))
}

async fn take(e: &Engine, body: BrowserAdd, html_mime: bool) -> Taken {
    let mut req = body.req;
    req.source.get_or_insert_with(|| "browser".into());
    req.allow_refresh = true;
    let spec = RequestSpec {
        url: req.url.clone(),
        mirrors: req.mirrors.clone(),
        headers: req.headers.clone(),
        cookies: req.cookies.clone(),
        referrer: req.referrer.clone(),
        user_agent: req.user_agent.clone(),
    };
    // The browser holds its download until we answer, so never keep it waiting long.
    let baidu = baidu_pcs(&spec.url);
    let attempt = async {
        if baidu {
            let nd = RequestSpec { user_agent: Some(NETDISK_UA.into()), ..spec.clone() };
            if let Ok(p) = e.probe(&nd).await {
                return Ok((p, true));
            }
        }
        e.probe(&spec).await.map(|p| (p, false))
    };
    let probe = match tokio::time::timeout(PROBE_BUDGET, attempt).await {
        Ok(Ok((p, netdisk))) => {
            if netdisk {
                req.user_agent = Some(NETDISK_UA.into());
            }
            p
        }
        Ok(Err(err)) => {
            // Let the browser keep the download: it owns the session state we could not replay.
            let (reason, shown) = match err.kind {
                ErrorKind::Auth => ("auth", format!("服务器拒绝访问（{}）", err.message)),
                ErrorKind::NotFound => ("not_found", format!("文件不存在（{}）", err.message)),
                _ => ("probe_failed", err.message.clone()),
            };
            return fail(format!("{reason}: {}", err.message), shown, None);
        }
        Err(_) => return fail("probe_timeout", format!("连接超时，{} 秒内服务器没有响应", PROBE_BUDGET.as_secs()), None),
    };
    let min = e.settings().takeover_min_size;
    if crate::download::is_web_page(&probe) {
        return reject("web_page", probe.size);
    }
    if !body.force {
        if let Some(s) = probe.size {
            if s < min {
                return reject("too_small", Some(s));
            }
        }
        if probe.looks_like_page() || html_mime {
            return reject("html", probe.size);
        }
    }
    match e.add(req, Some(&probe)) {
        Ok(o) => (AddResp { accepted: true, reason: None, id: Some(o.id.to_string()), filename: Some(o.filename), size: probe.size }, Some(o.id), None),
        Err(err) => fail(err.message.clone(), err.message, probe.size),
    }
}

async fn tasks(State(e): State<Engine>) -> impl IntoResponse {
    let v: Vec<_> = e
        .views(0)
        .into_iter()
        .take(30)
        .map(|t| {
            json!({
                "id": t.id.to_string(),
                "filename": t.filename,
                "size": t.size,
                "downloaded": t.downloaded,
                "speed": t.speed,
                "status": format!("{:?}", t.status),
                "connections": t.connections,
                "error": t.error.map(|e| e.kind.label()),
            })
        })
        .collect();
    let active = e.views(0).iter().filter(|t| t.status == Status::Running).count();
    Json(json!({ "tasks": v, "speed": e.total_speed(), "active": active }))
}

pub fn router(engine: Engine) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|o: &HeaderValue, _| o.to_str().map(origin_ok).unwrap_or(false)))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([axum::http::header::CONTENT_TYPE]);
    Router::new()
        .route("/api/ping", get(ping))
        .route("/api/add", post(add))
        .route("/api/tasks", get(tasks))
        .layer(middleware::from_fn(guard))
        .layer(cors)
        .with_state(engine)
}

pub async fn serve(engine: Engine, port: u16) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    axum::serve(listener, router(engine)).await
}
