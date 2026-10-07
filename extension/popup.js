const toggle = document.getElementById("toggle");
const stateText = document.getElementById("stateText");
const minSize = document.getElementById("minSize");

const size = (n) => (n >= 1 << 20 ? `${(n / (1 << 20)).toFixed(n % (1 << 20) ? 1 : 0)} MB` : `${Math.round(n / 1024)} KB`);

chrome.storage.local.get({ enabled: true, minSize: null }).then(({ enabled, minSize: m }) => {
  toggle.classList.toggle("on", enabled);
  if (typeof m === "number") minSize.textContent = size(m);
});
document.getElementById("toggleRow").addEventListener("click", async () => {
  const enabled = !toggle.classList.contains("on");
  toggle.classList.toggle("on", enabled);
  await chrome.storage.local.set({ enabled });
});
document.getElementById("ver").textContent = `版本 ${chrome.runtime.getManifest().version}`;

chrome.runtime.sendMessage("ping").then((j) => {
  stateText.textContent = j ? "已连接" : "未运行";
  if (j && typeof j.takeover_min_size === "number") minSize.textContent = size(j.takeover_min_size);
});
