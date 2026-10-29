# Windows 任务栏嵌入歌词

现行 v0.4.10 将歌词放在通知区域旁的任务栏空白中，随任务栏显示、隐藏和移动。它不使用桌面置顶悬浮条作为回退。

这是 Rust 调用 Win32 创建子窗口、GDI 自绘文字与按钮的方案，不依赖浏览器、WebView2 或 .NET。它不是 Windows Widgets 系统组件；现有代码自行指定颜色、字体和按钮绘制，因此不能把“原生 Win32 窗口”视为“系统负责渲染和样式”。

## 新目标与官方能力

2026-10-29 明确的新要求是：歌词显示于 Windows 11 左下角天气／小组件入口位置，使用系统组件与系统样式；字体、颜色、主题及交互状态由系统负责，不采用应用自绘。现行版本的右侧空白区定位和 GDI 绘制均不满足此要求。下面的兼容性测试只验证现行实现，不代表新目标已经完成；仅移动自绘窗口或修改颜色不能作为该问题的修复。

用户回报版本“应该是 25H”，随后补充“2”，暂按 Windows 11 25H2 评估；用户已明确“国家或地区”为中国。此前将“2”理解为选择欧洲经济区有误，现予以纠正。准确内部版本号及设备实际能力尚未实机核实。

**中国地区不在微软当前 Feed Provider 的公开支持范围内，下面的系统组件预览不能作为该设备的修复方案。** Windows 11 25H2 本身不解除地区限制；即便在受支持设备上，公告也不保证常驻或逐句实时更新。当前没有经过验证、同时满足该设备地区、左下角天气入口和系统渲染要求的方案，新目标仍未完成。

官方能力核查：

