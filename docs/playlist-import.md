# 歌单导入、导出与修复

## 导入其他播放器的歌单

从原播放器导出歌单文件，再指定你希望使用的名称：

```bash
wocli playlist import ./通勤.csv --name "每天通勤"
wocli playlist import ./收藏.lxmc --name "以前的收藏"
wocli playlist import ./MusicFree.json --list "我的收藏" --name "旧手机收藏"
```

TUI 按 `I` 打开导入窗口，也可从 `b` 音乐库菜单进入。填写路径和新名称后，后台解析与匹配；多张歌单可用上下键选择。预览数量后点击“确认导入”，Esc 取消。`Tab` 切换字段，空格切换 TXT 顺序及离线选项。路径支持粘贴，文件须为 UTF-8。

| 格式 | 导入范围 | 导出 |
| --- | --- | --- |
| CSV / TSV | 中英文表头、引号、换行、BOM；title/歌名、artist/歌手、album/专辑、platform、trackId、duration、isrc、url、path | 支持，UTF-8 BOM |
| TXT | 默认每行“歌手 - 歌名”；`--text-order title-artist` 切换顺序；从第一个 ` - ` 分隔 | 支持，歌手 - 歌名 |
| wocli JSON v1 | 完整歌单、顺序、来源、匹配记录、候选和重复条目 | 完整备份 |
| LX `.lxmc` / JSON | **单歌单 `playListPart_v2`**；支持 gzip 与普通 JSON，按官方 `meta.songId` 读取来源 ID | 请用 wocli JSON/CSV 备份 |
| MusicFree JSON | `musicSheets[].musicList`；多歌单需选择，**不安装 plugins** | 请用 wocli JSON/CSV 备份 |
| M3U / M3U8 / PLS | 本地路径、曲目描述；相对路径以歌单文件目录为基准；拒绝 HLS 分片清单 | M3U8 仅导出有本地文件路径的歌单 |
| JSPF | 开放 JSON 歌单结构，时长从毫秒转换为秒 | 支持曲目元数据 |

CSV `duration` 使用秒，也接受 `03:45`；`duration_ms` / `durationMilliseconds` / `时长毫秒` 使用毫秒。TXT 不能可靠表达名称中的分隔符、多行标题和完整来源信息，完整迁移优先选 JSON 或 CSV。

**暂未支持**：网易/QQ/酷我在线歌单分享链接、LX 旧版及全量备份、Apple Music/iTunes XML、XSPF、Spotify 账号 JSON、XLS/XLSX。请先在原工具导出当前支持的格式。不能把任意 JSON 或音乐源脚本当作歌单文件。

## 多歌单与预览

```bash
wocli playlist scan ./MusicFree.json
wocli playlist import ./MusicFree.json --list 2 --name "我的新歌单" --dry-run
wocli playlist import ./通勤.csv --name "通勤" --offline --dry-run --json
```

`--list` 可以是歌单名称或从 1 开始的序号；名称相同时优先用序号。`--dry-run` 不保存歌单、歌曲或匹配结果。`--offline` 仅匹配本地曲库及存在的本地文件，避免等待网络。

每份输入和解压内容最多 16 MiB，最多 1000 张歌单、总计 10000 条歌曲。网络匹配最多四个并发请求，显示处理进度；TUI Esc 立即取消保存，正在进行的网络请求会在超时后退出。CLI Ctrl+C 退出，保存事务开始前不会写入半张歌单。

## 跨平台匹配与未匹配歌曲

导入的是曲目信息，**不会自动下载音频**。网易、QQ 等来源 ID 与酷我 ID 分开保存；在线播放仍使用现有酷我提供器与配置的音乐源。

匹配会核对歌名、完整歌手、Live 等版本信息，以及可用时的时长。多个候选无法区分时保留“待确认”，不会直接选搜索第一项。只有匹配好的歌曲进入播放列表；未匹配、无效和重复的原条目仍保存在完整备份中。相同目标歌曲在播放列表中只保留一次。

```bash
wocli playlist inspect "每天通勤" --json
wocli playlist resolve "每天通勤"            # 重新匹配未完成项
wocli playlist resolve "每天通勤" --dry-run  # 只预览修复结果
wocli search "歌名 歌手"
wocli playlist match "每天通勤" --entry 3 --id kuwo:123456
wocli playlist drop "每天通勤" --entry 4
```

`--entry` 使用 `inspect` 原始条目顺序，从 1 开始，包含未匹配及重复项，与仅列可播放歌曲的 `playlist show` 序号不同。手动确认会记住选择，后续自动匹配不会覆盖。缺失的本地文件可修复原路径后执行 `resolve`。

TUI 歌单标题显示待匹配数量；歌单 `⋯` 菜单的“导入条目 / 重试匹配”可预览并重新匹配，确认更新时会备份旧内容。逐条手动选择目前使用 CLI。

## 同名处理与恢复

默认另存为“名称 (导入 2)”，不覆盖已有歌单。也可明确指定：

```bash
wocli playlist import ./通勤.csv --name "通勤" --on-conflict error
wocli playlist import ./通勤.csv --name "通勤" --on-conflict merge
wocli playlist import ./通勤.csv --name "通勤" --on-conflict replace
wocli playlist backups "通勤"
wocli playlist restore "通勤" --id 12
```

合并会按目标歌曲 ID、原平台及 ID、本地路径或完整元数据去重。合并、替换、匹配修复和删除条目前，在 SQLite 中保存旧歌单快照；`restore` 恢复前也保存当前版本。写入使用事务，失败时回滚。普通改名、增删歌曲仍与导入元数据保持一致。

## 导出与备份

```bash
wocli playlist export "通勤" --format json --output ./通勤.wocli.json
wocli playlist export "通勤" --format csv --output ./通勤.csv
wocli playlist export "本地音乐" --format m3u8 --output ./本地音乐.m3u8
```

TUI 打开歌单后按 `E`，填写保存路径和格式。文件已存在时默认拒绝覆盖；CLI 可明确添加 `--force`。文件先写入临时文件，完成后原子保存。JSON 可完整保存来源和匹配状态；CSV/TXT/JSPF 是交换曲目元数据，不能保存所有应用状态。M3U8 需要本地路径，在线歌曲没有稳定本地文件时会明确拒绝，避免导出一个无法播放的空壳清单。
