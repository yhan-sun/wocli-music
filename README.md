<div align="center">

  <img src="docs/screenshots/library.png" width="820" alt="wocli-music" style="border-radius: 10px; box-shadow: 0 8px 32px rgba(0,0,0,0.35); max-width: 100%;" />

  # wocli-music

  **让听歌找回纯粹，把音乐无缝融入你的终端与工作流**
  <br />
  *Blazing-fast, zero-bloat, native music player for your terminal, scripts, and AI workflows.*

  <p align="center">
    <a href="https://github.com/yhan-sun/wocli-music/releases/latest"><img src="https://img.shields.io/github/v/release/yhan-sun/wocli-music?color=7aa874&label=release&logo=github&style=flat-square" alt="Release" /></a>
    <img src="https://img.shields.io/badge/Rust-1.93%2B-e43716?logo=rust&style=flat-square" alt="Rust" />
    <img src="https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-blue?style=flat-square" alt="Platforms" />
    <img src="https://img.shields.io/badge/Binary_Size-~3.5MB-success?style=flat-square" alt="Binary Size" />
    <img src="https://img.shields.io/badge/RAM_Footprint-~20MB-brightgreen?style=flat-square" alt="RAM Footprint" />
    <img src="https://img.shields.io/badge/Auth-No_Login_Required-blueviolet?style=flat-square" alt="No Login" />
    <a href="docs/mcp.md"><img src="https://img.shields.io/badge/AI-MCP_Native-8a2be2?logo=anthropic&style=flat-square" alt="MCP" /></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--3.0-lightgrey?style=flat-square" alt="License" /></a>
  </p>

  <p align="center">
    <a href="#快速开始">快速开始</a> •
    <a href="#为什么做这个播放器">为什么做它</a> •
    <a href="#横向体验对比">横向对比</a> •
    <a href="#核心使用体验">核心体验</a> •
    <a href="#快捷键速查">快捷键</a> •
    <a href="#ai-伴侣模式mcp让-ai-控制播放器">AI MCP</a> •
    <a href="#命令行指令">命令行</a> •
    <a href="#高级配置与诊断">配置</a> •
    <a href="README_EN.md">English</a>
  </p>

</div>

---

## 为什么做这个播放器？

我们只想安安静静听首歌，但现在的音乐软件变了。

你大概也经历过这些令人厌烦的瞬间：
- 只是想听首歌，却要下载几百兆的安装包，每次打开还要看几秒钟的开屏广告。
- 刚搜到想听的歌，立刻弹窗阻拦：“请扫码登录”、“充值 VIP 才能听完整版”。
- 软件界面被直播、短视频、动态社区和各种促销横幅塞得满满当当，在后台悄悄吃掉几百兆甚至上千兆内存。
- 在工位、自习室沉浸写代码或办公，切歌还要在不同窗口间手忙脚乱切换；同事或老板走过身后时，巨大的专辑封面赫然显示在屏幕中央，甚至来不及暂停。

听音乐，本该是一件轻快、纯粹、不打扰的事情。

wocli-music 为此而生。它让你找回最初的听歌自由：

- **完全免登录，零骚扰**：不需要手机号，不用扫码，没有会员限制，也不弹广告。输入歌名直接放，想听什么就听什么。
- **小到几乎无感，快如闪电**：整个程序只有 3MB 左右大小，点开瞬间就能发声。常驻后台仅消耗约 20MB 内存，老旧笔记本或云服务器都能毫无负担地全天候静默运行。
- **工作伪装与全局防查（老板模式）**：专为职场人量身打造。遇到突击巡查或同事路过，随手敲下快捷键，音频瞬间静音，整个屏幕秒变真实的编译与单元测试终端；等人走后轻按一下，音乐原位继续播放。
- **桌面原生伴侣，不占屏幕**：在终端选好歌后，随手关掉窗口，音乐照常在后台播放。Windows 任务栏可以直接切歌看歌词，Mac 菜单栏默默滚动当前唱词，无需频繁切换视窗。
- **让 AI 成为你的私人 DJ**：当你在用 Claude 或 Cursor 写代码、做策划时，无需切出窗口，只要给 AI 说一句“放首适合专注工作的轻音乐”，AI 就会自动帮你选歌、播放、调整音量或整理进歌单。