- 普通 [Widget Provider](https://learn.microsoft.com/en-us/windows/apps/design/widgets/) 将模板和数据交给 Widgets Board 显示，并不是任务栏天气入口的实时文字槽位。
- [Windows App SDK 1.6 的历史 experimental2 发布说明](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-notes/windows-app-sdk-1-6) 曾列出 `WidgetAnnouncement` 和 `WidgetManager.TryShowAnnouncement`。但当前稳定版的 [WidgetManager 成员表](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.widgets.providers.widgetmanager?view=windows-app-sdk-2.0) 与 [Notifications 类型表](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.widgets.notifications?view=windows-app-sdk-2.0) 未提供这条普通 Widget 公告路径，不能仅凭历史预览条目承诺支持。
- [Feed Provider Announcement](https://learn.microsoft.com/en-us/windows/apps/develop/feeds/feed-provider-announcements) 可请求由系统显示任务栏主／副文本；使用 [AnnouncementTextColor.Default](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.widgets.notifications.announcementtextcolor?view=windows-app-sdk-2.0) 可采用系统颜色。系统决定排队、优先级和是否显示，并设有每日公告及徽标次数限制。
- 公告的 [Duration](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.widgets.notifications.feedannouncement.duration?view=windows-app-sdk-2.0) 受平台时长上下限约束，不能据此承诺歌词持续常驻或每句实时更新。
- 当前 [Feed Provider 文档](https://learn.microsoft.com/en-us/windows/apps/develop/feeds/feed-providers) 仍声明该能力处于预览且仅面向欧洲经济区；接入公告还要求 MSIX 打包身份。需要核对目标 Windows 版本、地区及实际可用性，不能直接以现有免安装 EXE 宣称完成系统组件接入。

地区限制针对 Feed Provider，不应推广为所有第三方 Widgets 的限制。微软的 [DMA 说明](https://blogs.windows.com/windows-insider/2023/11/16/previewing-changes-in-windows-to-comply-with-the-digital-markets-act-in-the-european-economic-area/) 指出相关资格使用设备初次设置时选择的地区；当前“国家或地区”设置只能作为诊断线索，不能单独证明接口可用，也不应要求用户修改地区或重置电脑来接入歌词功能。

## 系统组件预览（开发分支）

`codex/windows-system-widgets` 增加独立的 MSIX Feed Provider。任务栏文字通过 `FeedManager.TryShowAnnouncement` 提交，使用 `AnnouncementTextColor.Default`；运行时不创建自绘任务栏窗口、不修改 Explorer 或天气入口。Windows 决定展示位置、文字排版、主题及可见时长。启用配置和收到请求成功均不等于系统实际显示；`Shown` 回调仅表示曾经显示，也不等于现在仍可见。

这是一项仅供具备相关能力的欧洲经济区设备评估的预览，**不适用于中国地区，也不满足“常驻、每句实时同步”的完整目标**。它每 500ms 从本地后台读取当前歌词，最多每 5 秒请求一次最新内容，不积压漏过的句子，不使用徽标。5 秒是本项目的节流值，不是微软公布的配额；系统仍可能限制或拒绝公告。暂停、切歌、停止、老板模式、关闭开关、后台失联和宿主禁用都会影响请求或触发移除；平台移除接口同样需要真实宿主验收。

开发分支包含：

- Rust `lyrics_snapshot` 只返回当前一句、歌曲元数据和播放状态；不复制整份播放队列、下载记录或完整歌词。解析后的歌词仅在内容变化时更新缓存，老板模式、关闭歌词和停止时清空对外文本。
- 独立 C#/.NET Feed Provider 使用官方 Windows App SDK COM 接口，与播放器通过已有本机 TCP 授权协议通信，不启动音频后台。连接、文件大小、回复长度及取消均有边界。
- 点击公告后打开的 Feed 面板由 Widgets 宿主中的网页承载，采用系统字体和语义颜色。它不是任务栏的绘制层，也不是 Adaptive Card。`https://wocli.invalid/lyrics` 通过资源回调返回本地内容，不连接外网；HTML 转义歌曲文字。
- 配置与不含歌曲文本的心跳诊断保存在 `%LOCALAPPDATA%\wocli-music`。MSIX 仅为此目录排除 AppData 写入虚拟化，保证未打包的播放器与组件读写同一份文件；卸载组件不会自动删除该共享目录。

### 构建与试用

以下注册与启用步骤仅用于符合 Feed Provider 支持条件的开发测试。中国地区用户不应为解决天气入口歌词问题安装此预览；开发注册成功和 COM 测试通过均不能解除宿主的地区限制。

在有 PowerShell 7.4、Python、.NET 8 SDK、Windows SDK 22621 或更高版本的 Windows 开发环境运行：

```powershell
cargo build --release --locked
python .\scripts\build_native.py --platform windows-x86_64
.\scripts\build_windows_widgets.ps1 -PlayerBinary .\target\release\wocli.exe -NativeBundleDirectory .\build\native\wocli-music-0.4.11-alpha.1-windows-x86_64
```

输出位于 `artifacts/system-widgets-preview/x64`，含同一分支的 `wocli.exe`、自包含 .NET 的 `Layout`、未签名 MSIX、微软 Windows App Runtime 依赖包、校验和与构建信息。Windows App SDK 使用官方框架依赖方式：所选 SDK 的自包含清单明确未包含已验证的 Widgets 代理注册，不能据此保证宿主跨进程通信。稳定版 v0.4.10 不含新命令及 IPC；预览播放器明确标为 `0.4.11-alpha.1`，实际源码提交号见 `build-info.json`。运行库会增加包体积，实际大小由构建脚本记录。

脚本只构建和检查，不安装证书、不启用开发者模式、不修改系统地区或小组件设置。使用开发注册前，按产物中的 `PREVIEW.txt` 操作；注册后的 `Layout` 必须留在固定路径。普通双击安装未签名 MSIX 不可用，正式签名分发尚未完成。

基础 `--self-test` 检查进程内 COM 类工厂和三个接口；它不具备包身份，不能验证框架包的代理注册。CI 另在临时 Windows runner 上安装依赖并开发注册完整 `Layout`，以真实包身份运行 `--self-test-packaged-proxy`，验证临时 COM 类的本地服务器代理激活。此测试不会调用 FeedManager 或 Explorer；不代表真实 Widgets 宿主自动激活正式组件或显示公告已通过。仅 CI 专用脚本执行安装和临时开发模式设置，并在结束后卸载预览注册、还原设置；普通构建脚本不执行这些操作。

安装并在系统 Widgets 设置启用 wocli Feed 后，使用预览播放器：

```powershell
.\wocli.exe widgets on
.\wocli.exe widgets status --json
.\wocli.exe play "歌名 歌手"
.\wocli.exe widgets off
```

`widgets on` 为当前 `--home` 选择系统模式并打开歌词总开关；该曲库的旧 GDI 歌词条停止显示。曲库目录内的 `windows-lyrics-system` 标记持久保存这一选择，切换至其他曲库或删除共享配置也不会重新显示旧自绘条。`off` 关闭当前曲库的歌词；若组件已切换至其他曲库，不改变其开关。组件一次只连接一个选定曲库；切换时用相应 `--home` 再执行 `widgets on`。缺少组件、系统不支持或公告未获显示时，保持无自绘回退。

`provider_responding` 只验证同一 endpoint 的近期心跳；`diagnostics.requested_count` 与 `shown_count` 区分请求和宿主展示回调。检查不会假装确认屏幕上可见。正式验收需在目标 Windows 11 的真实 Widgets 宿主验证：浅/深主题、停播与老板模式移除、逐次切歌、配额触发、重启 Explorer、禁用 Feed、组件退出和长时间内存占用。构建、COM 自测和单元测试无法替代这些验收。

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

## 参考 TaskbarLyrics 的方法

核对版本：[ANYNC/TaskbarLyrics `71262b4`](https://github.com/ANYNC/TaskbarLyrics/tree/71262b42aaf0879d2a805b424107a95fbfd50e5a)。

1. [EmbeddedTaskbarAnchor.cs](https://github.com/ANYNC/TaskbarLyrics/blob/71262b42aaf0879d2a805b424107a95fbfd50e5a/TaskbarLyrics.App/EmbeddedTaskbarAnchor.cs) 使用 `SetParent` 与 `WS_CHILD` 挂接真实任务栏窗口，验证父窗口，去除浮窗置顶状态；任务栏目标变化后重新建立表面。
2. [TaskbarOccupiedSpaceProvider.cs](https://github.com/ANYNC/TaskbarLyrics/blob/71262b42aaf0879d2a805b424107a95fbfd50e5a/TaskbarLyrics.App/TaskbarOccupiedSpaceProvider.cs) 在后台查询 Explorer 自己的 UI Automation 控件，避免遍历嵌入的第三方界面；查询节流，并按开始时间判断快照是否过期。
3. [TaskbarFreeSpaceLayout.cs](https://github.com/ANYNC/TaskbarLyrics/blob/71262b42aaf0879d2a805b424107a95fbfd50e5a/TaskbarLyrics.App/TaskbarFreeSpaceLayout.cs) 合并占用区间，从剩余空间选择位置，允许缩短，空间不足时隐藏。

wocli 参考了挂接、后台检测、快照失效和空白区域布局的思路，使用自己的 Rust/Win32 实现。参考项目主版本使用 WPF/WebView2 绘制内容；wocli 保留 Win32/GDI 绘制，不增加 .NET 或 WebView2 运行依赖。它们均不是由 Windows 接收歌词并自动生成歌词控件。参考项目的 [MIT 许可证](licenses/TaskbarLyrics-MIT.txt) 随 Windows 发行包附带。

## wocli 的具体处理

- 创建隐藏分层窗口，先设置 `WS_CHILD` 并剥离 `WS_POPUP` / `WS_EX_TOOLWINDOW`，再调用 `SetParent` 挂接并核对父窗口，使用 `SWP_FRAMECHANGED` 应用样式变化。只有挂接和定位成功后才显示，失败时销毁或隐藏，不创建桌面置顶回退。
- 经典任务栏识别 `ReBarWindow32` / `WorkerW` 与任务按钮容器，暂时缩小按钮容器以预留空间，并回读实际矩形，确认 Explorer 接受调整。若只接受部分尺寸变化，先尝试回滚再隐藏；隐藏和退出时也恢复空间。如果 Explorer 或其他应用已再次修改布局，不覆盖它们的新布局。
- 现代任务栏识别 XAML 容器，挂在 `Shell_TrayWnd` 下。通过后台 UI Automation 在查询条件中直接过滤交互控件（按钮、列表项等），将占用区间合并并留出间距，在无经典托盘窗口（Windows 11）下自动在应用按钮与系统托盘控件之间寻址空白空间。
- 最多一个后台查询，约 500ms 检测；快照按查询开始时间计龄，最多使用两秒。句柄、矩形或查询代次不同的结果会被丢弃。关闭歌词时停止查询；空间不足、读取超时或边界信息不足时保持隐藏，恢复后重新尝试。UI Automation 的部分结果无法严格证明控件树完整，仍需实机验证避让效果。
- 使用任务栏 DPI 与父窗口客户区坐标；处理负屏幕坐标、缩放、镜像父窗口和左侧通知区。收到 Explorer 的 `TaskbarCreated` 或显示设置变化后，将重建延后到消息循环处理，避免同步窗口通知重入当前布局更新。
- 状态轮询和控制命令在独立串行线程执行，UI 消息线程不等待后台响应。超过两秒没有新鲜状态时隐藏歌词并清除旧歌曲提示；后台恢复后继续更新。
- 右键菜单打开期间不持有共享状态借用，避免嵌套消息循环导致崩溃。“退出播放器”先请求后台退出，成功后正常销毁窗口并恢复任务栏；请求失败保留托盘供重试，不在托盘进程里强杀自身。
- Windows 可执行文件携带 Windows 10/11 兼容性与 PerMonitorV2 DPI 声明，以支持透明分层子窗口。

## 兼容性与验证边界

现行自绘实现依赖 Explorer 窗口结构及 UI Automation；Windows 更新、第三方任务栏或安全软件可能影响挂接，失败原因见 `tray status`。上述系统公告接口是另一条集成路径，不应与此窗口挂接实现混为一谈；其公开契约也不保证歌词在指定位置持续实时显示。

自动测试覆盖空白区域避让、图标占满、窄窗口、DPI、负坐标、迟到快照，以及 Windows 原生隔离窗口的父子关系、窗口替换、无置顶状态和空间恢复。隔离窗口测试不能替代真实 Windows 10/11 Explorer、多屏及混合 DPI 的实机验收；不据此宣称所有任务栏版本都已验证。

2026-10-29 本轮检查：macOS 上 `cargo test --locked --offline --all-targets` 的 152 项测试通过，截图导出测试单独通过，`cargo fmt --check` 与 `cargo clippy --locked --offline --all-targets -- -D warnings` 通过。另用隔离工程直接引入真实 `tray.rs`、`ipc.rs`、`lyrics.rs` 及其任务栏子模块，以 `x86_64-pc-windows-msvc` 检查生产代码和测试代码，类型检查与 Clippy 通过；隔离工程仅替代业务层依赖，不替代 Win32 API。该检查没有执行 Windows 链接、Windows 专属测试或真实 Explorer 验收。

官方依据：[SetParent](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setparent)、[子窗口与透明分层窗口](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features)、[Windows 11 任务栏扩展限制](https://www.microsoft.com/en-us/windows/windows-11-specifications)。

## Windows 实机验收

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
