# Windows 任务栏嵌入歌词

目标是将歌词放在通知区域旁的任务栏空白中，随任务栏显示、隐藏和移动。它不使用桌面置顶悬浮条作为回退。

这是 Rust 调用 Win32 创建子窗口、GDI 绘制文字与按钮的原生方案，不依赖浏览器、WebView2 或 .NET。Windows 本身没有负责接收和绘制歌词的标准任务栏控件；“原生绘制”和“系统官方歌词接口”是不同能力，后者不能作为本项目的实现前提。

## 开启与诊断

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

Microsoft 没有为 Windows 11 提供向任意任务栏区域填入歌词文字的公开专用接口。此实现依赖 Explorer 窗口结构及 UI Automation；Windows 更新、第三方任务栏或安全软件可能影响挂接，失败原因见 `tray status`。

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
