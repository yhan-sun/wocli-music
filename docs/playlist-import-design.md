# 歌单迁移设计与格式适配调研

调研日期：2026-10-23。第一批实现见 [使用说明](playlist-import.md)：CSV/TSV/TXT、wocli JSON、LX 单歌单 v2、MusicFree、M3U/M3U8/PLS、JSPF，以及 CLI/TUI 预览、改名、匹配与快照恢复。本文保留调研依据和后续阶段设计；在线链接、Apple XML/XSPF、Spotify 和 LX 全量备份仍待实现。

## 目标与现状

用户可以选择其他播放器导出的歌单文件，或粘贴平台分享链接，预览曲目后保存为指定名称的本地歌单。导入曲目清单与下载音频是独立操作。

实现前 `src/cli.rs` 仅提供歌单创建、查看、增删歌曲、改名、删除和播放。`Track` 只有一个 `id`，在线提供器将数字 ID 作为酷我 ID；不能将网易、QQ 等平台 ID 直接传入现有播放接口。

`playlists` 用名称作为主键，`members` 用 `(name,id)` 作为主键，具有顺序字段，但不能表达未匹配曲目或重复出现的同一首歌。导入前需确定去重策略，并补充来源与匹配记录。

## 两类输入与适配顺序

| 输入 | 实际内容及适配要点 | 建议阶段 |
| --- | --- | --- |
| TXT | 一行一首，支持明确指定“歌手 - 歌名”或“歌名 - 歌手”；含连字符的名称不能随意切分 | 第一阶段 |
| CSV / TSV | 按表头映射歌名、歌手、专辑、时长、来源、原 ID、URL、ISRC；兼容中文表头、引号、BOM、CRLF | 第一阶段 |
| wocli JSON | 自有版本化备份格式，完整保存歌单、顺序、来源、匹配状态；保证导出后可无损导回 | 第一阶段 |
| LX `.lxmc` / JSON | 按内容检测 gzip/JSON，识别 `type`，先实现单歌单 `playListPart_v2`，再覆盖实际测试过的旧版与多歌单备份 | 第二阶段，优先国内生态 |
| MusicFree JSON | 识别备份中的 `musicSheets`，按歌曲的插件/平台信息解析；只导入歌单，不安装备份中的插件 | 第二阶段，优先国内生态 |
| 酷我、网易云、QQ 分享链接 | 独立平台适配器读取歌单标题、曲目、来源 ID；处理短链、分页、接口访问条件和失效链接 | 逐个平台验证后发布 |
| M3U / M3U8 / PLS | 本地文件或媒体地址列表；相对路径以歌单目录为基准，缺失文件保留待修复；HLS M3U8 切片清单不能当作歌曲列表 | 后续 |
| XSPF / JSPF | 开放的 XML/JSON 歌单格式；处理 URI、曲目元数据及必要的 wocli 扩展字段 | 后续 |
| Apple Music / iTunes XML | Apple plist 结构，不是 XSPF；通过 Track ID 关联曲目信息及歌单顺序 | 后续 |
| Spotify 账号数据 JSON | 按官方账号导出结构解析，不能当作任意 JSON 歌曲数组 | 后续 |

同一种后缀可能包含完全不同的数据。格式检测应依据内容、结构和明确的版本字段；不能声称“支持任意 JSON/XML”。XLS/XLSX 第一阶段可先要求另存 CSV，不引入额外表格运行时。

在线分享链接也不是标准歌单文件。应分开验证“能读取平台曲目清单”和“wocli 有对应可播放版本”。私有、登录受限或接口拒绝访问时保留准确错误，不显示完整导入成功。

## 用户流程与命名

1. 在 TUI 歌单菜单选择“导入歌单”，填写文件路径或分享链接；支持将纯文本曲目清单粘贴到专用输入框。
2. 展示识别格式、原歌单名、数量和目标名称。名称优先使用用户填写值，其次使用原歌单标题，最后使用文件名。
3. 多歌单备份先展示列表，允许选择一张或全部；自定义单个名称不能默默将多张歌单合并。
4. 同名时明确提供“另存新歌单、合并、替换”。默认保留现有歌单；替换前保存可恢复快照。
5. 展示总条目、已匹配、待确认、未匹配、无效、重复数量。每个条目有唯一处理结果，避免重复计数。
6. 一次导入原子写入本地歌单；未匹配信息一并保留。用户取消不会留下半张歌单。

已实现的主要 CLI 接口：

```bash
wocli playlist import ./通勤.csv --name "每天通勤" --dry-run
wocli playlist import ./歌单.lxmc --name "以前的收藏"
wocli playlist export "每天通勤" --format json --output ./通勤.wocli.json
wocli playlist export "每天通勤" --format csv --output ./通勤.csv
```

