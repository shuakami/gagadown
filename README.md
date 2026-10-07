[![][image-banner]][releases-link]

GaGaDown 是一个专为速度设计、UI 美观的桌面下载器，原生支持 Windows 与 Linux。

它通过专门的动态分段与多线程算法接管浏览器下载任务。自动测速，自动换线，自动接管慢速任务。

<a href="https://github.com/shuakami/gagadown/releases/latest/download/GagaDown-windows-setup.exe"><img src="docs/download-windows.svg" alt="Download for Windows" /></a>
<a href="https://github.com/shuakami/gagadown/releases/latest/download/GagaDown-linux-x86_64.tar.gz"><img src="docs/download-linux.svg" alt="Download for Linux" /></a>

**[下载][releases-link]** **[反馈问题][issues-link]**

[![][github-release-shield]][releases-link]
[![][github-downloads-shield]][releases-link]
[![][github-stars-shield]][stars-link]
[![][github-license-shield]][license-link]

## 特性

- **专研分段与多线程算法**：将文件动态切分，多线程并发下载。先下完的线程会自动接管最慢的分段，解决最后 1% 卡死的问题。
- **跨平台**：提供 Windows 与 Linux（X11 / Wayland）原生支持。
- **智能线路竞速**：直连、系统代理、本地代理并发测速。哪条线路快就用哪条，并自动记忆域名的最佳策略。
- **连接数自适应**：边下边测速。速度能涨就增加并发，遇到限流自动退让，下次下载直接复用最佳配置。
- **可靠续传**：基于实际落盘进度与 `If-Range` 校验。服务端文件若发生变化会自动重新下载，确保文件完整。
- **精准接管**：探测到网页、登录鉴权或错误码时，自动将任务交还给浏览器，不盲目下载。
- **浏览器扩展**：内置 Chrome / Edge 插件。GaGaDown 未启动时，浏览器原生下载照常工作。
- **安静运行**：支持最小化到托盘与开机静默启动，后台下载完成时即时推送通知（后台特性目前仅限 Windows）。
- **界面锐利**：Windows 端接入系统级 ClearType 字体渲染，文字显示清晰。

## 快速开始

### Windows

1. 从 [Releases][releases-link] 下载 `GagaDown-Setup-x.y.z.exe` 并安装。也提供免安装的 `portable` 便携版。
2. 打开 GaGaDown，在「浏览器插件」页按步骤将插件安装至 Chrome 或 Edge。
3. 在浏览器中正常点击下载即可。

### Linux

```bash
tar xzf GagaDown-*-linux-x86_64.tar.gz
cd GagaDown-*-linux-x86_64
./GagaDown
```

需要 Vulkan 或 OpenGL 显卡驱动，X11 和 Wayland 均可运行。

### 浏览器插件

在浏览器地址栏打开 `chrome://extensions`（Edge 为 `edge://extensions`），开启「开发者模式」。点击「加载已解压的扩展程序」，选择 GaGaDown 数据目录下的 `extension` 文件夹。可以在 GaGaDown 的插件页一键复制该路径。

### 命令行

```bash
gagadown-cli get <URL> [--dir DIR] [--max-connections N] [--proxy URL] [--direct-only] [--sha256 HEX]
gagadown-cli serve
gagadown-cli proxies
```

## 从源码构建

```bash
# Linux
cargo build --release -p gagadown-app -p gagadown-cli

# Windows（在 Linux 上交叉编译，需要 mingw-w64、nsis、zip）
bash tools/package.sh
```

| 目录 | 内容 |
| --- | --- |
| `crates/gagadown-core` | 下载引擎、线路选择、任务管理、本地 API |
| `crates/gagadown-app` | 桌面程序（egui + wgpu） |
| `crates/gagadown-cli` | 命令行 |
| `extension` | 浏览器插件（Chrome / Edge，MV3） |
| `patches` | eframe / epaint / egui-wgpu 的本地补丁 |

推送到 `main` 后，GitHub Actions 会自动更新版本号、打 tag，并编译发布 Windows 与 Linux 的 Release。

## 许可证

[GPL-3.0][license-link]

<!-- LINK GROUP -->

[image-banner]: docs/images/banner.png
[releases-link]: https://github.com/shuakami/gagadown/releases
[issues-link]: https://github.com/shuakami/gagadown/issues
[stars-link]: https://github.com/shuakami/gagadown/stargazers
[license-link]: LICENSE
[github-release-shield]: https://img.shields.io/github/v/release/shuakami/gagadown?color=317cfe&labelColor=black&logo=github&style=flat-square
[github-downloads-shield]: https://img.shields.io/github/downloads/shuakami/gagadown/total?color=317cfe&labelColor=black&style=flat-square
[github-stars-shield]: https://img.shields.io/github/stars/shuakami/gagadown?color=317cfe&labelColor=black&style=flat-square
[github-license-shield]: https://img.shields.io/badge/license-GPL--3.0-317cfe?labelColor=black&style=flat-square