---

## 横向体验对比

无论是在办公室日常办公，还是在服务器终端调试，wocli-music 都能带来截然不同的体验：

| 评估维度 / 播放器方案 | wocli-music | 官方商业客户端<br>*(网易云/QQ/Spotify)* | LX Music 桌面版 | go-musicfox | ncmpcpp + MPD |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **是否需要登录** | **完全免登录**<br>开箱即用，无账号门槛与会员限制 | **强制登录绑定**<br>未登录仅试听，重度依赖 VIP | **免登录**<br>依赖第三方公开音源脚本 | **必须登录**<br>需扫码或 Cookie 绑定网易云 | **免登录**<br>面向本地私有音乐库 |
| **内存占用 (RAM)** | **~15 – 30 MB**<br>Rust 原生极低内存常驻 | **300 MB – 1.2 GB+**<br>多进程 Chromium/Electron 庞大开销 | **150 – 450 MB**<br>基于 Electron + Web 渲染引擎 | **40 – 95 MB**<br>基于 Go 运行时与终端 UI | **15 – 40 MB**<br>基于 C/C++ 守护进程与终端 |
| **安装包 / App 体积** | **~3.0 – 4.2 MB**<br>单文件自包含二进制包 | **150 – 350 MB**<br>庞大富客户端安装包 | **80 – 120 MB**<br>跨平台 Electron 打包文件 | **20 – 35 MB**<br>Go 静态编译产物 | **5 – 15 MB**<br>需额外安装 MPD 服务与依赖库 |
| **启动与响应延迟** | **< 10ms 极速启动**<br>无开屏、无启动广告、无网络阻塞 | **2 – 6 秒**<br>含开屏推荐、商业广告与登录鉴权 | **1.5 – 3 秒**<br>桌面渲染引擎加载耗时 | **< 100ms**<br>终端启动迅速 | **< 50ms**<br>需系统已预启 MPD 守护进程 |
| **核心使用形态** | **TUI + CLI + AI MCP + 托盘歌词**<br>共用一套曲库与后台 | **传统桌面 GUI 窗口**<br>重度集成社交动态、社区与直播 | **桌面 GUI 窗口**<br>专注音乐播放与音源扩展 | **终端 TUI**<br>专注网易云生态终端化 | **终端 TUI (C/S 架构)**<br>专注本地音乐库与标签管理 |
| **AI 智能体控制 (MCP)**| **内置原生 stdio MCP**<br>Claude / Cursor 一句话查听与编排 | **不支持**<br>封闭商业生态 | **有限**<br>仅提供本地 HTTP 开放 API 需自封装 | **不支持** | **不支持**<br>仅支持传统 MPD 网络套接字协议 |
| **上班防查（老板模式）**| **一键仿真 cargo test 编译终端**<br>瞬间静音暂停，托盘指示器全隐蔽 | **不支持**<br>普通隐藏窗口，往往仍在发声 | **不支持**<br>仅常规最小化至系统托盘 | **不支持** | **不支持** |
| **外部运行环境依赖** | **零外部依赖**<br>内置解码、音频栈与 JS 引擎 | 捆绑庞大 Web 浏览器引擎 | 捆绑 Chromium / Node 运行时 | 依赖外部播放引擎与音频库 | 需繁琐配置系统级 MPD 守护进程 |

> **数据来源说明**：数据基于各项目公开版本与实际环境基准测试（核验于 2026-10）。wocli-music 安装包体积取自官方发布的压缩包，内存占用为播放 320kbps 音频时的实测常驻内存集。更多技术选择细节参见 [播放器横向对比报告](docs/comparison.md)。

---

## 快速开始

无需预装 Rust、Python、Node.js、mpv 或 FFmpeg，复制单行命令，10 秒内直接开听：

### 1. 一键安装

