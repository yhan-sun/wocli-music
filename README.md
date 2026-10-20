# wocli-music

无需登录的酷我终端音乐播放器。搜索、选歌、歌词和本地收藏都在终端完成，退出界面后仍可继续播放。交互体验参考 [MiaowCham/ncm-cli](https://github.com/MiaowCham/ncm-cli)，音源协议来自现有 `../music/kw_full.py`，独立实现 Python 播放器和后台服务。

## 快速开始

支持 **macOS / Linux，Python 3.11+，mpv**。无需账号、Cookie、API Key，也无需部署 API 服务器。

```bash
# 安装音频后端（二选一）
brew install mpv          # macOS
sudo apt install mpv     # Ubuntu / Debian

# 在项目目录运行，uv 会创建隔离环境并安装 Python 依赖
uv sync
uv run wocli
```

也可以使用 `pipx install .` 或 `uv tool install .` 安装全局命令 `wocli` / `wocli-music`。

## 终端界面

左侧选择「我喜欢」、播放队列、最近播放和自建收藏夹，中间选歌，右侧显示当前歌曲及滚动歌词。搜索默认收起，按 `/` 或 `Ctrl+F` 展开紧凑输入栏，Esc 回到原来的音乐库。

- 上下键选歌，Enter 从选中歌曲播放整份列表；空格播放 / 暂停。
- `f` 喜欢 / 取消喜欢选中的歌；底部爱心操作当前播放的歌。
- `c` 打开「收藏到…」，选择已有收藏夹或新建；记住上次使用的收藏夹。
- `n` 新建收藏夹，`r` 重命名当前收藏夹；Delete 将选中歌曲移出当前收藏夹、我喜欢或队列。
- 收藏夹底部可重命名或删除。删除前会确认，只移除该夹及收录关系，其他收藏保留。
- `m` 或底部播放方式按钮打开菜单：顺序播放、列表循环、单曲循环、随机播放；数字 `1–4` 直接选择。
- 随机播放每轮不重复，「上一首」返回实际播放历史；单曲循环时仍可手动切歌。
- 「播放全部 / 随机」直接播放当前列表；`a` 将选中歌曲加入队列。
- `l` 重新获取当前歌曲的歌词，不会中断音频播放。
- `Ctrl+←` / `Ctrl+→` 上一首 / 下一首；左右键后退 / 前进 5 秒。
- `b` 打开音乐库选择菜单；窄终端也可选择任意收藏夹。
- `q`（非输入框中）、`Ctrl+Q`、`Ctrl+C` 退出界面，音乐继续播放。

不足 110 列时收起右侧歌词，不足 68 列时使用顶部「曲库」菜单；歌曲列表和播放控制保持可用。收藏夹和播放方式保存在本地，重新打开仍会保留。

停止音乐：`wocli stop`；彻底退出后台服务：`wocli daemon stop`。

## 命令行

以下命令在源码目录中也可写成 `uv run wocli …`。

```bash
wocli search "周杰伦 晴天"
wocli search "周杰伦" --page 2 --limit 20 --json
wocli play "周杰伦 晴天"              # 交互选择
wocli play "周杰伦 晴天" --pick 1     # 脚本中明确选歌
wocli play --id kuwo:228908
wocli play --id 228908 --quality flac
wocli play ~/Music/song.flac         # 支持本地文件和同名 .lrc
wocli play                          # 从上次队列所选歌曲重新开始

wocli pause
wocli resume
wocli next
wocli prev
wocli seek +10
wocli seek -10
wocli volume 60
wocli mode repeat                   # list / repeat / single / shuffle
wocli status --json
wocli stop

wocli queue add --id kuwo:228908
wocli queue add "歌手 歌名" --pick 1 --next
wocli queue list
wocli queue remove 2
wocli queue clear

wocli like                          # 切换当前歌曲收藏状态
wocli like --id kuwo:228908
wocli favorites
wocli history
wocli playlist create "工作"
wocli playlist add "工作" --id kuwo:228908
wocli playlist show "工作"
wocli playlist play "工作"
wocli playlist remove "工作" --id kuwo:228908
wocli playlist rename "工作" "通勤"
wocli playlist delete "通勤"

wocli lyrics --id kuwo:228908
wocli lyrics --id kuwo:61631119 --refresh  # 重新获取并更新缓存
wocli lyrics --output song.lrc       # 当前歌曲歌词；已有文件需 --force
wocli download --id kuwo:228908 --quality 320k --lyrics --output ./downloads
wocli config
wocli config quality 320k
wocli config lyric_offset 200        # 正数让歌词提前，单位毫秒
wocli config direct true             # 显式忽略系统代理
wocli doctor
wocli daemon status
wocli daemon stop
```

`--json` 适用于数据和控制命令，错误通过 stderr 返回并使用非零退出码。搜索多条结果时，非交互环境要求提供 `--pick` 或 `--id`，避免自动播放错误版本。单次 `play` 替换当前队列，连续听歌请使用 `queue add` 或 `playlist play`。

## 音源与音质

默认偏好 320k，无法获得该档位时尝试 128k。`flac` 从 FLAC 开始向下尝试，`best` 尝试所有支持档位。播放状态始终显示实际解析到的音质。

旧 P2P 链路无可用音频时，会用同一歌曲 ID 请求公开播放接口作为备用通道，并同样检查音频内容与长度。接口返回成功和 URL 仍可能只对应试听片段，这类音频会被拒绝，并在界面说明未取得完整版。`wocli status --json` 中的 `error_code` 和 `error_details` 提供分类及各档位的诊断信息。

音源模块在播放前读取少量媒体数据，检查音频头及声明长度；不把“返回 URL”视为验证整首音频成功。下载会校验总长度，并在服务器提供 MD5 时进行比对，通过后再替换目标文件。

歌词优先按歌曲 ID 获取酷我原生时间轴，再尝试酷狗和网易云。跨源匹配检查歌名、歌手与时长，兼容“カバー / cover / Explicit”等标题注释，保留现场版、Remix 等版本差异。仅缓存有效的时间轴歌词；获取有总时间上限，异常或缺失均不阻塞音频播放。本地缓存更新后，界面会自动读取，支持用 CLI 重新获取后直接更新歌词显示。

歌词区分别显示音源解析、歌词加载、歌词缺失及播放失败，不会在播放失败后继续显示加载中。

无需登录不代表上游所有歌曲与音质均可用。拒绝、试听限制、网络错误或接口变化会明确报错；不会自动搜索并替换成其他版本的歌曲。音源链接临近播放时解析，不会将临时签名 URL 存入歌单。

## 配置与数据

默认数据目录：`$XDG_CONFIG_HOME/wocli-music` 或 `~/.config/wocli-music`，可用 `WOCLI_HOME` 覆盖。运行时 socket 放在当前用户独立的临时目录中。

| 文件 | 内容 |
| --- | --- |
| `settings.json` | 音质、音量、模式、歌词偏移、代理设置 |
| `library.sqlite3` | 搜索元数据、收藏、歌单、历史、队列、歌词 |
| `service.log` | 后台服务日志 |
| `mpv.log` | 音频后端日志 |

系统代理默认受尊重，HTTPS 使用证书校验。单次搜索或下载可加 `--direct`；后台播放如需直连，使用 `wocli config direct true`。`WOCLI_MPV=/path/to/mpv` 可指定播放器；`WOCLI_AUDIO_OUTPUT=null` 用于无声的播放器集成验证。

也会自动发现 `/Applications/mpv.app`、`~/Applications/mpv.app` 及数据目录下的 `tools/mpv.app`。本机验证使用隔离安装的 mpv 官方 CI 构建，来源和 SHA256 记录在该目录的 `source.json` 中。

停止服务会终止它启动的 mpv 进程；再次启动保留队列但不会自行恢复出声。第一版未实现封面图片协议、平台账号歌单同步、系统媒体键、音频持久缓存或 Windows 后台控制。

## 开发

```bash
uv sync --group dev
uv run pytest
uv run ruff check .
uv run python -m build
uv run python scripts/preview.py     # 导出真实搜索界面的 SVG，访问音源但不播放
```

`tests/` 包含协议解析、音频探测、下载校验、队列与异步切歌、IPC 生命周期、CLI 与 Textual 无头交互测试。测试不依赖真实账号，不会默认访问外部音源或输出声音。

模块：`provider.py` 音源，`player.py` mpv IPC，`service.py` 后台状态，`client.py` 控制客户端，`store.py` 本地曲库，`cli.py` 命令行，`tui.py` 交互界面。

## 参考与致谢

- [MiaowCham/ncm-cli](https://github.com/MiaowCham/ncm-cli)：终端选歌、常驻播放器和歌词交互的设计参考，MIT 许可，许可文本见 [NOTICE.md](NOTICE.md)。未引入其网易云登录与远端 API 部署流程。
- `../music/kw_full.py`：酷我搜索、音质表与 P2P 解析链路。
- [mpv](https://mpv.io/) / [Textual](https://textual.textualize.io/) / [Rich](https://rich.readthedocs.io/)：音频播放与终端界面。
