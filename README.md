[![][image-banner]][releases-link]

GaGaDown 是一个桌面下载器。浏览器里点下载，它会接过来，用多个连接同时下，下完弹窗告诉你。

它会自己挑最快的线路：直连、系统代理、本机代理一起试，哪条先通用哪条。连接数边下边调，慢的连接会被换掉，所以不会卡在 99%。断线了接着下，服务器上的文件变了会自动重下，不会拼出一个坏文件。百度网盘这类有限制的站点也做了专门处理。

<a href="https://github.com/shuakami/gagadown/releases/latest/download/GagaDown-windows-setup.exe"><img src="docs/download-windows.svg" alt="Download for Windows" /></a>
<a href="https://github.com/shuakami/gagadown/releases/latest/download/GagaDown-linux-x86_64.tar.gz"><img src="docs/download-linux.svg" alt="Download for Linux" /></a>

**[下载][releases-link]** **[反馈问题][issues-link]**

[![][github-release-shield]][releases-link]
[![][github-downloads-shield]][releases-link]
[![][github-stars-shield]][stars-link]
[![][github-license-shield]][license-link]

> \[!IMPORTANT]
>
> **Star 一下**，你将第一时间收到 GitHub 上的新版本发布通知哟

## 特性

- **多连接动态分段**：文件切成多段并行下载，先下完的连接会去分担最慢的那一段，直到最后一刻都不闲着
- **多线路竞速**：直连、系统代理、本机常用代理端口错开起跑，谁先通用谁，每个域名记住最快的线路
- **连接数自适应**：边下边测速，涨得动就加连接，被限流就自动退让，下次下同一个站直接用上次最好的设置
- **可靠续传**：只认已经写进磁盘的进度，带 `If-Range` 校验，服务器文件变了自动从头下，不会拼出坏文件
- **不当傻下载**：探测到网页、登录页或错误码就不接管，交还给浏览器，失败原因写清楚
- **浏览器接管**：内置 Chrome / Edge 插件，GaGaDown 没开时浏览器照常下载
- **安静**：最小化到托盘、开机静默启动，下载弹窗在后台也能及时出现（托盘与开机启动目前仅 Windows）
- **清晰**：Windows 上用系统 ClearType 画字，和原生程序一样锐利

## 快速开始

### Windows

1. 从 [Releases][releases-link] 下载 `GagaDown-Setup-x.y.z.exe` 并安装；不想安装就下 `portable` 便携版
2. 打开 GaGaDown，按「浏览器插件」页的步骤把插件装进 Chrome 或 Edge
3. 之后在浏览器里正常点下载就行

### Linux

```bash
tar xzf GagaDown-*-linux-x86_64.tar.gz
cd GagaDown-*-linux-x86_64
./GagaDown
```

需要 Vulkan 或 OpenGL 显卡驱动，X11 和 Wayland 都能用。

### 浏览器插件

打开 `chrome://extensions`（Edge 是 `edge://extensions`），打开开发者模式，点「加载已解压的扩展程序」，选择 GaGaDown 数据目录下的 `extension` 文件夹。插件页里有一键复制路径的按钮。

### 命令行

```
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

推送到 `main` 后，GitHub Actions 会自动把版本号加一、打 tag、编译 Windows 和 Linux 版并发布 Release。

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
