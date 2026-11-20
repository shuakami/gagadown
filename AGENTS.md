# GaGaDown 开发约束

## 构建

- **不要主动运行 `flatpak-builder`**：完整 Flatpak 构建很慢，只在用户明确要求时跑。日常反复调试用 `cargo run` / `cargo build --release -p gagadown-app` 产出的 `./target/release/GagaDown`。
- Linux 显示后端由 `settings.json` 的 `display_backend` 决定（`auto`/`wayland`/`x11`，默认 auto 跟随会话，改后需重启生效）；`GAGADOWN_FORCE_X11=1` / `GAGADOWN_FORCE_WAYLAND=1` 覆盖设置用于调试。Wayland 无法 unmap 已映射窗口，因此 Wayland 路径下 root viewport 由 `eframe::KEEP_ROOT_HIDDEN` 保持不可见（只跑事件循环），主窗口是 `main_viewport()` 的 immediate 子 viewport：关到托盘=停止 show_viewport 销毁窗口，托盘恢复=重建窗口（`App::hidden` 标志驱动，几何存 `main_size`/`main_pos`/`main_maximized`）。X11/Windows 仍走 root viewport 的传统 visible/focus 路径。
- UI 追踪：`GAGADOWN_UI_TRACE=1` 时日志写入数据目录 `ui-debug.log`（本地 `~/.local/share/gagadown/`，Flatpak `~/.var/app/io.github.shuakami.GaGaDown/data/gagadown/`），每次启用清空重建。
- `packaging/flatpak/gagadown-linux.patch` 是相对 tag `v0.1.7` 的 diff；改动任何构建输入（`Cargo.lock`、`crates/`、`extension/`、`patches/`）后必须重新生成：
  `git diff v0.1.7 HEAD -- Cargo.lock crates/ extension/ patches/ > packaging/flatpak/gagadown-linux.patch`（先提交再生成，确保新文件也进补丁）
- `packaging/flatpak/cargo-sources.json` 随 `Cargo.lock` 依赖变化重新生成：`python3 flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json`（需要 aiohttp+tomlkit，脚本见 .github/workflows/release.yml 的 flatpak-sources job）

## 已知坑

- `patches/eframe` / `patches/egui-wgpu` 是 fork 的 vendored 依赖，改动会随 Flatpak 补丁一起进包。
- winit(X11) 的 user-event `try_recv` 循环内不能触发新一轮直接绘制（会活锁、X 事件饿死）；隐藏窗口的直接绘制只在 `new_events` 里做。
- Linux vsync（Fifo）呈现在窗口位于不可见工作区时永久阻塞；egui-wgpu 补丁在 Linux 上改用 Mailbox/Immediate 并自行限帧。
