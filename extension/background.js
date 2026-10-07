// Hands browser downloads to the local GagaDown app with the exact request context
// (cookies, referrer, UA, auth headers) so authenticated links keep working.
// If GagaDown is not running or refuses the link, the browser download simply continues.

const DEFAULTS = { enabled: true, port: 18765, minSize: 1024 * 1024 };
const recent = new Map();
const MAX_RECENT = 400;
const SKIP = new Set([
  "cookie", "user-agent", "referer", "range", "if-range", "accept-encoding", "content-length",
  "host", "connection", "upgrade-insecure-requests", "if-none-match", "if-modified-since",
]);

const cfg = () => chrome.storage.local.get(DEFAULTS);
const BROWSER = /\bEdg\//.test(navigator.userAgent) ? "edge" : "chrome";

function remember(url, rec) {
  recent.delete(url);
  recent.set(url, rec);
  while (recent.size > MAX_RECENT) recent.delete(recent.keys().next().value);
}

const filter = { urls: ["<all_urls>"], types: ["main_frame", "sub_frame", "xmlhttprequest", "media", "other"] };
const onHeaders = (d) => remember(d.url, { headers: d.requestHeaders || [], method: d.method, at: Date.now() });
try {
  chrome.webRequest.onSendHeaders.addListener(onHeaders, filter, ["requestHeaders", "extraHeaders"]);
} catch {
  chrome.webRequest.onSendHeaders.addListener(onHeaders, filter, ["requestHeaders"]);
}
chrome.webRequest.onBeforeRedirect.addListener((d) => {
  const r = recent.get(d.url);
  if (r && d.redirectUrl) remember(d.redirectUrl, r);
}, filter);

async function cookiesFor(url) {
  try {
    const list = await chrome.cookies.getAll({ url });
    return list.length ? list.map((c) => `${c.name}=${c.value}`).join("; ") : null;
  } catch {
    return null;
  }
}

const baseName = (p) => (p ? p.split(/[\\/]/).pop() : "") || null;

async function api(path, body) {
  const { port } = await cfg();
  const r = await fetch(`http://127.0.0.1:${port}${path}`, {
    method: body ? "POST" : "GET",
    headers: body ? { "content-type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(body ? 15000 : 1500),
  });
  if (!r.ok) throw new Error(String(r.status));
  return r.json();
}

async function ping() {
  try {
    const j = await api(`/api/ping?browser=${BROWSER}`);
    if (j && typeof j.takeover_min_size === "number") await chrome.storage.local.set({ minSize: j.takeover_min_size });
    return j;
  } catch {
    return null;
  }
}

async function handoff({ url, originalUrl, referrer, filename, size, mime, force }) {
  const rec = recent.get(url) || (originalUrl && recent.get(originalUrl));
  if (rec && rec.method && rec.method !== "GET") return false;
  let cookie = null;
  let ua = navigator.userAgent;
  const headers = [];
  for (const h of rec?.headers || []) {
    const n = h.name.toLowerCase();
    if (n === "cookie") cookie = h.value;
    else if (n === "user-agent") ua = h.value;
    else if (n === "referer") referrer = referrer || h.value;
    else if (!SKIP.has(n) && h.value !== undefined) headers.push([h.name, h.value]);
  }
  if (!cookie) cookie = await cookiesFor(url);
  try {
    const j = await api("/api/add", {
      url,
      filename: baseName(filename),
      referrer: referrer || null,
      cookies: cookie,
      headers,
      user_agent: ua,
      size_hint: size > 0 ? size : null,
      mime: mime || null,
      source: "browser",
      allow_refresh: true,
      force: !!force,
    });
    return !!j.accepted;
  } catch {
    return false;
  }
}

// Decide inside onDeterminingFilename: Chrome only shows a download once its target is
// known, so a hidden download UI plus cancel+erase here means nothing ever flashes.
let hidden = 0;
const ui = (on) => {
  try {
    chrome.downloads.setUiOptions({ enabled: on }).catch(() => {});
  } catch {}
};
const hideUi = () => {
  if (hidden++ === 0) ui(false);
};
const showUi = () =>
  setTimeout(() => {
    if (--hidden === 0) ui(true);
  }, 400);

chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  const url = item.finalUrl || item.url;
  if (!/^https?:/i.test(url) || item.byExtensionId === chrome.runtime.id) return;
  hideUi();
  (async () => {
    let taken = false;
    try {
      const c = await cfg();
      const size = item.totalBytes > 0 ? item.totalBytes : item.fileSize > 0 ? item.fileSize : 0;
      if (c.enabled && !(size && size < c.minSize)) {
        taken = await handoff({ url, originalUrl: item.url, referrer: item.referrer, filename: item.filename, size, mime: item.mime });
      }
    } catch {}
    if (taken) {
      try { await chrome.downloads.cancel(item.id); } catch {}
      try { await chrome.downloads.erase({ id: item.id }); } catch {}
    }
    try { suggest(); } catch {}
    showUi();
  })();
  return true;
});

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: "gagadown", title: "使用 GaGaDown 下载", contexts: ["link", "video", "audio", "image"] });
  ping();
});
chrome.runtime.onStartup.addListener(ping);
// Lets GaGaDown show the extension as connected.
chrome.alarms.create("ping", { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((a) => a.name === "ping" && ping());
ping();

chrome.contextMenus.onClicked.addListener(async (info) => {
  const url = info.linkUrl || info.srcUrl;
  if (!url || !/^https?:/i.test(url)) return;
  const ok = await handoff({ url, referrer: info.pageUrl, force: true });
  if (!ok) {
    chrome.action.setBadgeBackgroundColor({ color: "#e5484d" });
    chrome.action.setBadgeText({ text: "!" });
    setTimeout(() => chrome.action.setBadgeText({ text: "" }), 4000);
  }
});

chrome.runtime.onMessage.addListener((msg, _sender, reply) => {
  if (msg === "ping") {
    ping().then(reply);
    return true;
  }
});
