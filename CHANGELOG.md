# 更新日志

## 0.3.1 — 2026-10-22

- Windows 后台与托盘改用原生脱离进程启动，保留 `WOCLI_*` 环境并禁止继承调用方的管道句柄，修复脚本命令不返回和无 PowerShell/CMD 搜索路径时的后台启动。

- 修复下载交互：任务页新增下载入口，提交期间避免重复请求，服务确认后才进入任务页，失败留在表单内显示；重试/取消保持任务页打开。覆盖小终端鼠标点击与键盘操作。
- 下载任务先检查目录写权限，统一相对路径解析，并正确传递 `download --direct`。

- 新增 `wocli update` 原生自更新：自动检测平台、校验 SHA256 并替换当前安装；支持 `--check`、`--force`、`--json`、`--direct`。更新保留曲库与设置，兼容 Windows 运行中文件替换及 Linux 包装脚本安装。

- 修复本地音频向后跳转失败，将解码与跳转准备移出后台控制循环；等待缓冲时仍可暂停、停止和切歌，跳转等待最多 10 秒。
- 修复流式下载的 10 秒整请求超时、尾部提前 EOF、失败缓存阻断完整下载回退，以及试听片段与不完整音频未校验的问题；恢复长度和 MD5 校验。
- 流式网络中断会重试一次；无法播放或解码提前中断的歌曲会自动跳过，支持全部播放模式，整队列失败后停止并保留错误。系统音频设备错误直接提示。
- 修复 Windows 重装后当前终端仍找不到 `wocli`；每次同步用户及进程 PATH，并通知终端启动器刷新环境。补充离线安装和重装的 Windows CI 验证。

## 0.3.0 — 2026-10-21

- **P0 真正的渐进式流式播放与边下边播（Progressive Audio Streaming）**：
  - 新增 `src/stream.rs`：实现基于原子条件变量通知的 `ProgressiveReader`（支持 `Read + Seek`）与预缓冲机制（`PREBUFFER_SIZE = 128KB`）。
  - Symphonia 解码器仅需读取前 16KB 元数据即可立即起播，首播/切歌发声延迟从数秒骤降至 **< 200ms**，后续音频流在后台静默缓冲。
  - 网络断开或异常时自动回退并妥善回收临时流句柄。
- **后台常驻服务有界线程池与并发限流（Bounded Concurrency & Stability）**：
  - 新增 `src/pool.rs`：固定容量轻量工作线程池 `WorkerPool` 取代原无界 `thread::spawn`，规范歌词、任务与流式任务的调度与生命周期。
  - 引入 `ConnectionLimiter`（基于原子 CAS 操作的高性能并发连接门禁，默认最大 16 客户端并发），防止 IPC 客户端泛洪与连接泄漏。
- **模块化音源抽象设计（Modular Provider Trait Architecture）**：
  - 抽象并规范统一音源接口 `MusicProvider` Trait（`src/provider/`），解耦公开在线协议解析、本地音频扫描与未来扩展源。
- **彻底移除历史 Python 遗留实现（100% Pure Rust）**：
  - 完全清理 `src/wocli_music/`、`pyproject.toml` 及过时构建脚本，代码库瘦身 5,100+ 行，成为真正的单二进制纯 Rust 终端音乐播放器。
- **键盘优先与 Vim 风格终端交互（Keyboard-First & Vim Navigation）**：
  - 支持 `j` / `k` 上下移动曲目。
  - 支持 `g` / `G` 快速跳转至列表顶部 / 底部。
  - 支持 `[` / `]`（或 `<` / `>`）快速切换上一首 / 下一首曲目。
  - 支持 `Ctrl+U` / `Ctrl+D` 半屏 / 翻页滚动。
  - 统一更新 TUI 快捷键帮助弹窗与交互提示。
- **国际化与英文文档（English Documentation）**：
  - 新增全面英文文档 `README_EN.md`，面向全球开源社区展示原生 Rust 架构与特性。

- 移除状态栏小图标与右键菜单中全部 emoji，改为纯净原生排版设计。
- macOS 菜单栏接入系统原生 SF Symbol 单色矢量模板图标（`music.note`），自适应深浅色模式与视网膜高分屏。
- 桌面 / 任务栏悬浮歌词条移除前缀 emoji，专注呈现纯净文本与阴影效果。


## 0.2.4 — 2026-10-21

- 修复 macOS Cocoa 状态栏 StatusItem 自动释放池回收导致图标消失的问题，显式持久化 `status_item`、`button`、`menu` 与事件目标。
- 为 macOS 新增原生桌面 / 任务栏透明悬浮歌词条（`NSPanel` + `NSTextField`）：
  - 悬浮于桌面下方（Dock 之上），鼠标完全穿透不遮挡点击，多桌面空间联动（`CanJoinAllSpaces`）。
  - 大号加粗白色字体配合柔和半透明阴影，支持随 `wocli lyrics-bar [on|off|toggle]` 实时显隐。
- 优化 macOS 状态栏标题长度限制，避免被 MacBook 刘海屏（Notch）挤出屏幕。
- 增强 `install.ps1` 与 `install.sh`：Windows 默认关闭慢速控制台进度流并优先调用 `curl.exe`；macOS 自动执行 ad-hoc 签名彻底避免 Gatekeeper 137 SIGKILL。