`--dry-run` 只解析与生成预览，不写入曲库。网络解析与跨平台搜索在可取消的后台任务中运行，显示当前进度，不阻塞播放控制。CLI 可提供 JSON 结果，TUI 和将来的 MCP 导入工具复用同一实现。

## 数据与匹配

中间模型使用 `ImportedPlaylist` / `ImportedEntry`，与播放用的 `Track` 分开。至少保存：原平台、原歌曲 ID、标题、歌手列表、专辑、时长、可选 ISRC、原分享链接、原始顺序、匹配状态、目标歌曲 ID、候选及失败原因。

- 对来源和目标都可核对的可靠标识优先匹配；同平台 ID 可直接查详情验证。
- 跨平台按歌名、歌手、专辑、时长、版本信息生成候选。ISRC 在双方可获取时可以辅助，不能假定现有酷我搜索会返回该字段。
- Live、伴奏、翻唱、重制等版本信息参与判定；不能仅因为歌名相似就自动选第一个结果。
- 低置信度或多个接近候选进入“待确认”，无结果进入“未匹配”，均保留原始信息以便重试或手动纠正。
- 实际播放继续使用现有本地文件/酷我提供器。能导入其他平台清单，不表示已经实现其他平台的直接音频播放。
- 导出保留原始来源与匹配关系，不将临时音频直链作为长期歌曲标识。

当前采用独立 `playlist_imports` 元数据和 `playlist_backups` 快照表，保留旧 `members` 作为可播放视图，避免迁移影响旧曲库。后续可引入稳定的歌单 ID、独立条目 ID，让名称成为可修改属性。旧歌单迁移保持当前名称和顺序。对现有全局歌曲 ID 的修改必须兼容收藏、队列、历史、歌词和 MCP，不能仅修改导入分支。

## 必须验证的情况

- 使用各播放器真实导出样本作为固定回归数据，并记录软件版本与来源；单元测试默认离线，在线适配器另做人工/独立检查。
- 中文及特殊符号、BOM/CRLF、CSV 引号与换行、带连字符的歌曲名、毫秒/秒时长、损坏文件、空列表。
- 相同数字 ID 来自不同平台、同名不同歌手、Live/录音室版本、多歌手、多歌单、重复条目及原始顺序。
- 文件导入—导出—再次导入的往返一致性，同名冲突、替换回滚、取消及中断恢复。
- 在线歌单分页与数量核对；来源声明 500 首却只返回 100 首时必须报告不完整，不能报 500 首成功。
- 解析体积、解压后体积、条目数和并发有上限；歌单文件中的脚本、插件和设置不参与导入执行。

## 相关后续能力

优先完成导入导出、未匹配修复及歌单备份，再完善歌单多选/批量移动/排序和整单下载。在线歌单同步后做，需明确本地编辑与远端删除的冲突策略，不能默认用远端覆盖本地。

另一个值得独立完善的方向是音源故障冷却：现有实现已有单首换源和源测试，应考虑短时跳过连续失效的源，减少每首歌都重复等待同一个失败源。

## 调研依据

- [LX 单歌单导出实现（固定提交）](https://github.com/lyswhut/lx-music-desktop/blob/ad95d5091c9ed689fa72b5e5c849df65f5a679ce/src/renderer/views/List/MyList/useShare.ts)
- [LX gzip 与 JSON 读写实现（固定提交）](https://github.com/lyswhut/lx-music-desktop/blob/ad95d5091c9ed689fa72b5e5c849df65f5a679ce/src/common/utils/nodejs.ts)
- [MusicFree 备份结构（固定提交）](https://github.com/maotoumao/MusicFree/blob/d118b18b3d0c904400f7eea7bf99c0ceec6c1aee/src/core/backup.ts)
- [MusicFree 平台歌单导入插件协议](https://musicfree.catcat.work/plugin/protocol.html)
- [Apple Music 导出 XML 说明](https://support.apple.com/zh-cn/guide/music/mus27cd5060f/mac)
- [Spotify 账号数据导出说明](https://support.spotify.com/in-en/article/understanding-your-data/)
- [XSPF 标准](https://xspf.org/spec)、[JSPF 格式](https://www.xspf.org/jspf)
- [Soundiiz 导出格式与字段](https://support.soundiiz.com/hc/en-us/articles/38063068900754-How-to-Export-Your-Playlists-and-Favorites-to-a-File)
- [mpv 歌单与 HLS 清单处理说明](https://mpv.io/manual/stable/)

## 当前验证范围

LX 与 MusicFree 回归数据按上述固定提交的官方 TypeScript 结构构造，不含用户真实备份；尚未声称验证其他历史版本。跨平台匹配测试默认离线，区分来源 ID、同名歌曲、Live 版本、多个候选及手动确认。
