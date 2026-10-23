<div align="center">

  # wocli-music

  **让听歌找回纯粹，把音乐无缝融入你的终端与工作流**
  <br />
  *Blazing-fast, zero-bloat, native music player for your terminal, scripts, and AI workflows.*

  <p align="center">
    <a href="https://github.com/yhan-sun/wocli-music/releases/latest"><img src="https://img.shields.io/github/v/release/yhan-sun/wocli-music?color=7aa874&label=release&logo=github&style=flat-square" alt="Release" /></a>
    <img src="https://img.shields.io/badge/Rust-1.93%2B-e43716?logo=rust&style=flat-square" alt="Rust" />
    <img src="https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-blue?style=flat-square" alt="Platforms" />
    <img src="https://img.shields.io/badge/Size-~3.5MB-success?style=flat-square" alt="Binary Size" />
    <img src="https://img.shields.io/badge/RAM-~20MB-brightgreen?style=flat-square" alt="RAM Footprint" />
    <img src="https://img.shields.io/badge/Auth-No_Login-blueviolet?style=flat-square" alt="No Login" />
    <a href="docs/mcp.md"><img src="https://img.shields.io/badge/AI-MCP_Native-8a2be2?logo=anthropic&style=flat-square" alt="MCP" /></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--3.0-lightgrey?style=flat-square" alt="License" /></a>
  </p>

  <p align="center">
    <a href="#快速上手">快速上手</a> •
    <a href="#为什么做它">为什么做它</a> •
    <a href="#横向对比">横向对比</a> •
    <a href="#核心体验">核心体验</a> •
    <a href="#常用速查">常用速查</a> •
    <a href="README_EN.md">English</a>
  </p>

</div>

---

## 为什么做它？

我们只想安安静静听首歌，但现在的音乐客户端变了：
- 想听首歌，要下几百兆安装包，每次打开还要看开屏广告。
- 搜到喜欢的歌，动辄提示“扫码登录”、“充值 VIP 才能试听完整版”。
- 界面堆满了直播、短视频与促销横幅，后台悄悄吃掉上千兆内存。
- 在工位沉浸写代码或办公，切歌来回切窗口；有人走过时，巨大的专辑封面赫然在目，来不及暂停。

`wocli-music` 为找回听歌自由而生：

- **完全免登录**：不登账号、不扫码、无广告，输入歌名直接放。
- **极小极快**：单文件仅 ~3.5MB，秒开即播，后台常驻仅消耗 ~20MB 内存。
- **硬核老板模式**：一键静音暂停，全屏切换为固定虚构身份的 Rust 工程代码视图。
- **原生桌面伴侣**：关掉终端音乐继续播；Mac 菜单栏、Windows 任务栏静默滚动当前歌词。
- **让 AI 做你的私人 DJ**：原生接入 MCP 协议，在 Claude / Cursor 里一句话智能点歌与搜集歌单。

---

## 横向对比

| 评估维度 | wocli-music | 商业客户端 (网易/QQ/Spotify) | LX Music 桌面版 | go-musicfox | ncmpcpp + MPD |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **是否需要登录** | **完全免登录**（开箱即播） | **强制登录**（重度 VIP 限制） | **免登录**（依赖第三方音源） | **必须登录**（需扫码绑定） | **免登录**（本地曲库） |
| **内存占用 (RAM)** | **~15 – 30 MB** | **300 MB – 1.2 GB+** | **150 – 450 MB** | **40 – 95 MB** | **15 – 40 MB** |
| **安装包体积** | **~3.5 MB**（单二进制） | **150 – 350 MB** | **80 – 120 MB** | **20 – 35 MB** | **5 – 15 MB** (+系统依赖) |
| **启动耗时** | **< 10ms**（极速秒启） | **2 – 6 秒**（开屏广告与鉴权） | **1.5 – 3 秒** | **< 100ms** | **< 50ms** |
| **运行形态** | **TUI + CLI + AI MCP + 任务栏歌词** | 桌面重度 GUI（多媒体/社交） | 桌面 GUI 窗口 | 终端 TUI | 终端 TUI (需预启守护进程) |
| **上班防查老板模式** | **支持**（一键全屏代码编译伪装） | **不支持**（仅常规隐藏/最小化） | **不支持** | **不支持** | **不支持** |
| **AI 智能体 (MCP)** | **内置原生 stdio MCP** | **不支持** | **仅本地 HTTP API** | **不支持** | **不支持** |
| **运行依赖** | **零外部依赖**（纯 Rust 原生） | 捆绑 Chromium / Electron | 捆绑 Electron / Node | 依赖系统音频库 | 需配置本地 MPD 服务 |

---

## 快速上手

```bash
# macOS / Linux
curl -fsSL https://wocli.yhan.fun/install.sh | bash

# Windows (PowerShell)
irm https://wocli.yhan.fun/install.ps1 | iex
```

### 30 秒开听

```bash
wocli                       # 打开交互式终端播放器
wocli play '周杰伦 晴天'      # 命令行直接搜索并播放歌曲
wocli toggle                  # 切换播放 / 暂停
wocli mcp prompt             # 输出 AI 客户端接入提示词（直接复制给 Claude / Cursor）
```

> **提示**：终端中按 <kbd>q</kbd> 可退出界面，**音乐与下载在后台静默继续**。运行 `wocli stop` 停止播放，`wocli daemon stop` 彻底关闭后台服务。