#### macOS / Linux (Terminal)
```bash
curl -fsSL https://wocli.yhan.fun/install.sh | bash
```

#### Windows (PowerShell)
```powershell
irm https://wocli.yhan.fun/install.ps1 | iex
```

### 2. 极速上手

安装完成后，打开任何终端窗口即可使用：

```bash
wocli                          # 打开交互式终端播放器（按 / 搜索，Enter 播放，Space 暂停）
wocli play '周杰伦 晴天'         # CLI 直接搜索并起播首选歌曲
wocli status --json             # 获取结构化播放状态（便于脚本编排与状态栏展示）
wocli mcp prompt                # 输出 AI 客户端接入提示词（直接复制给 Claude / Cursor）
```

> **说明**：在终端中按 <kbd>q</kbd> 可安全退出界面，**音乐与后台下载在系统守护进程中静默继续**；运行 `wocli stop` 停止播放，`wocli daemon stop` 彻底关闭后台服务。

---

## 界面展示

<div align="center">
  <table>
    <tr>
      <td width="50%">
        <img src="docs/screenshots/library.png" alt="主曲库与歌词" /><br />
        <p align="center"><b>曲库浏览、播放控制与实时滚动歌词</b></p>
      </td>
      <td width="50%">
        <img src="docs/screenshots/collections.png" alt="歌单与收藏夹管理" /><br />
        <p align="center"><b>分类收藏夹、歌单管理与曲库切换</b></p>
      </td>
    </tr>
    <tr>
      <td colspan="2">
        <img src="docs/screenshots/compact.png" alt="80x26 紧凑小窗口模式" /><br />
        <p align="center"><b>自适应响应式布局：在 80×26 极小窗口甚至 tmux 分屏中自适应排版</b></p>
      </td>
    </tr>
  </table>
</div>

---

## 核心使用体验

### 1. 现代化交互式终端（TUI）
- **键鼠双控**：支持完全使用 Vim 键位（`j` / `k` 上下移动、`g` / `G` 顶底跳转、`/` 搜索），同时支持鼠标直接点击、拖拽进度条跳转与双击切歌。
- **动态歌词**：支持时间轴高亮同屏歌词；按 <kbd>v</kbd> 可一键切换沉浸式全屏歌词视图。
- **本地与在线统一管理**：本地无损文件与在线搜索曲目自由混合收藏，建立专属音乐库。
- **后台异步下载**：选中歌曲按 <kbd>d</kbd> 一键下载并自动保存元数据与同名 `.lrc` 歌词文件，退出 TUI 界面下载任务仍平稳进行。

### 2. 硬核老板模式（Boss Mode / 工作伪装）
专为办公室、工位及自习室设计的防查模式，彻底杜绝意外尴尬：
- **终端工作伪装**：在 TUI 界面中按下 <kbd>Shift</kbd>+<kbd>B</kbd> 或 <kbd>Ctrl</kbd>+<kbd>B</kbd>，播放器**立即静音暂停**，屏幕瞬间全屏切换为逼真的底层工程编译与测试运行日志（模拟真实项目的 `cargo test` 编译日志、18 项核心测试全部 `ok` 通过提示以及带动态光标 `█` 的命令行提示符）。
- **一键无缝恢复**：轻按 <kbd>B</kbd>、<kbd>Ctrl</kbd>+<kbd>B</kbd> 或 <kbd>Esc</kbd>，瞬间恢复播放器界面并**继续无缝播放**原有歌曲。
- **全局桌面隐蔽**：
  - **Windows**：后台原生全局热键 <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>B</kbd> 或 <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>B</kbd>，一键暂停音频并完全隐藏任务栏歌词条与通知区提示。
  - **macOS**：状态栏菜单点击「老板模式 (Shift+B)」，即刻清除菜单栏歌词并清空系统 Control Center / Now Playing 播控状态。
  - **Linux**：系统桌面快捷键可直接绑定命令 `wocli boss`，实现全局一键静音隐蔽。

