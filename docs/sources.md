# 命令帮助与音乐源

```bash
wocli help
wocli thelp download
wocli help source
wocli play --help
wocli help keys
```

`help`、`thelp`、`命令 --help` 共用分项说明，也支持 `--json`。查看帮助不创建数据目录、不启动后台。

## 默认使用国内平台接口

默认启用两个原生源，按以下顺序尝试：

| ID | 作用 |
| --- | --- |
| `kuwo` | 酷我原生解析，按请求音质依次尝试 P2P 与公开接口 |
| `kuwo-mobile` | 酷我移动接口备用，独立获取歌曲地址，核对歌曲 ID、音质与时长 |

两者直接访问酷我域名，不需要在播放时访问 GitHub、下载脚本或安装 Node.js。这里的“国内”指国内音乐平台的直接接口，用户导入的第三方服务由各自维护者部署。

搜索、收藏、队列仍使用酷我目录和 `kuwo:ID`。换源保持同一歌曲 ID，不自动拿其他平台的相同数字 ID 或同名翻唱替换。

播放和下载共用配置。解析、音频传输或解码失败后尝试其他已启用源；单个源内部允许按所选音质降级。全部失败后，播放跳过当前歌曲，下载任务显示错误。播放中的网络中断先重试当前源一次，再换源。音频设备故障直接提示，不会把它误判为音乐源故障。

`best` / `flac` 尝试 FLAC、320k、128k；`320k` 尝试 320k、128k；`128k` 仅尝试 128k。解码时会拒绝明显不足的试听时长。下载在临时目录完成传输与解码校验后才提交到目标文件；失败的 `--force` 下载也会保留已有文件。

## 管理与诊断

```bash
wocli source list
wocli source show kuwo-mobile
wocli source test kuwo-mobile --id 228908 --quality 128k
wocli source prefer kuwo-mobile
wocli source disable kuwo-mobile
wocli source enable kuwo-mobile
wocli retry --quality 128k
```

`prefer` 启用并移到尝试顺序第一位。内置源可停用，用户源还可 `source remove ID`。`source test` 只测试指定源（即使它已停用），检查地址和音频头，不开始播放。`--direct` 忽略环境代理，所有命令支持 `--home` 指定独立配置目录。

管理命令即时保存到数据目录中的 `sources.json`；后台在下次播放、重试或下载时读取，无需重启。已选中的音频连接不会因配置变化被中断。`status --json` 中的 `source` 表示本次选择的源，`source_failures` 保存本次播放在音频加载阶段的换源原因。

## 导入 LX 音源

可从 [pdone/lx-music-source](https://github.com/pdone/lx-music-source) 获取脚本，然后本地导入。网络受限时，优先将脚本保存到本地再导入，也可使用自己选择的可直连下载地址。

```bash
wocli source add ./qdy.js --name QDY
# 输出新源的 ID，例如 user-012345abcdef
wocli source test user-012345abcdef --id 228908 --quality 128k
wocli source prefer user-012345abcdef
```

也可直接导入 HTTP/HTTPS 网址：

```bash
wocli source add https://raw.githubusercontent.com/pdone/lx-music-source/main/qdy/latest.js
```

远程脚本只在明确执行 `source add` 时下载，导入后使用本地副本；播放时不请求 GitHub。脚本以 SHA256 摘要存储，每次读取校验完整性。相同脚本不会重复导入。更新脚本时可先导入新版并测试，再调整顺序、移除旧版。

兼容范围以 [LX 官方协议](https://lxmusic.toside.cn/desktop/custom-source) 为准，目前实现：

- `globalThis.lx` 的 `inited`、`request`、`musicUrl`，以及 `kw` 歌曲信息。
- Promise / async、`setTimeout` / `clearTimeout`、`URLSearchParams`、基础 Buffer 与文本编码。
- `lx.request` 的 HTTP(S)、请求头、body、form、超时和取消回调；回调提供 `err, resp, body`，其中 `resp.body` 对 JSON 自动解析。
- `lx.utils.buffer`、MD5、随机字节及 zlib 压缩/解压。

暂不提供 AES/RSA、`formData`、Node.js 模块、文件/进程访问、浏览器 DOM 或其他平台的搜索/歌词协议。依赖这些能力的脚本可能无法使用，会给出兼容错误并允许继续换源。当前仓库的幻音、长青、QDY、星海脚本已通过导入和初始化检查，接口是否可用应另行运行 `source test`；初始化成功不等于接口可播放。

脚本在内嵌 QuickJS 中执行，无需额外运行时。每个上下文限制为 32 MiB 内存、512 KiB 栈；初始化最多 3 秒、一次解析最多 10 秒，脚本/HTTP 响应最多 2 MiB，单次操作最多 32 个 HTTP 请求。没有文件或进程 API；脚本可通过 HTTP 请求访问网络，因此导入前应自行选择可信来源。

## HTTP 模板音源

对于自己的解析服务，可以直接创建 JSON，无需编写 JavaScript：

```json
{
  "name": "我的解析服务",
  "url": "https://music.example.cn/resolve?id={id}&quality={quality}",
  "response_path": "/data/url",
  "qualities": ["320k", "128k"]
}
```

```bash
wocli source add ./my-source.json
```

`response_path` 是 JSON Pointer，上例适用于 `{"data":{"url":"https://.../song.mp3"}}`。如果 `url` 直接返回或重定向到音频，省略 `response_path`。支持 `{id}`（不含 `kuwo:` 前缀）、`{source}`（`kw`）、`{quality}`、`{title}`、`{artist}`；替换值会经过 URL 编码。
