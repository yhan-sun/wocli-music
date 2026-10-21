# wocli-music

在终端搜索、听歌、看歌词和整理收藏夹。Rust 原生播放器，退出界面后音乐继续播放。

[下载发行版](https://github.com/yhan-sun/wocli-music/releases/latest) · [安装](#安装) · [使用方案](#使用方案) · [命令行](#命令行) · [常见问题](#常见问题) · [更新日志](CHANGELOG.md)

![Rust 版：收藏夹、同步歌词和播放控制](docs/screenshots/library.png)

## 能做什么

- 搜索歌名或歌手，点击选中、双击或 Enter 播放；支持本地 MP3、FLAC、AAC/M4A、WAV。
- 我喜欢、自建收藏夹、播放队列、最近播放。歌曲列表与歌词面板均有「收藏到…」。
- 顺序、列表循环、单曲循环、随机播放；点击或拖动进度条跳转，暂停时跳转保持暂停。
- 同步歌词、本地同名 LRC、独立刷新；歌曲网络失败自动重试，可手动重试或切换音质。
- 后台下载，可选目录、音质与 LRC；查看进度、取消和重试。
- CLI 与 JSON 输出，可从其他终端控制同一个后台。

**v0.2.0 用 Rust 替换了 Python、Textual、mpv 和 FFmpeg。** 音频通过 Rodio/Symphonia 解码，使用系统音频设备；SQLite 保存曲库。macOS ARM64 本地实测含许可证的压缩包约 **2.2 MB**，可执行文件约 **4 MB**。其他平台以 [Release 附件大小](https://github.com/yhan-sun/wocli-music/releases/latest) 为准；2 MB 是体积目标，未对所有平台设置相同的硬性上限。

音乐来自公开音源，搜索不需要登录；部分曲目或音质可能无完整音频。程序会拒绝明显的试听片段、错误内容和不完整传输，并显示具体错误。没有账号歌单同步、Windows 后台控制、封面或系统媒体键。

## 安装

从 [Releases](https://github.com/yhan-sun/wocli-music/releases/latest) 下载对应的 **安装包**，无需安装 Rust、Python、uv、mpv 或 FFmpeg。

| 设备 | 文件 |
| --- | --- |
| Apple Silicon Mac，macOS 15+ | `wocli-music-0.2.0-macos-arm64.tar.gz` |
| Intel Mac，macOS 15+ | `wocli-music-0.2.0-macos-x86_64.tar.gz` |
| Linux x86_64，glibc 2.35+ | `wocli-music-0.2.0-linux-x86_64.tar.gz` |

以 Apple Silicon Mac 为例：

```bash
tar -xzf wocli-music-0.2.0-macos-arm64.tar.gz
cd wocli-music-0.2.0-macos-arm64
./wocli
```

macOS 的 `wocli` 可单独复制到 `~/.local/bin/` 等 PATH 目录。Linux **保留 `lib/` 在 `wocli` 旁边**，其中只有必要的 ALSA 音频库；使用桌面已有的音频配置和设备。应用会在首次播放时打开音频输出，纯搜索、收藏管理和状态查询无需音频设备。

安装目录未加入 PATH 时，下面命令中的 `wocli` 改为 `./wocli`。

`*-sources.tar.gz` 是第三方依赖许可要求的源码归档，**运行播放器不需要下载**。Rust 版不再附带上百 MB 的 mpv/FFmpeg 依赖源码，也不再发布 Python wheel。每个安装包包含 `THIRD_PARTY.txt` 与依赖许可证。

校验下载：

```bash
# macOS
shasum -a 256 wocli-music-0.2.0-macos-arm64.tar.gz
# Linux
sha256sum wocli-music-0.2.0-linux-x86_64.tar.gz
```

将结果与 Release 中 `SHA256SUMS` 的对应行比较。macOS 发行版尚未经过 Developer ID 公证；若系统阻止打开，校验后可在系统安全设置允许，也可对已校验的解压目录执行 `xattr -dr com.apple.quarantine ./wocli-music-0.2.0-macos-arm64`。

### 从旧版升级

先退出旧界面，使用 **旧版命令** 停止后台，再启动新二进制：

```bash
# 原来从源码运行
uv run wocli daemon stop
# 或：旧二进制安装目录中的 ./wocli daemon stop

# 再运行新版本
/path/to/new/wocli
```

旧版的 `library.sqlite3` 与 `settings.json` 继续使用：我喜欢、收藏夹、历史、队列、歌词缓存和设置不需要重新导入。Rust 与 Python 后台使用不同的 socket，升级时应先停掉旧版，避免两个播放器同时发声。改过 `WOCLI_HOME` 的用户，在新版本中继续使用相同路径。

以后更新：退出界面，`wocli daemon stop`，替换安装目录，再运行。卸载：停掉后台并删除安装目录；曲库和下载文件会保留。

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

歌曲失败时显示原因，可「重试」或「切换音质」。Ctrl+R 手动重试当前歌曲；网络错误自动重试一次，单首最多两次尝试。歌词与歌曲分别加载，切歌后会丢弃上一首的过期结果。

当前在线播放先把音频缓存到磁盘并检查完整性，再开始解码，加载时显示缓冲进度。FLAC 或慢网络的首次等待会更长；可改用 320k/128k。缓存随切歌和停止清理，下载文件则长期保留。

### 保存歌曲

选中歌曲后按 `d` 或从歌曲菜单选择「下载」，编辑目录、音质与「保存 LRC」，然后开始下载。顶部「下载」查看任务、进度和错误；失败任务可重试，进行中的任务可取消。下载在后台进行，退出界面后继续，退出后台会取消未完成任务。

默认不覆盖同名文件。传输使用分块、长度检查和上游提供的 MD5；校验失败清理临时文件，保留已有文件。

### 小窗口

![Rust 版：80×26 终端](docs/screenshots/compact.png)

小窗口收起右侧歌词，`v` 单独查看。宽度不足时侧栏隐藏，按 `b` 选择曲库。按 `?` 查看完整快捷键。

## 命令行

```bash
wocli search '心做し majiko'
wocli play '心做し majiko' --pick 1
wocli play --id 61631119 --quality 320k
wocli play ~/Music/song.flac
wocli pause
wocli resume
wocli seek +15
wocli volume 60
wocli mode shuffle          # list / repeat / single / shuffle
wocli status --json
wocli stop
```

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

`--json` 用于脚本输出；非交互搜索播放请提供 `--pick` 或 `--id`，避免误选歌曲。`--no-wait` 让 `play` 在后台开始加载后立即返回。`download --force` 允许在下载成功并校验后覆盖目标。完整命令见 `wocli --help`。

```bash
wocli config quality best
wocli config mode repeat
wocli config lyric_offset 250   # 毫秒；正值让歌词提前
wocli config direct true        # 忽略环境代理
wocli doctor
wocli daemon status
wocli daemon stop
```

默认数据目录为 `~/.config/wocli-music`，遵循 `XDG_CONFIG_HOME`；`WOCLI_HOME=/path/to/data wocli` 可使用独立曲库。后台日志为该目录下 `service-rust.log`。数据库、设置和歌词均保存在本机。

## 常见问题

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

发布构建使用 `opt-level=z`、LTO、单 codegen unit、符号裁剪。GitHub Actions 为 macOS ARM64 / Intel、Linux x86_64 构建，验证解压后的实际产物，再发布 `v*` 标签；Linux 额外运行无 Python/mpv 的干净容器测试。Python 仅用于 CI 打包和截图，不是播放器运行依赖。

截图由真实 Rust TUI 的 TestBackend 渲染，使用隔离的虚构示例曲库：`uv run --with resvg-py python scripts/screenshots_native.py`。`src/wocli_music` 和 Python 打包文件暂保留为 v0.1.x 迁移参考，当前应用入口为 `cargo run` / 原生 `wocli`。

项目交互参考 [MiaowCham/ncm-cli](https://github.com/MiaowCham/ncm-cli)，音源协议从原有 `../music` 脚本整理移植；不要求登录网易云账号。
