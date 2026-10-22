# wocli-music

**把音乐放进你的终端、脚本和 AI 工作流。**

Rust 原生音乐播放器，提供键盘与鼠标操作的 TUI、可编排的 CLI，以及本地 MCP 服务。在线搜索、本地音频、歌词、队列和收藏共用一套曲库；退出界面后，音乐和下载任务继续在后台运行。

[English](README_EN.md) · [下载与安装](https://wocli.yhan.fun/) · [快速开始](#快速开始) · [优势](#为什么选择-wocli-music) · [同类对比](#与其他播放器怎么选) · [MCP](#mcp让-ai-客户端控制播放器) · [命令行](#命令行) · [快捷键](#快捷键) · [常见问题](#常见问题) · [更新日志](CHANGELOG.md)

![wocli-music：收藏夹、同步歌词与播放控制](docs/screenshots/library.png)

## 快速开始

macOS / Linux：

```bash
curl -fsSL https://wocli.yhan.fun/install.sh | bash
```

Windows PowerShell：

```powershell
irm https://wocli.yhan.fun/install.ps1 | iex
```

无需预装 Rust、Python、Node.js、mpv 或 FFmpeg。安装完成后：

```bash
wocli                          # 打开终端播放器；/ 搜索，Enter 播放
wocli search '周杰伦'            # 先查看候选歌曲
wocli play '周杰伦' --pick 1     # 明确选择搜索结果中的第 1 首
wocli status --json             # 从脚本读取当前状态
wocli mcp config                # 生成 AI 客户端接入配置
```

按 `q` 退出界面，音乐继续；`wocli stop` 停止播放，`wocli daemon stop` 结束后台。快捷键可用 `?` 查看，命令说明可用 `wocli thelp` 查看。

## 为什么选择 wocli-music

| 能力 | 对你的实际帮助 |
| --- | --- |
| **TUI + CLI + MCP，共用一套状态** | 在终端选歌、用脚本暂停、让 AI 整理歌单，都操作同一个后台和曲库。内置 17 个 MCP 工具，只读模式开放 8 个查询工具。 |
| **后台播放与下载** | 关闭 TUI 后继续听歌和下载；重新打开界面或在另一个终端发命令即可接着控制。 |
| **原生音频与小体积发行包** | 音频解码、播放及 LX 脚本引擎随程序提供，无需维护外部语言运行时或播放器。v0.3.5 四个平台的压缩安装包约 **3.0–4.2 MB**。 |
| **在线与本地统一管理** | 无需登录即可搜索公开音源；在线歌曲与本地 MP3、FLAC、AAC/M4A、WAV 使用同一套队列、收藏夹和最近播放；支持同步 LRC 歌词。 |
| **失败后继续尝试** | 默认提供酷我原生及移动备用接口，支持自定义源与优先级。解析、传输或解码失败会尝试下一源，单曲全部失败后自动跳过。 |
| **为国内访问提供独立分发入口** | 安装器、版本查询、安装包与源码通过 `wocli.yhan.fun` 分发；v0.3.5 起 `wocli update` 默认使用此入口，仍校验 SHA256。 |

安装包体积来自 [v0.3.5 正式发行文件](https://github.com/yhan-sun/wocli-music/releases/tag/v0.3.5)，以十进制 MB 计，包含该平台随包文件；不等于运行内存。Linux 包附带 ALSA 库，需保留 `lib/` 目录。

### 哪些场景适合

- 写代码时把播放器留在终端或 tmux 窗口，使用 Vim 风格按键和鼠标操作。
- 给桌面快捷键、状态栏或脚本接入 `wocli toggle`、`wocli next` 和 `wocli status --json`。
- 让支持本地 stdio MCP 的 AI 客户端搜索歌曲、控制播放、管理收藏和下载。
- 同时听在线音乐与本地文件，希望歌单和设置保存在自己的电脑上。

## 与其他播放器怎么选

| 方案 | 主要定位与已有能力 | 选择建议 |
| --- | --- | --- |
| **wocli-music** | Rust TUI、独立 CLI、内置本地 MCP；在线与本地播放共享后台；内置备用源与受限 LX 兼容层 | 看重终端操作、脚本集成和 AI 控制，且不依赖平台账号同步时，可以优先考虑。 |
| [Spotify 官方客户端](https://support.spotify.com/us/article/listen-offline/) | 平台曲库与账号服务；Premium 支持下载专辑、歌单等内容离线播放 | 希望围绕平台账号和订阅离线服务使用时可考虑；wocli 的本地收藏不与 Spotify 账号同步。 |
| [go-musicfox](https://github.com/go-musicfox/go-musicfox) | Go 编写的网易云音乐终端客户端，支持 Last.fm、MPRIS、macOS 交互与 DLNA | 希望围绕网易云生态在终端听歌，或看重这些系统集成时，值得考虑。 |
| [LX Music 桌面版](https://github.com/lyswhut/lx-music-desktop) | Electron/Vue 图形界面、自定义音源、数据同步服务，以及本地 HTTP 开放 API | 更习惯桌面界面、需要其音源生态或同步方式时更合适；已有开放 API 也能用于集成。 |
| [YesPlayMusic](https://github.com/qier222/YesPlayMusic) | 网易云图形客户端，提供账号登录、MV、私人 FM、每日推荐等功能 | 更看重封面浏览、视频及网易云账号体验时更合适。 |
| [ncmpcpp](https://github.com/ncmpcpp/ncmpcpp) + [MPD](https://www.musicpd.org/) | 终端客户端与独立音乐服务组合，提供媒体库、标签编辑、可视化及网络协议控制 | 已有 MPD 曲库或服务部署，希望沿用其客户端和协议生态时更合适。 |

对比依据为各项目公开文档，核对于 **2026-10-22**；比较的是定位和可验证能力，不按未做的内存、CPU 或起播测速排名。来源与选择细节见 [完整对比说明](docs/comparison.md)。

### 当前边界

- 在线搜索使用酷我目录；支持的 LX 接口目前为 **`kw/musicUrl`**，并非完整的 LX 协议或全平台聚合搜索。
- 没有平台账号登录、云歌单同步、封面墙、MV、私人 FM 或系统媒体键集成。
- MCP 只提供本机 stdio 服务，音频在运行后台的电脑输出；关闭 MCP 不会停止后台。
- 搜索结果不保证具有可播放的完整音频；歌曲和音质取决于上游。换源成功率、起播速度与 Cloudflare 国内线路速度均受实际网络影响。

## 安装

无需预装 Rust、Python、uv、mpv 或 FFmpeg。

### 一键脚本安装（推荐）

#### macOS / Linux (Terminal)

在终端运行以下命令，脚本会自动识别你的系统（macOS ARM64 / Intel、Linux x86_64）、下载对应安装包、校验 SHA256 完整性并安装到用户命令路径：

```bash
curl -fsSL https://wocli.yhan.fun/install.sh | bash
```

- **自定义安装目录**（例如安装到 `/usr/local/bin`）：
  ```bash
  curl -fsSL https://wocli.yhan.fun/install.sh | bash -s -- --dir /usr/local/bin
  ```
- **指定安装版本**：
  ```bash
  curl -fsSL https://wocli.yhan.fun/install.sh | bash -s -- --version v0.3.5
  ```
- **卸载**：
  ```bash
  curl -fsSL https://wocli.yhan.fun/install.sh | bash -s -- --uninstall
  ```

#### Windows (PowerShell)

在 PowerShell 中执行以下命令，脚本会自动下载 Windows 原生版本、校验 SHA256 完整性，并配置用户及当前 PowerShell 的 PATH，安装后可直接运行 `wocli`：

```powershell
irm https://wocli.yhan.fun/install.ps1 | iex
```

安装完成后，当前 PowerShell 无需重开。其他已经打开的终端需要完全退出并重新启动终端应用，才能继承更新后的 PATH。若旧安装仍提示找不到命令，可先在 PowerShell 执行：

```powershell
$env:Path += ";$env:LOCALAPPDATA\Programs\wocli-music"
wocli --version
```

使用自定义安装目录时，将上面的目录替换为实际目录。

- **自定义安装目录**：
  ```powershell
  & ([scriptblock]::Create((irm https://wocli.yhan.fun/install.ps1))) -InstallDir "C:\Tools\wocli"
  ```
- **卸载**：
  ```powershell
  & ([scriptblock]::Create((irm https://wocli.yhan.fun/install.ps1))) -Uninstall
  ```

### 更新

从 v0.3.1 起支持自更新，**v0.3.5 起默认走 Cloudflare 分发**：

```bash
wocli update --check     # 只检查最新稳定版
wocli update             # 下载、校验 SHA256 并更新当前安装
wocli update --force     # 重新安装当前稳定版
```

`v0.3.1–v0.3.4` 若无法连接 GitHub，请先重新执行上方安装命令升级；曲库与设置会保留。新版本信息最多缓存 10 分钟。

支持 `--json` 和 `--direct`（忽略环境代理，仍使用分发入口）；设 `WOCLI_DOWNLOAD_BASE=github` 可改为直连 GitHub，详见 [分发配置](docs/distribution.md)。更新保留曲库、收藏、队列与设置；正在运行的后台和托盘会停止并重新启动，播放需要重新开始。安装目录需要当前账户有写权限。通过 Linux 安装脚本创建的入口包装脚本会保留，更新实际程序及配套 ALSA 库。

v0.3.0 及更早版本尚无 `update` 命令，首次升级请重新执行上面的安装命令。Windows 安装脚本还会修复当前 PowerShell 的 PATH。

### 手动下载安装

你也可以从 [Cloudflare 下载入口](https://wocli.yhan.fun/) 或 [GitHub Releases](https://github.com/yhan-sun/wocli-music/releases/latest) 下载对应平台的打包文件：

| 设备与架构 | v0.3.5 发行文件 | 解包安装方式 |
| --- | --- | --- |
| Apple Silicon Mac (macOS 15+) | `wocli-music-0.3.5-macos-arm64.tar.gz` | 解压后将 `wocli` 复制到 `~/.local/bin/` |
| Intel Mac (macOS 15+) | `wocli-music-0.3.5-macos-x86_64.tar.gz` | 解压后将 `wocli` 复制到 `~/.local/bin/` |
| Linux x86_64 (glibc 2.35+) | `wocli-music-0.3.5-linux-x86_64.tar.gz` | 解压后保留 `lib/` 在 `wocli` 同级目录下 |
| Windows 10/11 x86_64；ARM64 需支持 x64 仿真 | `wocli-music-0.3.5-windows-x86_64.zip` | 解压后将 `wocli.exe` 放置于任何 PATH 目录 |

以 Apple Silicon Mac 为例：

```bash
tar -xzf wocli-music-0.3.5-macos-arm64.tar.gz
cd wocli-music-0.3.5-macos-arm64
./wocli
```

> **说明**：
> - Linux 版本保留自带的 ALSA 音频库 `lib/libasound.so.2` 位于 `wocli` 旁边；直接运行即可使用系统桌面的声音配置。
> - `*-sources.tar.gz` 是第三方依赖许可要求的源码包，**用户日常使用无需下载**。每个安装包内均包含 `THIRD_PARTY.txt` 与对应许可证。
> - macOS 发行版如提示安全阻止，可对解压文件执行 `xattr -d com.apple.quarantine ./wocli` 移除隔离属性。

### 从旧版升级

先退出旧界面，使用旧版命令彻底停止后台常驻进程，再启动新二进制：

```bash
# 原来从 Python 源码运行：
uv run wocli daemon stop
# 或在旧版二进制目录下：
./wocli daemon stop

# 启动新版本
wocli
```

沿用相同数据目录时，旧版的 `library.sqlite3` 曲库与 `settings.json` 会被继承：自建收藏夹、我喜欢、历史播放、队列、歌词缓存均无需重新导入。改过 `WOCLI_HOME` 的用户在新版本中继续指定相同路径即可。

## 使用方案

### 找歌与听歌

启动 `wocli`，点击顶部「搜索」或按 `/`，输入关键词后 Enter。搜索只占列表顶部一行。方向键选择，Enter 播放；鼠标双击也能播放。播放后可关闭搜索继续浏览曲库。

底部只保留歌曲信息、播放控制和时间轴。Space 暂停或继续；点击时间轴定位，拖动松开后跳转。`m` 切换播放方式，`+` / `-` 调整音量。界面退出用 `q`，音乐继续；停止播放用 `wocli stop`，结束后台用 `wocli daemon stop`。

### 整理收藏夹

选中歌曲后点击 **「收藏到…」** 或按 `c`，从收藏夹列表选择，也可在此直接新建。`f` 加入或移出「我喜欢」。歌曲菜单 `x` 包含收藏、下一首播放、加入队列、下载与从当前列表移除。

侧栏 `+` 或 `n` 新建收藏夹；收藏夹标题旁 `⋯` 可重命名、删除。删除收藏夹只移除列表关系，保留其他收藏与本地文件。`b` 可打开音乐库选择菜单，窄窗口也能切换收藏夹。

![Rust 版：直接选择收藏夹](docs/screenshots/collections.png)

### 看歌词、处理播放失败

宽窗口右侧显示歌词；`v` 切换完整歌词视图，Esc 返回。歌词失败时可以点「重试歌词」，或按 `l` 独立刷新，不需要中断音乐。本地音频读取同名 `.lrc`。

歌曲失败时显示原因，可「重试」或「切换音质」。Ctrl+R 手动重试当前歌曲；播放中的网络中断先重试当前源一次，再尝试其他启用源。所有源失败后自动跳过，整队列失败则停止并保留错误。音频设备故障直接提示。歌词与歌曲分别加载，切歌后会丢弃上一首的过期结果。

在线播放采用渐进式流式播放：解析音源并预缓冲最多 256KB 后起播，其余音频在后台下载。起播速度取决于网络和音频格式。流式网络中断会重试一次，无法播放的歌曲自动跳过，整队列失败后停止并显示错误。下载与播放均校验传输完整性。

### 保存歌曲

选中歌曲后按 `d` 或从歌曲菜单选择「下载」，编辑目录、音质与「保存 LRC」，然后开始下载。顶部「下载」查看任务、进度和错误；失败任务可重试，进行中的任务可取消。下载在后台进行，退出界面后继续，退出后台会取消未完成任务。

默认不覆盖同名文件。传输使用分块、长度检查和上游提供的 MD5；校验失败清理临时文件，保留已有文件。

### 小窗口

![Rust 版：80×26 终端](docs/screenshots/compact.png)

小窗口收起右侧歌词，`v` 单独查看。宽度不足时侧栏隐藏，按 `b` 选择曲库。按 `?` 查看完整快捷键。

## 快捷键

| 按键 | 功能说明 |
| :--- | :--- |
| `j` / `k` 或 `↓` / `↑` | 向下 / 向上选择歌曲或菜单项 |
| `g` / `G` 或 `Home` / `End` | 快速跳至列表顶部 / 底部 |
| `[` / `]` 或 `<` / `>` | 切换上一首 / 下一首 |
| `Space` | 播放 / 暂停切换 |
| `Enter` | 播放选中的歌曲 / 确定 |
| `/` 或 `Ctrl+F` | 聚焦搜索栏 |
| `←` / `→` | 快退 / 快进 5 秒 |
| `+` / `-` | 调整音量 |
| `Ctrl+U` / `Ctrl+D` | 向上 / 向下翻页 |
| `v` | 展开 / 收起全屏歌词 |
| `m` | 切换循环模式（顺序 / 列表循环 / 单曲 / 随机） |
| `c` | 收藏选中的歌曲到指定歌单 |
| `f` | 将歌曲加入 / 移出「我喜欢」 |
| `d` | 下载当前选中的歌曲 |
| `D` | 打开下载任务 |
| `a` | 将歌曲添加到播放队列末尾 |
| `x` | 打开歌曲快捷操作菜单 |
| `Tab` | 侧栏曲库与主列表焦点切换 |
| `b` | 唤出曲库切换菜单 |
| `?` | 弹出快捷键帮助弹窗 |
| `Esc` | 退出歌词全屏 / 取消搜索 / 关闭弹窗 |
| `q` | 退出 TUI 界面（后台继续播放） |

## MCP：让 AI 客户端控制播放器

```bash
wocli mcp config             # 生成包含程序绝对路径的客户端 JSON 配置
wocli mcp serve              # 本地 stdio MCP 服务，由客户端启动
wocli mcp serve --read-only  # 仅搜索和查询
wocli mcp tools              # 查看工具目录和参数 Schema
```

支持搜索、播放控制、队列、收藏/歌单、歌词、下载任务和已有音源管理；与 CLI/TUI 共享后台和曲库，关闭 MCP 后音乐继续播放。默认提供 17 个工具，只读模式开放 8 个查询工具。接入方式、资源和协议版本见 [MCP 文档](docs/mcp.md)。

## 命令行

### 帮助与音乐源

```bash
wocli help                  # 全部命令
wocli thelp download        # 分项说明，也可用 wocli download --help
wocli help keys             # 终端快捷键
wocli source list           # 音源顺序与启用状态
wocli source add ./qdy.js    # 从本地导入 LX 音源
wocli source test kuwo-mobile --id 228908 --quality 128k
wocli retry                 # 重新尝试当前歌曲及备用源
```

默认音源直接访问国内平台，无需播放时从 GitHub 下载脚本。支持音乐源的启停、优先级与用户导入，详见[音乐源与帮助说明](docs/sources.md)。目前 LX 兼容 `kw/musicUrl`，仍使用酷我搜索目录；部分高级脚本 API 尚不支持。

### 播放控制

```bash
wocli search '心做し majiko'
wocli play '心做し majiko' --pick 1
wocli play --id 61631119 --quality 320k
wocli play ~/Music/song.flac
wocli pause
wocli resume
wocli toggle               # 或 wocli play-pause
wocli next                 # 切下一首（支持别名 wocli skip，--no-wait 异步）
wocli prev                 # 切上一首（支持别名 wocli previous）
wocli seek +15
wocli volume 60
wocli mode shuffle          # list / repeat / single / shuffle
wocli status --json
wocli stop
```

### 托盘与任务栏歌词

```bash
wocli tray start           # 启动系统托盘小图标服务（macOS 菜单栏 / Windows 通知区）
wocli tray status          # 查看托盘运行状态
wocli tray stop            # 停止托盘服务

wocli lyrics-bar on        # 开启任务栏 / 状态栏实时歌词
wocli lyrics-bar off       # 关闭任务栏实时歌词
wocli lyrics-bar toggle    # 快速切换开启/关闭
```

- **macOS**：菜单栏显示当前逐句歌词，点击展开播放控制菜单。
- **Windows**：通知区托盘与任务栏歌词条，包含上一首、播放/暂停、下一首按钮；支持拖动文字区域定位、双击打开终端、右键菜单。
- **Linux**：当前提供终端与命令行控制，尚未实现托盘。

```bash
wocli like                 # 喜欢或取消喜欢当前歌曲
wocli favorites
wocli playlist create '通勤'
wocli playlist add '通勤' --id 61631119
wocli playlist show '通勤'
wocli playlist play '通勤'
wocli playlist rename '通勤' '每天听'
wocli queue add --id 61631119 --next
wocli lyrics --id 61631119 --refresh --output song.lrc
wocli download --id 61631119 --quality 320k --lyrics --output ~/Music/wocli
```

在终端界面选中歌曲后按 `d`，或打开右上角“下载任务”并点击“新建 d”。点击“开始下载”或按 Enter 提交，失败原因会显示在下载弹窗中；任务列表支持鼠标重试/取消及 `r` / `c` 快捷键。本地文件会提示已有保存位置。

`--json` 用于脚本输出；非交互搜索播放请提供 `--pick` 或 `--id`，避免误选歌曲。`--no-wait` 让 `play` 在后台开始加载后立即返回。`download --force` 允许在下载成功并校验后覆盖目标。完整命令见 `wocli --help`。

```bash
wocli daemon status
wocli daemon stop
```

## 环境变量与配置

wocli-music 可以通过 CLI 命令设置持久化偏好，也可以使用环境变量在运行时灵活调整路径与网络行为。

### 环境变量

| 环境变量 | 默认值 | 说明 |
| :--- | :--- | :--- |
| `WOCLI_HOME` | `~/.config/wocli-music` (Linux/macOS)<br>`%APPDATA%\wocli-music` (Windows) | 数据与配置主目录。存放 SQLite 曲库 (`library.sqlite3`)、配置 (`settings.json`)、后台日志 (`service-rust.log`) 和歌词缓存。遵循 `XDG_CONFIG_HOME`。 |
| `WOCLI_RUNTIME_DIR` | `/tmp/wocli-rust-{uid}-{hash}` (Linux/macOS)<br>`%LOCALAPPDATA%\wocli-music\run` (Windows) | 运行期 IPC 控制 Socket/Endpoint 目录。类 Unix 下存放 `control.sock`，Windows 下存放 `control.endpoint`。 |
| `WOCLI_DOWNLOAD_BASE` | `https://wocli.yhan.fun` | 安装器与新版自更新的分发地址；设为 `github` 直连 GitHub。详见 [分发配置](docs/distribution.md)。 |
| `WOCLI_DOWNLOAD_DIR` | `~/Music/wocli` (TUI) / `downloads` (CLI) | 歌曲与歌词下载的默认目标保存目录。 |
| `WOCLI_AUDIO_OUTPUT` | *(默认系统声卡)* | 设为 `null` 时禁用实际音频硬件输出（仅解码不发声）。适用于无声卡服务器、容器环境或自动化测试。 |
| `HTTP_PROXY` / `HTTPS_PROXY` | *(无)* | HTTP / HTTPS 代理服务器地址（支持 socks5:// 或 http://），内部网络请求自动遵循代理设置。 |
| `ALL_PROXY` / `NO_PROXY` | *(无)* | 全局代理或跳过代理的主机名单。 |

使用示例：

```bash
# 使用独立的数据目录（多用户或多测试配置）
WOCLI_HOME=~/.config/wocli-work wocli

# 设定默认下载保存路径并直接启动
WOCLI_DOWNLOAD_DIR=~/Music/Favorites wocli

# 在无音频设备的远程服务器或容器中运行测试
WOCLI_AUDIO_OUTPUT=null wocli doctor
```

### 持久化配置

使用 `wocli config` 读写持久化设置（保存在 `$WOCLI_HOME/settings.json`）：

```bash
# 查看当前全部配置
wocli config

# 设置首选音质: best / 320k / 192k / 128k
wocli config quality best

# 设置默认循环模式: list (顺序) / repeat (列表循环) / single (单曲循环) / shuffle (随机)
wocli config mode repeat

# 设置启动音量 (0 - 100)
wocli config volume 70

# 设置歌词微调偏移 (毫秒，正数歌词提前，负数延后)
wocli config lyric_offset 200

# 忽略环境代理直接直连网络 (true / false)
wocli config direct true
```

### 环境诊断

运行 `wocli doctor` 可以快速排查当前环境、依赖、有效路径、活跃环境变量与运行状态：

```bash
wocli doctor
```

```json
{
  "architecture": "aarch64",
  "engine": "rodio/symphonia",
  "env": {
    "WOCLI_DOWNLOAD_DIR": "/Users/you/Downloads/Music"
  },
  "home": "/Users/you/.config/wocli-music",
  "platform": "macos",
  "runtime": "native",
  "runtime_dir": "/tmp/wocli-rust-501-bde92153d833",
  "service_running": false,
  "socket": "/tmp/wocli-rust-501-bde92153d833/control.sock",
  "version": "0.3.5"
}
```

## 常见问题

**旧版 `update` 连不上 GitHub？** v0.3.1–v0.3.4 的更新地址仍是 GitHub。重新执行本页安装命令升级到 v0.3.5 或更新版，后续 `wocli update` 就会使用 Cloudflare；不用重新导入曲库。

**Windows 安装后找不到 `wocli`？** 新安装器会配置用户及当前 PowerShell 的 PATH；其他已经打开的终端需完全退出并重开终端应用。自定义目录和旧安装的修复方式见 [安装](#安装)。

**可以通过 SSH 或 AI 客户端在本机听到远端声音吗？** CLI/MCP 控制运行后台的那台机器，声音也从那台机器输出，不提供远程音频转发。`WOCLI_AUDIO_OUTPUT=null` 仅用于无声测试。

**为什么搜索到却播不了？** 搜索索引和音频授权并不一致。上游可能没有当前档位地址，只返回试听，或网络暂时失败。程序保持同一歌曲 ID 尝试其他音质，不用同名翻唱替换；失败信息区分无音源、仅试听、网络和解码错误。

**为什么没有声音？** 用 `wocli status` 检查状态和音量，确认系统输出设备。Linux 使用 ALSA 及桌面系统已有的声音配置；无声服务器没有可播放的硬件设备。`WOCLI_AUDIO_OUTPUT=null` 仅供测试，会解码但不输出声音。

**歌词不匹配或没找到？** 优先按歌曲 ID 获取，备用源会比较歌名、歌手和时长；Live/Remix 等版本不能随意混用。可独立刷新，或在本地音频旁放同名 LRC。

**如何备份？** 先停后台，复制整个数据目录。下载目录独立保存，按自己设置的位置备份。

## 开发与构建

需要 Rust **1.93.1+** 与 C 编译器。Linux 构建需 `libasound2-dev`、`pkg-config`；macOS 使用 Xcode Command Line Tools。

```bash
git clone https://github.com/yhan-sun/wocli-music.git
cd wocli-music
cargo run --release
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release --locked
```

发布构建使用 `opt-level=z`、LTO、单 codegen unit、符号裁剪。GitHub Actions 为 Windows x86_64、macOS ARM64 / Intel、Linux x86_64 构建，验证解压后的实际产物，再发布 `v*` 标签；Linux 额外运行无 Python/mpv 的干净容器测试。Python 仅用于 CI 打包和截图，不是播放器运行依赖。

截图由真实 Rust TUI 的 TestBackend 渲染，使用隔离的虚构示例曲库：`uv run --with resvg-py python scripts/screenshots_native.py`。v0.3.0 已完全移除旧版 Python 实现，播放器由 Rust 实现，LX 兼容层内嵌 QuickJS。

## 致谢与第三方组件

终端交互参考 [MiaowCham/ncm-cli](https://github.com/MiaowCham/ncm-cli)。界面、音频、数据存储、脚本与 MCP 能力分别基于 Ratatui、Rodio/Symphonia、SQLite、QuickJS 和官方 Rust MCP SDK 等组件。

第三方许可见每个发行包中的 `THIRD_PARTY.txt` 与 `licenses/`；相应的 `*-sources` 包提供所需依赖源码，运行播放器无需下载这些源码包。
