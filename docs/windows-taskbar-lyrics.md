# Windows 系统歌词小组件

## 当前方案：展开面板查看歌词

`0.4.11-alpha.2` 使用微软官方 `IWidgetProvider`、`WidgetManager.UpdateWidget` 和 Adaptive Card，在 Windows 11 的 Widgets 面板内显示歌词及播放控制。点击任务栏天气／小组件入口，或按 `Win+W` 展开面板后查看。用户已确认接受这个位置，并要求使用官方接口和系统渲染。

卡片由 Windows Widgets Host 渲染：布局使用 Adaptive Card 标准元素，字体、颜色、按钮及主题由系统处理。歌曲和歌词作为纯文本交给控件，不解释为网页或 Markdown 链接。支持系统提供的小、中、大尺寸。

| 卡片尺寸 | 内容与播放控制 |
| --- | --- |
| 小 | 歌曲名、当前歌词，以及暂停／继续按钮 |
| 中、大 | 歌曲名、歌手、当前歌词，以及上一首、暂停／继续、下一首按钮 |

过长文本会省略，以给按钮保留空间；大卡片显示更多歌词文字。实际换行和字体缩放效果仍需目标 Windows 宿主验收。

此方案采用普通 Adaptive Card Provider，不依赖 Feed Provider 或 HTML/Web Widget 的 EEA 地区能力；清单没有排除中国地区，也不要求修改系统地区。仍需目标 Windows 11 设备具备可用的 Widgets 面板。参考：[普通 Widget Provider](https://learn.microsoft.com/en-us/windows/apps/develop/widgets/widget-providers)、[官方 C# 实现](https://learn.microsoft.com/en-us/windows/apps/develop/widgets/implement-widget-provider-cs)、[注册清单](https://learn.microsoft.com/en-us/windows/apps/develop/widgets/widget-provider-manifest)。

它不会替换任务栏天气文字；关闭面板后不会提供任务栏常驻歌词。这里没有 Explorer Hook、注入、GDI 自绘歌词条或 HTML 页面。旧版自绘任务栏歌词的说明保留在下方供旧版本使用。

## 内容、控制与资源使用

- 卡片活跃时，通过现有本机授权 IPC 每 500ms 读取当前一句歌词；只在内容或状态变化时提交卡片更新。暂停时保留当前歌词和继续播放按钮。
- 卡片停用时清除歌曲内容并停止该卡片的实时轮询；删除时释放实例状态。组件启动后从系统恢复已添加的卡片，并按宿主的激活通知继续同步。
- 停播、老板模式、关闭歌词、切换曲库和后台失联时清除旧歌曲信息。卡片显示明确状态，避免把旧歌词误认为当前播放。
- 上一首、暂停／继续、下一首只发送白名单操作。按钮绑定当前快照；播放器在同一服务事件内核对歌曲代次、播放状态和隐私开关，过期按钮不能操作新歌或退出老板模式。
- 单一后台工作循环、有界通信和可取消超时；不为每个歌词更新创建线程，不复制整份队列或完整 LRC。歌词解析缓存仅在歌词内容变化时更新。
- 配置与诊断位于 `%LOCALAPPDATA%\wocli-music`。诊断只记录状态、计数和错误，不保存歌曲文本或 IPC 授权令牌。

`widgets status --json` 中 `mode` 为 `system-widget-card`，`surface` 为 `widgets-board`。`provider_responding` 只表示同一曲库的新卡片组件近期心跳；旧 Feed 组件的心跳不会被接受。更新接口返回成功或宿主报告激活，都不等于已验证屏幕上的实际显示效果。

## 构建与安装

在有 PowerShell 7.4、Python、.NET 8 SDK、Windows SDK 22621 或更高版本的 Windows 开发环境运行：

```powershell
cargo build --release --locked
python .\scripts\build_native.py --platform windows-x86_64
.\scripts\build_windows_widgets.ps1 -PlayerBinary .\target\release\wocli.exe -NativeBundleDirectory .\build\native\wocli-music-0.4.11-alpha.2-windows-x86_64
```

输出位于 `artifacts/system-widgets-preview/x64`，含匹配的 `wocli.exe`、自包含 .NET 的 `Layout`、未签名 MSIX、微软签名的 Windows App Runtime 框架依赖、校验和与第三方许可。版本为 `0.4.11-alpha.2`，MSIX 版本为 `0.4.11.1`；实际源码提交和部署信息见 `build-info.json`。

组件使用 .NET 8 和 Windows App SDK `2.6.4`。配套框架包为 `Microsoft.WindowsAppRuntime.2`、版本 `2.6.4.0`；构建从实际还原的微软 MSIX 读取依赖身份并校验签名，不沿用 alpha.1 的 1.8 框架包。选择器里的浅色／深色预览图是标明用途的卡片示意，不是实机截图。

这是开发预览注册包，**不能直接双击未签名 MSIX 安装**。先完整解压到固定目录，按产物 `PREVIEW.txt` 安装微软依赖，并在已有开发者模式的环境开发注册 `Layout/AppxManifest.xml`。注册后须保留整个 `Layout` 在该路径。构建脚本不会安装证书、开启开发者模式或修改系统地区。正式签名／商店分发尚未完成。

升级前先退出旧 TUI，使用包内播放器执行 `./wocli.exe tray stop` 和 `./wocli.exe daemon stop`，避免旧托盘和旧后台继续处理请求。然后使用包内配套播放器：

```powershell
.\wocli.exe widgets on
.\wocli.exe play "歌名 歌手"
.\wocli.exe widgets status --json
```

按 `Win+W`，在面板的小组件选择器中找到并添加 **wocli 歌词**。组件不会自动播放音乐或启动音频后台。选择器和按钮名称可能随 Windows Widgets 版本变化。

关闭歌词：`./wocli.exe widgets off`。卸载开发注册的方法见 `PREVIEW.txt`；共享配置目录不随组件卸载删除。组件一次只连接一个曲库，用相应 `--home` 再执行 `widgets on` 可以切换。曲库的 `windows-lyrics-system` 标记持续禁用旧自绘条，关闭组件或切换曲库也不会重新显示旧条。

从 alpha.1 升级时按新包 `PREVIEW.txt` 处理旧开发注册，再注册新 `Layout` 并添加卡片。alpha.1 的 Feed 启用开关不能替代新卡片的添加步骤。稳定版 v0.4.10 不具备新组件 IPC；请使用 alpha.2 包中的播放器和组件配套测试。

## 验证范围

自动验证包括卡片数据和动作白名单、暂停／停播／老板模式、配置切换、迟到通信、取消、尺寸与实例生命周期、Rust 原子动作守卫，以及 COM 接口和 MSIX 清单。Windows CI 在临时 runner 上实际开发注册包，再以包身份检查本地服务器 COM 代理；测试不调用真实 Widgets 面板。

真实 Windows 11 Widgets 宿主的添加／移除、浅深主题、高对比度、中文长歌词、DPI、面板反复开关、休眠、组件重启和长时间内存占用仍须实机验收。CI 的编译、包注册和 COM 自测不能替代屏幕显示验收，因此本次以开发预览发布，不声称所有 Windows 环境已验证。

## 历史 alpha.1：Feed 公告预览

alpha.1 曾尝试通过 `FeedManager.TryShowAnnouncement` 在任务栏入口请求公告。微软的 [Feed Provider 文档](https://learn.microsoft.com/en-us/windows/apps/develop/feeds/feed-providers) 明确该能力仅面向 EEA；[公告规则](https://learn.microsoft.com/en-us/windows/apps/develop/feeds/feed-provider-announcements) 还限制显示次数和时长。因此它不适用于用户确认的中国地区，也不能承诺常驻逐句歌词。

alpha.2 已改为上述官方 Adaptive Card 面板卡片。不能将旧公告测试结果当作新卡片的实机验收，也不能将 Feed 地区限制泛化到普通 Adaptive Card 小组件。

## v0.4.10 旧实现：开启与诊断

```powershell
wocli tray start
wocli lyrics-bar on
wocli play "歌名 歌手"
wocli tray status
wocli tray status --json
```

`lyrics_surface` 提供 `mode: taskbar-child`、`attached`、`visible` 与 `note`。`attached` 表示窗口确实挂在任务栏父窗口下，`visible` 表示已通过当前布局检查并请求显示；任务栏自动隐藏或被全屏窗口遮住时，不代表屏幕上一定能看到它。

更新后如旧托盘仍在运行，先运行 `wocli tray stop`，再运行 `wocli tray start`。歌词只在播放或暂停时显示；停止、关闭歌词和老板模式会隐藏窗口。无歌词时可显示歌曲和歌手。

- 宽度足够时显示上一首、播放/暂停、下一首按钮。
- 空间较窄时收起按钮，保留歌词文字；右键菜单仍可控制播放。
- 双击歌词文字打开 TUI。嵌入窗口不再作为桌面窗口任意拖拽。
- 控制按钮使用实体背景，整个按钮面都可点击；双击控制按钮不会打开终端。打开的控制终端沿用托盘的 `--home` 曲库目录，路径包含空格或特殊字符也无需经过命令解释器。
- 托盘菜单可分别增加或降低音量，到达 100% 后继续增加会保持 100%。连续调整音量、模式或歌词开关按后台最新状态依次执行。
- 当前覆盖主任务栏的横向布局；竖向任务栏暂时隐藏，多任务栏选择尚未提供。

## 旧自绘实现参考 TaskbarLyrics 的方法

核对版本：[ANYNC/TaskbarLyrics `71262b4`](https://github.com/ANYNC/TaskbarLyrics/tree/71262b42aaf0879d2a805b424107a95fbfd50e5a)。

1. [EmbeddedTaskbarAnchor.cs](https://github.com/ANYNC/TaskbarLyrics/blob/71262b42aaf0879d2a805b424107a95fbfd50e5a/TaskbarLyrics.App/EmbeddedTaskbarAnchor.cs) 使用 `SetParent` 与 `WS_CHILD` 挂接真实任务栏窗口，验证父窗口，去除浮窗置顶状态；任务栏目标变化后重新建立表面。
2. [TaskbarOccupiedSpaceProvider.cs](https://github.com/ANYNC/TaskbarLyrics/blob/71262b42aaf0879d2a805b424107a95fbfd50e5a/TaskbarLyrics.App/TaskbarOccupiedSpaceProvider.cs) 在后台查询 Explorer 自己的 UI Automation 控件，避免遍历嵌入的第三方界面；查询节流，并按开始时间判断快照是否过期。
3. [TaskbarFreeSpaceLayout.cs](https://github.com/ANYNC/TaskbarLyrics/blob/71262b42aaf0879d2a805b424107a95fbfd50e5a/TaskbarLyrics.App/TaskbarFreeSpaceLayout.cs) 合并占用区间，从剩余空间选择位置，允许缩短，空间不足时隐藏。

wocli 参考了挂接、后台检测、快照失效和空白区域布局的思路，使用自己的 Rust/Win32 实现。参考项目主版本使用 WPF/WebView2 绘制内容；wocli 保留 Win32/GDI 绘制，不增加 .NET 或 WebView2 运行依赖。它们均不是由 Windows 接收歌词并自动生成歌词控件。参考项目的 [MIT 许可证](licenses/TaskbarLyrics-MIT.txt) 随 Windows 发行包附带。

## 旧自绘实现的具体处理

- 创建隐藏分层窗口，先设置 `WS_CHILD` 并剥离 `WS_POPUP` / `WS_EX_TOOLWINDOW`，再调用 `SetParent` 挂接并核对父窗口，使用 `SWP_FRAMECHANGED` 应用样式变化。只有挂接和定位成功后才显示，失败时销毁或隐藏，不创建桌面置顶回退。
- 经典任务栏识别 `ReBarWindow32` / `WorkerW` 与任务按钮容器，暂时缩小按钮容器以预留空间，并回读实际矩形，确认 Explorer 接受调整。若只接受部分尺寸变化，先尝试回滚再隐藏；隐藏和退出时也恢复空间。如果 Explorer 或其他应用已再次修改布局，不覆盖它们的新布局。
- 现代任务栏识别 XAML 容器，挂在 `Shell_TrayWnd` 下。通过后台 UI Automation 在查询条件中直接过滤交互控件（按钮、列表项等），将占用区间合并并留出间距，在无经典托盘窗口（Windows 11）下自动在应用按钮与系统托盘控件之间寻址空白空间。
- 最多一个后台查询，约 500ms 检测；快照按查询开始时间计龄，最多使用两秒。句柄、矩形或查询代次不同的结果会被丢弃。关闭歌词时停止查询；空间不足、读取超时或边界信息不足时保持隐藏，恢复后重新尝试。UI Automation 的部分结果无法严格证明控件树完整，仍需实机验证避让效果。
- 使用任务栏 DPI 与父窗口客户区坐标；处理负屏幕坐标、缩放、镜像父窗口和左侧通知区。收到 Explorer 的 `TaskbarCreated` 或显示设置变化后，将重建延后到消息循环处理，避免同步窗口通知重入当前布局更新。
- 状态轮询和控制命令在独立串行线程执行，UI 消息线程不等待后台响应。超过两秒没有新鲜状态时隐藏歌词并清除旧歌曲提示；后台恢复后继续更新。
- 右键菜单打开期间不持有共享状态借用，避免嵌套消息循环导致崩溃。“退出播放器”先请求后台退出，成功后正常销毁窗口并恢复任务栏；请求失败保留托盘供重试，不在托盘进程里强杀自身。
- Windows 可执行文件携带 Windows 10/11 兼容性与 PerMonitorV2 DPI 声明，以支持透明分层子窗口。

## 旧自绘实现的兼容性与验证边界

旧自绘实现依赖 Explorer 窗口结构及 UI Automation；Windows 更新、第三方任务栏或安全软件可能影响挂接，失败原因见 `tray status`。当前分支的 Adaptive Card 小组件是另一条集成路径，显示在展开的 Widgets 面板内，不使用窗口挂接。

自动测试覆盖空白区域避让、图标占满、窄窗口、DPI、负坐标、迟到快照，以及 Windows 原生隔离窗口的父子关系、窗口替换、无置顶状态和空间恢复。隔离窗口测试不能替代真实 Windows 10/11 Explorer、多屏及混合 DPI 的实机验收；不据此宣称所有任务栏版本都已验证。

2026-10-29 本轮检查：macOS 上 `cargo test --locked --offline --all-targets` 的 152 项测试通过，截图导出测试单独通过，`cargo fmt --check` 与 `cargo clippy --locked --offline --all-targets -- -D warnings` 通过。另用隔离工程直接引入真实 `tray.rs`、`ipc.rs`、`lyrics.rs` 及其任务栏子模块，以 `x86_64-pc-windows-msvc` 检查生产代码和测试代码，类型检查与 Clippy 通过；隔离工程仅替代业务层依赖，不替代 Win32 API。该检查没有执行 Windows 链接、Windows 专属测试或真实 Explorer 验收。

官方依据：[SetParent](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent)、[子窗口与透明分层窗口](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features)、[Windows 11 任务栏扩展限制](https://www.microsoft.com/en-us/windows/windows-11-specifications)。

## 旧自绘实现的 Windows 实机验收

在 Windows 10 经典任务栏和 Windows 11 默认任务栏分别执行以下检查；Windows 交叉编译和隔离窗口测试不能替代这些步骤。

| 场景 | 预期结果 |
| --- | --- |
| 播放、暂停、停止、无歌词歌曲 | 歌词/标题与播放状态对应，停止后隐藏 |
| 右键歌词，停留菜单，再切歌或退出 | 菜单可正常操作和关闭，进程不崩溃，退出后经典按钮区域恢复 |
| 后台无响应、重启后台 | 窗口仍可响应；过期歌词隐藏，连接恢复后重新显示 |
| 快速连续调整音量、模式、歌词开关 | 操作依次生效；音量上下界为 0–100，不循环跳到其他值 |
| 增加任务栏图标，直到空白不足 | 按钮收起或歌词隐藏，不覆盖系统图标 |
| 自动隐藏、重启 Explorer、更改显示设置 | 歌词跟随任务栏，重新挂接成功后恢复，不产生桌面置顶浮窗 |
| 100% / 150% / 200% 缩放，负坐标副屏 | 原生字体、按钮命中与实际位置一致；当前只选择主任务栏 |
| 老板模式、设备重连、暂停时重试 | 伪装期间隐藏歌曲信息；原本暂停的音频不会因迟到回调而恢复 |
| 自定义 `--home`、程序或目录包含空格 | 双击文字或菜单打开的 TUI 仍连接同一曲库 |

相关 API 说明：[矩形与镜像坐标转换](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-mapwindowpoints)、[窗口尺寸与样式刷新](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos)、[托盘菜单消息处理](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-trackpopupmenu)。