---

## 核心体验

### 1. 终端交互与沉浸歌词 (TUI)
- 键盘快捷键（Vim 键位 `j`/`k`/`g`/`G`、`/` 搜索）与鼠标点击、拖拽进度条双向支持。
- 按 <kbd>v</kbd> 展开全屏高亮实时滚动歌词。
- 本地无损音频与在线数百万曲库统一纳管，按 <kbd>d</kbd> 一键后台下载。

### 2. 老板模式（虚构工程工作区）

- TUI 按 `Shift+B`、`Ctrl+B` 或 `F12`：暂停音频，切换到 ATLAS 工作区，显示带行号的 Rust 代码、符号与诊断侧栏、构建测试日志。
- 使用固定虚构身份，**不读取真实用户名、主机名或个人路径**。按 `b`、`B`、`Ctrl+B`、`F12` 或 `Esc` 恢复；仅进入前正在播放或加载的歌曲会续播。
- 全局键需先运行 `wocli tray start`：macOS `Ctrl+Shift+B`；Windows `Ctrl+Shift+B` / `Ctrl+Alt+B`；Linux 可在桌面快捷键设置绑定 `wocli boss`。`wocli tray status` 查看注册结果。
- 隐藏歌词和歌曲信息，保留中性的 wocli 托盘入口。输入框的大写 B 用于输入；tmux 占用 Ctrl+B 时可用 F12。

![老板模式虚构代码视图](docs/screenshots/boss.png)

### 歌单导入与迁移

支持 CSV/TSV/TXT、wocli JSON、LX **单歌单 v2**（`.lxmc`/JSON）、MusicFree 备份、M3U/M3U8/PLS、JSPF。TUI 按 `I` 导入、`E` 导出；可指定新名称，多歌单先选择，匹配后预览并确认。

```bash
wocli playlist import ./收藏.lxmc --name "以前的收藏"
wocli playlist scan ./MusicFree.json
wocli playlist import ./MusicFree.json --list 1 --name "手机收藏"
wocli playlist import ./通勤.csv --name "通勤" --dry-run
wocli playlist export "通勤" --format json --output ./通勤.wocli.json
```

保留原平台 ID，匹配到酷我或本地版本后加入播放列表；未匹配信息不会丢弃。支持离线匹配、重试、手动指定、同名另存/合并/替换和快照恢复。在线分享链接、旧版/全量 LX、Apple XML/XSPF、Spotify 备份暂未支持。参见 [完整使用说明与格式表](docs/playlist-import.md)。

### 3. 原生桌面伴侣
- **macOS**：顶部菜单栏逐句滚动歌词，无缝联动系统 Control Center 与硬件媒体键。
- **Windows**：独家任务栏嵌入式歌词条，带播放/暂停/切歌按键，支持鼠标随意拖拽。
- 运行 `wocli lyrics-bar on` 开启，`wocli lyrics-bar off` 关闭。

### 4. 原生 AI MCP 智能体驱动
通过本地标准 stdio 接入 Claude Desktop、Cursor 或 OpenCode，完全本地离线通信、不开放端口、不泄露隐私：
- 复制 `wocli mcp prompt` 的引导词给 AI，直接自然语言点歌：“放首写代码听的轻音乐”、“把当前播放歌曲加到‘深度工作’歌单”。

---

## 常用速查

### 终端快捷键 (TUI)

| 快捷键 | 说明 | 快捷键 | 说明 |
| :--- | :--- | :--- | :--- |
| <kbd>/</kbd> | 快速搜索歌曲 | <kbd>Space</kbd> | 播放 / 暂停 |
| <kbd>j</kbd> / <kbd>k</kbd> | 列表上下选择 | <kbd>Enter</kbd> | 播放选中歌曲 |
| <kbd>[</kbd> / <kbd>]</kbd> | 上一首 / 下一首 | <kbd>←</kbd> / <kbd>→</kbd> | 后退 / 快进 5 秒 |
| <kbd>v</kbd> | 沉浸式歌词视图 | <kbd>Shift</kbd>+<kbd>B</kbd> | 老板模式（伪装测试终端） |
| <kbd>I</kbd> / <kbd>E</kbd> | 导入 / 导出歌单 | <kbd>F12</kbd> | 老板模式备用键 |
| <kbd>f</kbd> | 收藏到我喜欢 | <kbd>d</kbd> | 一键后台下载 |
| <kbd>m</kbd> | 切换循环模式 | <kbd>q</kbd> | 退出界面（后台不断流） |

### 命令行常用指令 (CLI)

```bash
wocli play '歌名'              # 搜索起播
wocli play ~/Music/song.flac   # 播放本地文件
wocli next / wocli prev        # 切歌
wocli volume 60                # 调整音量 (0-100)
wocli status --json            # 获取结构化播放状态
wocli boss                     # 触发老板模式切换
wocli playlist list            # 浏览歌单
wocli stop                     # 停止播放
```

---

## 文档与支持

- [播放器详细技术对比](docs/comparison.md)
- [AI MCP 协议接入指南](docs/mcp.md)
- [歌单导入导出、格式与恢复](docs/playlist-import.md)
- 更多指令与配置帮助：运行 `wocli help` 或 `wocli doctor` 进行环境诊断

---

## 开源协议

本项目基于 [GPL-3.0 License](LICENSE) 开源。
