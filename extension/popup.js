const toggle = document.getElementById("toggle");
const stateText = document.getElementById("stateText");
const consent = document.getElementById("consent");
const controls = document.getElementById("controls");
const enable = document.getElementById("enable");
const disable = document.getElementById("disable");
const apiTokenInput = document.getElementById("apiToken");
const pairStep = document.getElementById("pairStep");
const pairIntro = document.getElementById("pairIntro");
const reconnectNotice = document.getElementById("reconnectNotice");
const pairEditor = document.getElementById("pairEditor");
const editPairing = document.getElementById("editPairing");
const permissionStep = document.getElementById("permissionStep");
const authorize = document.getElementById("authorize");
const hosts = { permissions: ["cookies", "webRequest"], origins: ["http://*/*", "https://*/*"] };
let pairingCheckId = 0;
let renderId = 0;
let appConnected = false;
let pairingCheckComplete = false;
let editingPairing = false;

async function refreshPairingStatus(failureText) {
  const checkId = ++pairingCheckId;
  stateText.textContent = "正在检测桌面程序…";
  try {
    const j = await chrome.runtime.sendMessage("ping");
    if (checkId !== pairingCheckId) return;
    appConnected = j?.app === "gagadown";
    pairingCheckComplete = true;
    stateText.textContent = appConnected ? "已连接" : failureText;
    if (appConnected) await chrome.storage.local.set({ pairingVerified: true });
    render();
  } catch {
    if (checkId === pairingCheckId) {
      appConnected = false;
      pairingCheckComplete = true;
      stateText.textContent = failureText;
      render();
    }
  }
}

async function render() {
  const currentRenderId = ++renderId;
  const { enabled, consented, pairingVerified, apiToken } = await chrome.storage.local.get({ enabled: false, consented: false, pairingVerified: false, apiToken: "" });
  const granted = await chrome.permissions.contains(hosts);
  if (currentRenderId !== renderId) return;
  const active = !!enabled && !!consented && granted;
  const hasPairingCode = !!apiToken;
  const hasVerifiedPairing = !!pairingVerified && !!apiToken;
  const disconnected = hasVerifiedPairing && pairingCheckComplete && !appConnected;
  const needsPairing = !hasPairingCode || (pairingCheckComplete && !appConnected && !hasVerifiedPairing);
  pairStep.hidden = !disconnected && !needsPairing;
  pairIntro.hidden = disconnected;
  reconnectNotice.hidden = !disconnected;
  pairEditor.hidden = (disconnected && !editingPairing) || (!needsPairing && !disconnected);
  editPairing.hidden = !disconnected || editingPairing;
  permissionStep.hidden = !hasPairingCode || granted || !!consented;
  consent.hidden = !hasPairingCode || !pairingCheckComplete || !appConnected || !granted || !!consented;
  controls.hidden = !consented || !appConnected || !granted;
  toggle.classList.toggle("on", active);
  enable.disabled = !document.getElementById("agree").checked;
}

document.getElementById("agree").addEventListener("change", render);
editPairing.addEventListener("click", () => {
  editingPairing = true;
  pairEditor.hidden = false;
  editPairing.hidden = true;
  apiTokenInput.focus();
});
authorize.addEventListener("click", async () => {
  const granted = await chrome.permissions.request(hosts);
  if (!granted) {
    stateText.textContent = "未获得浏览器权限，接管功能尚未启用";
    render();
    return;
  }
  const { apiToken } = await chrome.storage.local.get({ apiToken: "" });
  if (apiToken) {
    await refreshPairingStatus("配对失败，请检查桌面程序或修改配对码");
  } else {
    stateText.textContent = "权限已授权，请输入桌面端配对码";
    render();
  }
});
enable.addEventListener("click", async () => {
  if (!document.getElementById("agree").checked) return;
  if (!await chrome.permissions.contains(hosts) || !appConnected) return;
  await chrome.storage.local.set({ consented: true, enabled: true });
  render();
});
document.getElementById("toggleRow").addEventListener("click", async () => {
  const { enabled, consented } = await chrome.storage.local.get({ enabled: false, consented: false });
  if (enabled) {
    await chrome.storage.local.set({ enabled: false, consented: false });
    await chrome.permissions.remove(hosts);
  } else if (consented) {
    if (await chrome.permissions.request(hosts)) await chrome.storage.local.set({ enabled: true });
  }
  render();
});
disable.addEventListener("click", async () => {
  await chrome.storage.local.set({ enabled: false, consented: false });
  await chrome.permissions.remove(hosts);
  render();
});
document.getElementById("ver").textContent = `版本 ${chrome.runtime.getManifest().version}`;
chrome.storage.local.get({ apiToken: "" }).then(({ apiToken }) => { apiTokenInput.value = apiToken; });
document.getElementById("saveToken").addEventListener("click", async () => {
  const apiToken = apiTokenInput.value.trim();
  if (!/^[0-9a-f]{64}$/i.test(apiToken)) {
    stateText.textContent = "配对码格式无效";
    return;
  }
  await chrome.storage.local.set({ apiToken });
  await chrome.storage.local.set({ pairingVerified: false });
  editingPairing = false;
  appConnected = false;
  pairingCheckComplete = false;
  const granted = await chrome.permissions.contains(hosts);
  if (granted) {
    await refreshPairingStatus("配对失败，请检查桌面程序或修改配对码");
  } else {
    stateText.textContent = "配对码已保存，请先授权浏览器权限";
    render();
  }
});
async function initialize() {
  const { apiToken } = await chrome.storage.local.get({ apiToken: "" });
  apiTokenInput.value = apiToken;
  const granted = await chrome.permissions.contains(hosts);
  if (apiToken && granted) {
    await refreshPairingStatus("未连接到桌面程序，请检查客户端");
  } else {
    pairingCheckComplete = !apiToken;
    stateText.textContent = apiToken ? "配对码已保存，请先授权浏览器权限" : "尚未配对";
    render();
  }
}
initialize();