### 3. 任务栏与菜单栏歌词（Tray & Taskbar Lyrics）
无需时刻盯着终端，音乐信息尽在眼帘：
- **macOS**：原生接入顶部菜单栏，实时滚动当前逐句歌词；深度接入 `MPRemoteCommandCenter`，键盘多媒体键与 AirPods 耳机控制协同工作。
- **Windows**：**独家任务栏嵌入式歌词**，高度精确匹配 Windows 任务栏，左侧自带上一首、播放/暂停、下一首交互按键；文字区支持鼠标按住任意拖动位置。
- 控制指令：
  ```bash
  wocli tray start           # 启动系统托盘常驻服务
  wocli lyrics-bar on        # 开启任务栏 / 状态栏悬浮歌词
  wocli lyrics-bar off       # 关闭任务栏歌词
  wocli lyrics-bar toggle    # 快速切换开启/关闭
  ```

---

## AI 伴侣模式（MCP：让 AI 控制播放器）

wocli-music 原生实现了官方 **Model Context Protocol (MCP)** 标准，通过本地标准输入/输出 (stdio) 为 AI Agent 提供系统级音乐控制能力，**不监听 HTTP 端口、不泄露隐私、完全离线安全**。

```
 ┌──────────────────────┐        stdio (JSON-RPC)       ┌────────────────────────┐
 │  Claude / Cursor /   │ ────────────────────────────> │  wocli mcp serve       │
 │  OpenCode 等 AI 客户端 │ <──────────────────────────── │  (内置 17 个核心工具)    │
 └──────────────────────┘                               └───────────┬────────────┘
                                                                    │ IPC
                                                        ┌───────────▼────────────┐
                                                        │  wocli 后台常驻守护进程  │
                                                        └────────────────────────┘
```

### 1. 快速接入配置

运行以下命令，直接获取适配本机的配置 JSON 或提示词：

```bash
wocli mcp prompt             # 输出给 AI 的一键连接与引导提示词
wocli mcp config             # 输出包含当前绝对路径的客户端配置
```

以 **Claude Desktop** 或 **Cursor** 为例，在配置文件（如 `claude_desktop_config.json`）中添加：

```json
{
  "mcpServers": {
    "wocli-music": {
      "command": "wocli",
      "args": ["mcp", "serve"]
    }
  }
}
```

### 2. 赋予 AI 的核心能力

AI 助手接入后，即可直接使用 17 个工具接管音乐世界：
- **智能点歌**：“帮我放一首周杰伦适合现在写的抒情歌”
- **歌单管理**：“把当前播放的歌曲加入我的‘深度工作’歌单”
- **播放控制**：“暂停”、“快进 30 秒”、“切换随机播放”、“音量调到 40”
- **自动归档**：“检索告五人的最新 3 首歌并下载无损音频到我的本地音乐目录”

> 详见 [MCP 协议与接入完整指南](docs/mcp.md)。

---

## CLI 与自动化指令

所有操作均可通过命令行直接调用，与 TUI 界面完全共享同一套后台：

### 播放与切歌
```bash
wocli play '周杰伦 晴天'          # 搜索并播放第一首匹配曲目
wocli play --id 61631119 --quality best  # 指定 ID 与最高无损音质
wocli play ~/Music/song.flac     # 播放本地无损音频文件
wocli toggle                     # 切换播放 / 暂停
wocli next                       # 下一首（支持别名 wocli skip，添加 --no-wait 异步切歌）
wocli prev                       # 上一首（支持别名 wocli previous）
wocli seek +15                   # 快进 15 秒（支持负数快退）
wocli volume 65                  # 调整音量 (0 - 100)
wocli mode shuffle               # 切换播放方式：list / repeat / single / shuffle
```

### 曲库、收藏与队列
```bash
wocli status --json              # 获取当前播放状态（JSON 结构化）
wocli like                       # 喜欢 / 取消喜欢当前正在播放的歌曲
wocli favorites                  # 查看「我喜欢」歌曲列表
wocli playlist create '沉浸写码'  # 创建专属歌单
wocli playlist add '沉浸写码' --id 61631119 # 收藏歌曲至指定歌单
wocli playlist play '沉浸写码'    # 播放歌单全部歌曲
wocli queue list                 # 查看当前播放队列
```

