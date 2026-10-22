# 播放器对比与选择

核对日期：**2026-10-22**。wocli-music 以 v0.3.5 代码与正式发行包为依据；其他项目以本文链接的官方仓库和文档为依据。文档记录功能，不代表各平台接口在每个网络环境中都能使用。

## 先看使用方式

| 你主要想做什么 | 可考虑的方案 | wocli-music 的侧重点 |
| --- | --- | --- |
| 在终端听歌，再用脚本或 AI 控制同一播放器 | wocli-music | TUI、CLI 和本地 stdio MCP 共用后台与 SQLite 曲库，减少入口之间的状态切换。 |
| 使用平台账号及订阅提供的离线播放 | [Spotify 官方客户端](https://support.spotify.com/us/article/listen-offline/) | wocli 无需平台登录即可搜索公开音源，收藏保存在本地；不接入 Spotify 曲库或同步其账号。可播放性仍取决于所选音源。 |
| 在终端使用网易云生态，并接入系统媒体协议或投送 | [go-musicfox](https://github.com/go-musicfox/go-musicfox) | wocli 使用酷我搜索目录、自己的本地收藏与队列，目前没有网易云账号同步、MPRIS 或 DLNA。 |
| 使用图形界面、自定义音源与多端列表同步 | [LX Music 桌面版](https://github.com/lyswhut/lx-music-desktop) | wocli 提供终端界面及内置备用解析；仅兼容部分 LX 协议，目前不提供列表云同步。 |
| 用图形界面浏览网易云账号内容、MV 和推荐 | [YesPlayMusic](https://github.com/qier222/YesPlayMusic) | wocli 聚焦搜索、播放、歌词和收藏，目前没有账号、MV 或推荐系统。 |
| 管理已有的 MPD 服务、媒体库、标签与可视化 | [ncmpcpp](https://github.com/ncmpcpp/ncmpcpp) + [MPD](https://www.musicpd.org/) | wocli 自带音频后台，按需启动，无需单独配置 MPD 服务；不实现 MPD 协议或标签编辑器。 |

这些工具都能在各自定位中发挥作用。选择依据应是所需曲库、控制方式和部署环境。

## wocli-music 的具体优势

### 多个入口共享状态

TUI 负责浏览与键鼠交互，CLI 提供播放控制和 JSON 状态，本地 MCP 提供 17 个工具与 5 个只读资源。三个入口操作同一个后台与数据目录；关闭界面或 MCP 后，后台继续播放和处理已提交的下载。可用 `--home` 选择独立曲库。

LX Music 也有 [本地 HTTP 开放 API](https://lxmusic.toside.cn/desktop/open-api)，MPD 也有网络协议控制能力。因此这里强调的是 wocli **内置 CLI 与 MCP、共用状态的接入方式**，并不将其他播放器描述为“无法自动化”。

### 运行依赖随包提供

wocli 使用 Rust 原生音频栈，内嵌 QuickJS 执行支持的音源脚本。运行正式发行包无需另装 Rust、Python、Node.js、mpv 或 FFmpeg。Linux 随包提供 ALSA 库，仍依赖系统的声音配置与兼容的 glibc。

v0.3.5 的四个平台压缩安装包如下，取自 [正式 Release](https://github.com/yhan-sun/wocli-music/releases/tag/v0.3.5)，包含各包随附文件；不含独立的 `*-sources` 源码包。MB 按 1,000,000 字节计。

| 平台 | 文件字节数 | 压缩大小 |
| --- | ---: | ---: |
| macOS ARM64 | 3,013,352 | 3.01 MB |
| macOS Intel | 3,152,804 | 3.15 MB |
| Linux x86_64 | 3,764,503 | 3.76 MB |
| Windows x86_64 | 4,206,358 | 4.21 MB |

这些数字不是内存或 CPU 测试，也不用于推断比 Electron、Go 或 MPD 更省资源。本项目当前没有同机、同曲目、同网络的横向性能基准，因此不宣称“固定 200ms 起播”“内存降低多少”或“音质更好”。

### 默认备用源与可检查的失败处理

默认包含酷我原生与移动备用接口，播放时无需去 GitHub 获取脚本。解析、音频传输或解码失败会尝试其他启用源；单曲全部失败后跳过，整队列失败则停止。下载在临时目录完成完整性与解码检查，再提交到目标路径。具体行为见 [音乐源文档](sources.md)。

导入的 LX 脚本仅支持 `kw/musicUrl` 及已实现的运行时 API，不能将其他平台的 ID 直接混用。LX 原项目的 [音源协议](https://lxmusic.toside.cn/desktop/custom-source) 能力更广，不能把部分兼容宣传为完全替代。

### 下载与更新提供独立入口

[Cloudflare 分发入口](https://wocli.yhan.fun/) 提供安装脚本、版本信息、四个平台包、源码与 SHA256 文件。v0.3.5 起原生自更新默认使用该入口，减少终端用户对 GitHub 直连的依赖；可用环境变量切换镜像或直连 GitHub。

这是缓存分发服务，仍依赖 GitHub 上游；并非永久离线镜像，也不保证中国大陆各运营商的速度。部署与维护见 [分发文档](distribution.md)。

## 资料来源与维护

- [Spotify 官方离线播放说明](https://support.spotify.com/us/article/listen-offline/)：Premium 的专辑、歌单下载与离线播放条件。
- [go-musicfox 官方 README](https://github.com/go-musicfox/go-musicfox)：项目定位、平台支持与系统集成。
- [LX Music 官方 README](https://github.com/lyswhut/lx-music-desktop)：桌面应用、自定义音源、同步服务与开放 API。
- [YesPlayMusic 官方 README](https://github.com/qier222/YesPlayMusic)：图形界面、账号、视频与推荐功能。
- [ncmpcpp 官方 README](https://github.com/ncmpcpp/ncmpcpp)：MPD 客户端、曲库、标签编辑与可视化。
- [MPD 官方网站](https://www.musicpd.org/)：独立音乐服务与协议控制。

没有列出的能力不等同于“不支持”；不因文档未提到 MCP 就断言其他项目没有相关扩展。更新对比时应同步核对来源和日期；新增性能优势需给出可复现的测试条件。
