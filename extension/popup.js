const toggle = document.getElementById("toggle");
const stateText = document.getElementById("stateText");
const minSize = document.getElementById("minSize");
const consent = document.getElementById("consent");
const controls = document.getElementById("controls");
const enable = document.getElementById("enable");
const disable = document.getElementById("disable");
const apiTokenInput = document.getElementById("apiToken");
const size = (n) => (n >= 1 << 20 ? `${(n / (1 << 20)).toFixed(n % (1 << 20) ? 1 : 0)} MB` : `${Math.round(n / 1024)} KB`);
const hosts = { permissions: ["cookies", "webRequest"], origins: ["http://*/*", "https://*/*"] };

async function render() {
  const { enabled, consented, minSize: m } = await chrome.storage.local.get({ enabled: false, consented: false, minSize: null });
  const granted = await chrome.permissions.contains(hosts);
  const active = !!enabled && !!consented && granted;
  consent.hidden = !!consented;
  controls.hidden = !consented;
  toggle.classList.toggle("on", active);
  enable.disabled = !document.getElementById("agree").checked;
  if (typeof m === "number") minSize.textContent = size(m);
}

document.getElementById("agree").addEventListener("change", render);
enable.addEventListener("click", async () => {
  if (!document.getElementById("agree").checked) return;
  const granted = await chrome.permissions.request(hosts);
  if (!granted) return;
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
  const j = await chrome.runtime.sendMessage("ping");
  stateText.textContent = j ? "已连接" : "配对失败，请检查配对码和桌面程序";
});
render();
chrome.runtime.sendMessage("ping").then((j) => {
  stateText.textContent = j ? "已连接" : "未配对或桌面程序未运行";
  if (j && typeof j.takeover_min_size === "number") minSize.textContent = size(j.takeover_min_size);
});