### 老板键与托盘
```bash
wocli boss                       # 快速切换老板模式（静音暂停并隐藏所有桌面指示器）
wocli boss on                    # 强制开启老板模式
wocli boss off                   # 退出老板模式并恢复播放
wocli tray start                 # 启动系统托盘常驻进程
wocli tray stop                  # 停止系统托盘
```

### 下载与音源
```bash
wocli download '晴天' --pick 1 --quality 320k --lyrics --output ~/Music
wocli source list                # 查看音源解析链路与启停状态
wocli source add ./custom.js     # 导入自定义 LX 音乐源解析脚本
wocli retry                      # 重新尝试当前歌曲及备用音源
```

---

## 快捷键速查

在终端交互界面（TUI）中，键盘多媒体键与以下快捷键原生可用：

| 快捷键 | 功能动作 |
| :--- | :--- |
| <kbd>j</kbd> / <kbd>k</kbd> 或 <kbd>↓</kbd> / <kbd>↑</kbd> | 向下 / 向上选择歌曲或菜单项 |
| <kbd>g</kbd> / <kbd>G</kbd> 或 <kbd>Home</kbd> / <kbd>End</kbd> | 快速直达列表顶部 / 底部 |
| <kbd>[</kbd> / <kbd>]</kbd> 或 硬件媒体上一首/下一首 | 切换上一首 / 下一首 |
| <kbd>Space</kbd> 或 硬件播放/暂停键 | 播放 / 暂停切换（空闲或停止时自动起播） |
| <kbd>Enter</kbd> | 播放选中的歌曲 / 确定操作 |
| <kbd>/</kbd> 或 <kbd>Ctrl</kbd>+<kbd>F</kbd> | 激活快速搜索栏 |
| <kbd>←</kbd> / <kbd>→</kbd> | 后退 / 前进 5 秒 |
| <kbd>+</kbd> / <kbd>-</kbd> 或 硬件音量键 | 调整播放音量 |
| <kbd>Ctrl</kbd>+<kbd>U</kbd> / <kbd>Ctrl</kbd>+<kbd>D</kbd> | 列表向上 / 向下翻页 |
| <kbd>v</kbd> | 展开 / 收起全屏沉浸歌词 |
| <kbd>m</kbd> | 切换播放循环模式（顺序 / 列表循环 / 单曲 / 随机） |
| <kbd>c</kbd> | 收藏选中的歌曲到指定歌单 |
| <kbd>f</kbd> | 加入 / 移出「我喜欢」红心 |
| <kbd>d</kbd> / <kbd>D</kbd> | 下载选中歌曲 / 打开下载任务列表 |
| <kbd>a</kbd> / <kbd>x</kbd> | 追加到播放队列 / 调出歌曲快捷菜单 |
| <kbd>Shift</kbd>+<kbd>B</kbd> / <kbd>Ctrl</kbd>+<kbd>B</kbd> | **老板模式**：静音暂停并全屏伪装为编译测试终端（按 <kbd>B</kbd>/<kbd>Esc</kbd> 恢复） |
| <kbd>Tab</kbd> / <kbd>b</kbd> | 焦点切换 / 弹出曲库选择菜单 |
| <kbd>?</kbd> | 弹出快捷键即时帮助浮窗 |
| <kbd>Esc</kbd> | 关闭当前弹窗 / 取消搜索 / 退出全屏歌词 |
| <kbd>q</kbd> | 退出终端界面（后台守护进程继续播放） |

---

## 高级配置与诊断

<details>
<summary><b>持久化配置 (wocli config)</b></summary>

```bash
wocli config quality best        # 设置默认音质：best / 320k / 192k / 128k
wocli config mode repeat         # 设置启动默认模式：list / repeat / single / shuffle
wocli config volume 70           # 设置启动音量 (0 - 100)
wocli config lyric_offset 150    # 设置歌词时间微调偏移（毫秒，正数提前，负数延后）
wocli config direct true         # 忽略环境代理直接直连网络 (true / false)
```
</details>