- 新增系统托盘小图标控制（macOS 状态栏 / Windows 通知区域）：支持播放/暂停、上一首、下一首、播放模式循环切换、音量调节与打开终端。
- 新增系统任务栏 / 状态栏实时歌词显示：
  - macOS：状态栏图标动态呈现当前唱到的实时逐句歌词（`🎵 歌词内容`）。
  - Windows：高对比度、带阴影描边、鼠标穿透的桌面/任务栏置顶透明歌词条，并在托盘悬停 Tooltip 中同步多行曲目与歌词信息。
- 完善切歌与控制命令：
  - 增强 `wocli next` 与 `wocli prev`（支持 `--no-wait` 异步切换），新增别名 `skip`、`previous`。
  - 新增播放暂停切换别名 `wocli play-pause`。
  - 新增任务栏歌词开关与查询命令 `wocli lyrics-bar [on|off|toggle|status]`。
  - 新增系统托盘服务守护命令 `wocli tray start|stop|status|run`，并在 `daemon start --tray` 中提供一体化拉起。

## 0.2.2 — 2026-10-21

- 修复 Windows Winsock 接受套接字继承非阻塞模式问题（解决偶发 `os error 10035` WSAEWOULDBLOCK 与 `10054` WSAECONNRESET）。
- 增强 Windows 后台服务启动生命周期：优先使用 WMI `Win32_Process Create` 启动常驻后台，彻底避免父控制台/SSH 会话退出时后台被误杀。
- CLI 新增 `--home` 参数支持并在后台守护进程显式传递，保证无环境变量继承场景下的路径解析一致性。

## 0.2.1 — 2026-10-21

- 支持 Windows 10/11 原生运行：重构跨平台 IPC 通信与后台常驻服务，实现零窗口后台常驻与 CLI/TUI 控制。
- 新增官方跨平台一键安装脚本体系：支持 Shell (`install.sh`) 与 PowerShell (`install.ps1`) 自动安装与自动 PATH 配置。
- 完善环境变量与运行期诊断：支持 `WOCLI_HOME`、`WOCLI_RUNTIME_DIR`、`WOCLI_DOWNLOAD_DIR` 等并在 `wocli doctor` 中展示。
- 将 Windows x86_64 接入 GitHub Actions 自动化构建与冒烟测试流水线，产出约 3.0 MB 的原生 Windows 发行包。

## 0.2.0 — 2026-10-21

- 迁移为 Rust 原生播放器，以 Rodio/Symphonia 替代 mpv/FFmpeg，不再捆绑 Python。
- 保留搜索、歌词、收藏夹、后台、CLI、播放方式、进度跳转和下载；兼容旧版 SQLite 曲库。
- macOS ARM64 本地完整安装包约 2.2 MB；Actions 改为三个平台的原生构建及真实产物验证。
- 歌曲与歌词独立重试、分块长度与 MD5 验证、下载取消、过期请求隔离。
- 重做 README、迁移说明和真实 Rust TUI 截图。

## 0.1.1 — 2026-10-21

- 新增 GitHub Actions 构建与发布：macOS Apple Silicon、macOS Intel、Linux x86_64。
- 二进制包包含 Python、mpv、证书及动态库，下载解压后直接运行，无需安装 Python、uv 或 mpv。
- 支持冻结程序启动独立后台，界面关闭后播放继续；优先使用包内音频后端。
- 打包时收集第三方许可、来源与原生依赖源码，发布各平台源码归档和完整 SHA-256 清单。
- 发布前验证解压产物的 CLI、TUI、后台、本地播放、歌词、收藏夹和进度跳转；Linux 额外验证无 Python / mpv 的干净容器。
- README 增加二进制下载、解压运行、更新、系统要求与 macOS 签名说明。

## 0.1.0 — 2026-10-21

首个发行版，支持 macOS / Linux 和 Python 3.11+，播放依赖 mpv。

- 终端内搜索、选歌、歌词同步；支持本地音频和同名 LRC。
- 独立后台播放，退出界面后继续播放，可用 CLI 控制。
- 我喜欢、自建收藏夹、播放队列和最近播放；列表与歌词面板直接「收藏到…」。
- 顺序播放、列表循环、单曲循环与随机播放，保存音量和播放方式。
- 进度条点击定位、拖动跳转，暂停时跳转保持暂停。
- 音源与歌词独立加载、重试；拒绝试听和明显不完整的音频，区分「仅试听」「无音源」及其他失败。
- 后台下载、音质与目录选择、LRC 保存、进度查看、取消和重试。
- 窄屏布局、键盘快捷键、歌曲与收藏夹操作菜单。
- GitHub Releases 提供 wheel、源码包和 SHA-256 校验文件；README 包含安装、使用场景和真实 TUI 演示截图。

已知限制：上游部分歌曲或音质不可用；收藏夹保存在本机，未同步平台账号；暂无 Windows 后台控制、系统媒体键、封面图和音频缓存。发行包不包含 mpv。
