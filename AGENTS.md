# GaGaDown 开发约束

## 构建

- **不要主动运行 `flatpak-builder`**：完整 Flatpak 构建很慢，只在用户明确要求时跑。日常反复调试用 `cargo run` / `cargo build --release -p gagadown-app` 产出的 `./target/release/GagaDown`。
- Linux 显示后端由 `settings.json` 的 `display_backend` 决定（`x11`/`wayland`/`auto`，默认 `x11`，改动后应用自动重启生效）；`GAGADOWN_FORCE_X11=1` / `GAGADOWN_FORCE_WAYLAND=1` 覆盖设置用于调试。Wayland 无法 unmap 已映射窗口，因此 Wayland 路径下 root viewport 由 `eframe::KEEP_ROOT_HIDDEN` 保持不可见（只跑事件循环），主窗口是 `main_viewport()` 的 immediate 子 viewport：关到托盘=停止 show_viewport 销毁窗口，托盘恢复=重建窗口（`App::hidden` 标志驱动，几何存 `main_size`/`main_pos`/`main_maximized`）。X11/Windows 仍走 root viewport 的传统 visible/focus 路径。
- **Dev Mode：`GAGADOWN_DEV=1`**（release/flatpak 构建同样生效）——更新提示药丸走本地 mock 闭环（检查→假新版→点击→模拟进度→安装→重启回循环，不触网不安装）；URL 输入框接受任意文本生成模拟下载任务；UI 追踪默认开启并同时打到 stderr，`gagadown_app`/`gagadown_core` 放开 DEBUG。
- UI 追踪：`GAGADOWN_UI_TRACE=1` 单独开文件日志（不含 mock 行为，适合让用户抓真实现场），写入数据目录 `ui-debug.log`（本地 `~/.local/share/gagadown/`，Flatpak `~/.var/app/io.github.shuakami.GaGaDown/data/gagadown/`），每次启用清空重建。
- 应用内更新器：release 检查走 GitHub API（`update::REPO`）；下载交给下载引擎（多段/竞速/限速/续传，任务以 `source: "updater"` 短暂出现在下载列表，装完自动移除）；安装 Flatpak 走 `flatpak-spawn --host flatpak install`（manifest 的 `--talk-name=org.freedesktop.Flatpak` 权限必需）、Windows 拉起 NSIS 后退出；更新包落 `data_dir/updates/`。
- Flatpak manifest 用 `dir` source 直接构建仓库 checkout（自分发模式，见 release.yml `flatpak` job 产出的 `.flatpak`）；`gagadown-linux.patch` 已随投稿式流程一起删除。应用内更新器在 flatpak 里经 `flatpak-spawn --host flatpak install` 安装 release 资产，manifest 的 `--talk-name=org.freedesktop.Flatpak` 是它必需的权限。
- `packaging/flatpak/cargo-sources.json` 随 `Cargo.lock` 依赖变化重新生成：`python3 flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json`（需要 aiohttp+tomlkit，脚本见 .github/workflows/release.yml 的 flatpak-sources job）

## 已知坑

- `patches/eframe` / `patches/egui-wgpu` 是 fork 的 vendored 依赖，改动会随 Flatpak 补丁一起进包。
- winit(X11) 的 user-event `try_recv` 循环内不能触发新一轮直接绘制（会活锁、X 事件饿死）；隐藏窗口的直接绘制只在 `new_events` 里做。
- Linux vsync（Fifo）呈现在窗口位于不可见工作区时永久阻塞；egui-wgpu 补丁在 Linux 上改用 Mailbox/Immediate 并自行限帧。