<details>
<summary><b>环境变量支持</b></summary>

| 环境变量 | 默认值 | 说明 |
| :--- | :--- | :--- |
| `WOCLI_HOME` | `~/.config/wocli-music` *(Win: `%APPDATA%\wocli-music`)* | 曲库 (`library.sqlite3`)、配置与日志主目录。 |
| `WOCLI_RUNTIME_DIR` | `/tmp/wocli-rust-{uid}-{hash}` *(Win: `%LOCALAPPDATA%\wocli-music\run`)* | 运行时 IPC 通信 Socket 目录。 |
| `WOCLI_DOWNLOAD_DIR` | `~/Music/wocli` *(TUI)* / `downloads` *(CLI)* | 歌曲与歌词下载默认存储目录。 |
| `WOCLI_AUDIO_OUTPUT` | *(默认系统声卡)* | 设为 `null` 时仅解码不发声（用于服务器无声卡测试）。 |
| `HTTP_PROXY` / `HTTPS_PROXY` | *(无)* | HTTP/HTTPS 代理配置，内部网络请求自动继承。 |
</details>

<details>
<summary><b>环境诊断与自检 (wocli doctor)</b></summary>

遇到任何网络或音频设备问题时，运行环境自检：
```bash
wocli doctor
```
输出当前系统的架构、音频引擎、配置有效路径、后台运行状态及活跃网络变量。
</details>

<details>
<summary><b>离线安装与手动下载</b></summary>

无法使用一键安装脚本时，可直接从 [Cloudflare 快速分发入口](https://wocli.yhan.fun/) 或 [GitHub Releases](https://github.com/yhan-sun/wocli-music/releases/latest) 下载解压：
- **Apple Silicon Mac**: `wocli-music-*-macos-arm64.tar.gz`
- **Intel Mac**: `wocli-music-*-macos-x86_64.tar.gz`
- **Linux x86_64**: `wocli-music-*-linux-x86_64.tar.gz` (保留自带 `lib/libasound.so.2`)
- **Windows x86_64**: `wocli-music-*-windows-x86_64.zip`
</details>

<details>
<summary><b>源码构建与开发</b></summary>

需要 Rust **1.93.1+** 与 C 编译器工具链：
```bash
git clone https://github.com/yhan-sun/wocli-music.git
cd wocli-music
cargo run --release            # 编译并运行
cargo test --all-targets       # 运行完整测试套件
cargo clippy --all-targets     # 代码质量检查
```
</details>

---

## 常见问题 (FAQ)

**Q: 退出终端界面后，怎么停止音乐？**  
A: `wocli stop` 停止播放；`wocli daemon stop` 彻底关闭后台服务。

**Q: 为什么搜索到的歌曲有的无法播放？**  
A: 在线搜索索引与上游音频授权不完全一致。若当前音质无资源，程序会自动按音质依序降级并轮询备用源；如所有源均不可用会自动跳过，保证播放列表不卡死。

**Q: Windows 安装后在终端中提示找不到命令？**  
A: Windows 安装脚本会自动写入环境变量。如果此前已有打开的命令提示符或 PowerShell 窗口，请完全退出并重新打开终端应用以继承最新 PATH。

**Q: 可以在远程服务器或 Docker 容器中使用吗？**  
A: 可以。在无物理声卡的 Linux 服务器或 CI 容器中，设定 `WOCLI_AUDIO_OUTPUT=null` 即可无声运行与测试。

---

## 致谢与开源协议

- 终端界面交互深受 [MiaowCham/ncm-cli](https://github.com/MiaowCham/ncm-cli) 启发。
- 底层技术栈依托 [Ratatui](https://github.com/ratatui/ratatui)、[Rodio](https://github.com/RustAudio/rodio)、[Symphonia](https://github.com/pdeljanov/Symphonia)、[QuickJS](https://bellard.org/quickjs/) 与官方 [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)。
- 本项目采用 [GPL-3.0 License](LICENSE) 开源许可。
