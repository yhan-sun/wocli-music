use crate::{
    ipc, lyrics,
    model::{clock, normalize_id, Error, Result, Track},
    provider::Provider,
    service::{self, Status},
    store::{Paths, Store},
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, IsTerminal, Write},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

pub const HELP: &str = "wocli — 原生终端音乐播放器

用法：wocli [命令] [参数]
  tui                               打开终端界面（默认）
  search 关键词 [--page 1 --limit 20] 搜索歌曲
  play 关键词 [--pick N]             选择并播放
  play --id ID [--quality best]      按 ID 播放
  play /path/song.flac               播放本地音频
  pause | resume | toggle | stop     播放控制（支持 play-pause）
  next | prev | seek 秒数            切歌与跳转（支持 skip / previous，--no-wait）
  volume 0-100 | mode 播放方式        list / repeat / single / shuffle
  status | favorites | history       状态、喜欢、历史
  daily [play|add|dislike|undo]      每日推荐
  queue add|list|remove|clear         管理队列
  playlist create|list|show|add|remove|rename|delete|play
  playlist import|export|scan|inspect|resolve|match|drop|backups|restore
  like [--id ID]                     喜欢或取消喜欢
  lyrics [--id ID --refresh --output song.lrc]
  lyrics-bar [on|off|toggle|status]  任务栏 / 状态栏实时歌词控制
  widgets on|off|status             Windows 小组件面板歌词卡片（预览）
  boss [on|off|toggle]               老板模式（暂停并隐藏歌词、歌曲信息）
  tray start|stop|status|run         系统托盘小图标控制服务
  download [--id ID | 关键词 --pick N] [--quality best --lyrics --output 目录]
  retry [--quality best]             重试当前歌曲及备用音源
  source list|add|show|enable|disable|prefer|remove|test  音源管理
  help [命令] | thelp [命令]         分项帮助（keys 查看快捷键）
  mcp [serve|prompt|config|tools]         MCP 服务、AI 接入提示词及配置
  config [配置项 值]                 quality / volume / mode / direct / lyric_offset / lyrics_bar / tray
  update [--check] [--force]         检查或安装最新稳定版
  doctor | daemon start|stop|status [--tray]  诊断及后台服务

环境变量：
  WOCLI_HOME         数据与配置目录（默认 ~/.config/wocli-music）
  WOCLI_RUNTIME_DIR  运行时 socket 目录（默认 /tmp/wocli-rust-{uid}-{hash}）
  WOCLI_DOWNLOAD_DIR 默认下载目录（默认 ~/Music/wocli 或 downloads）
  WOCLI_AUDIO_OUTPUT 音频输出（设为 null 可在无音频设备环境静音运行）
  HTTP_PROXY / HTTPS_PROXY / ALL_PROXY / NO_PROXY 网络代理配置

选项：
  --json 输出 JSON；--direct 忽略环境代理；--tray 启动托盘小图标；--no-wait 异步切歌；--version 查看版本。
";
struct Args {
    pos: Vec<String>,
    values: HashMap<String, String>,
    flags: HashSet<String>,
}
impl Args {
    fn parse(raw: Vec<String>) -> Result<Self> {
        let mut args = Self {
            pos: vec![],
            values: HashMap::new(),
            flags: HashSet::new(),
        };
        let mut iter = raw.into_iter();
        while let Some(value) = iter.next() {
            if value == "--" {
                args.pos.extend(iter);
                break;
            }
            if [
                "--id",
                "--pick",
                "--quality",
                "--output",
                "--page",
                "--limit",
                "--home",
                "--name",
                "--format",
                "--text-order",
                "--on-conflict",
                "--list",
                "--entry",
            ]
            .contains(&value.as_str())
            {
                args.values.insert(
                    value.clone(),
                    iter.next()
                        .ok_or_else(|| Error::new(format!("{value} 缺少参数")))?,
                );
            } else if [
                "--json",
                "--direct",
                "--force",
                "--lyrics",
                "--refresh",
                "--no-wait",
                "--version",
                "--help",
                "-h",
                "--next",
                "--tray",
                "--check",
                "--read-only",
                "--dry-run",
                "--offline",
            ]
            .contains(&value.as_str())
            {
                args.flags.insert(value);
            } else if value.starts_with("--") {
                return Err(Error::new(format!("未知选项：{value}")));
            } else {
                args.pos.push(value);
            }
        }
        Ok(args)
    }
    fn flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }
    fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
    fn arg(&self, index: usize) -> Result<&str> {
        self.pos
            .get(index)
            .map(String::as_str)
            .ok_or_else(|| Error::new("命令缺少参数，请使用 --help"))
    }
    fn integer(&self, key: &str, default: usize) -> Result<usize> {
        self.value(key)
            .map(|v| {
                v.parse()
                    .map_err(|_| Error::new(format!("{key} 应为正整数")))
            })
            .unwrap_or(Ok(default))
    }
}
pub fn expand_path(value: &str) -> PathBuf {
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .unwrap_or_default();
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(value)
    }
}
fn by_id(store: &Store, provider: &Provider, id: &str) -> Result<Track> {
    let id = if id.starts_with("local:") {
        id.into()
    } else {
        normalize_id(id)?
    };
    if let Some(track) = store.track(&id)? {
        return Ok(track);
    }
    if id.starts_with("local:") {
        return Err(Error::new("本地歌曲不在曲库中，请使用文件路径"));
    }
    let track = provider.details(&id)?;
    store.remember(std::slice::from_ref(&track))?;
    Ok(track)
}
fn choose(args: &Args, start: usize, store: &Store, provider: &Provider) -> Result<Track> {
    if let Some(id) = args.value("--id") {
        return by_id(store, provider, id);
    }
    let keyword = args.pos.get(start..).unwrap_or_default().join(" ");
    if keyword.is_empty() {
        return Err(Error::new("请提供歌名、--id 或本地音频路径"));
    }
    let path = expand_path(&keyword);
    if path.is_file() {
        let track = Track::local(&path)?;
        store.remember(std::slice::from_ref(&track))?;
        return Ok(track);
    }
    let (_, tracks) = provider.search(&keyword, 0, 20)?;
    store.remember(&tracks)?;
    if tracks.is_empty() {
        return Err(Error::new("没有找到歌曲"));
    }
    let pick = if let Some(value) = args.value("--pick") {
        value
            .parse::<usize>()
            .map_err(|_| Error::new("--pick 应为正整数"))?
    } else if io::stdin().is_terminal() && !args.flag("--json") {
        table(&tracks);
        print!("播放第几首 [1]：");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if input.trim().is_empty() {
            1
        } else {
            input
                .trim()
                .parse()
                .map_err(|_| Error::new("请输入歌曲序号"))?
        }
    } else {
        return Err(Error::new("请用 --pick N 或 --id 明确选择歌曲"));
    };
    pick.checked_sub(1)
        .and_then(|i| tracks.get(i))
        .cloned()
        .ok_or_else(|| Error::new("歌曲序号超出结果范围"))
}
fn table(tracks: &[Track]) {
    for (i, track) in tracks.iter().enumerate() {
        println!(
            "{:>3}  {} · {}  {}  {}",
            i + 1,
            track.title,
            track.artist,
            clock(track.duration),
            track.id
        );
    }
}
fn request(paths: &Paths, value: Value) -> Result<Value> {
    ipc::request(paths, value, true)?.ok_or_else(|| Error::new("后台服务未运行"))
}
fn wait(paths: &Paths) -> Result<Value> {
    let start = Instant::now();
    loop {
        let value = request(paths, json!({"op":"status"}))?;
        match value["phase"].as_str() {
            Some("loading") => {}
            Some("error") => {
                return Err(Error {
                    message: value["error"].as_str().unwrap_or("播放失败").into(),
                    code: value["error_code"].as_str().unwrap_or("error").into(),
                    details: serde_json::from_value(value["error_details"].clone())
                        .unwrap_or_default(),
                })
            }
            _ => return Ok(value),
        }
        if start.elapsed() > Duration::from_secs(360) {
            return Err(Error::new("等待音频加载超时"));
        }
        thread::sleep(Duration::from_millis(40));
    }
}
fn print_status(value: &Value) {
    if value.is_null() {
        println!("后台未运行");
        return;
    }
    let phase = match value["phase"].as_str().unwrap_or_default() {
        "playing" => "播放中",
        "paused" => "已暂停",
        "loading" => "加载中",
        "error" => "播放失败",
        _ => "已停止",
    };
    if value["track"].is_object() {
        println!(
            "{phase}  {} · {}  {} / {}  {}",
            value["track"]["title"].as_str().unwrap_or_default(),
            value["track"]["artist"].as_str().unwrap_or_default(),
            clock(value["position"].as_f64().unwrap_or(0.)),
            clock(value["duration"].as_f64().unwrap_or(0.)),
            value["quality"].as_str().unwrap_or_default()
        );
    } else {
        println!("{phase}");
    }
    if value["buffering"] == true {
        println!("音频缓冲中，等待供数；播放进度暂时停留在实际播放位置");
    }
}

fn source_command(paths: &Paths, args: &Args) -> Result<()> {
    use crate::sources::{self, SourceKind, Sources};
    use std::sync::{atomic::AtomicBool, Arc};
    let action = args.pos.get(1).map(String::as_str).unwrap_or("list");
    let direct = args.flag("--direct") || paths.settings()?.direct;
    let provider = Provider::new(direct);
    let value = match action {
        "list" => sources::describe(&Sources::load(&paths.home)?),
        "show" => serde_json::to_value(Sources::load(&paths.home)?.get(args.arg(2)?)?)?,
        "add" => serde_json::to_value(sources::add(
            paths,
            args.arg(2)?,
            args.value("--name"),
            provider.agent(),
        )?)?,
        "enable" | "disable" => sources::describe(&sources::enable(
            &paths.home,
            args.arg(2)?,
            action == "enable",
        )?),
        "prefer" => sources::describe(&sources::prefer(&paths.home, args.arg(2)?)?),
        "remove" => sources::describe(&sources::remove(&paths.home, args.arg(2)?)?),
        "test" => {
            let list = Sources::load(&paths.home)?;
            let source = list.get(args.arg(2)?)?;
            let quality = args.value("--quality").unwrap_or("128k");
            if !crate::model::QUALITIES.contains(&quality) {
                return Err(Error::new("音质无效"));
            }
            let track = Track {
                id: normalize_id(args.value("--id").unwrap_or("228908"))?,
                ..Default::default()
            };
            let cancelled = Arc::new(AtomicBool::new(false));
            let stream = if matches!(source.kind, SourceKind::Native) {
                provider.native_stream(&source.id, &track, quality, &cancelled)?
            } else {
                let q = sources::quality_order(quality)
                    .iter()
                    .find(|q| source.qualities.iter().any(|s| s == **q))
                    .ok_or_else(|| Error::new("音源不支持所选音质"))?;
                let mut stream =
                    sources::resolve(source, &paths.home, &track, q, provider.agent(), cancelled)?;
                stream.size = provider.kuwo.probe(&stream.url, 0)?;
                stream
            };
            json!({"source":source.id,"name":source.name,"status":"ok","quality":stream.quality,"bytes":stream.size,"track_id":track.id})
        }
        _ => return Err(Error::new("未知音源操作，运行 wocli help source")),
    };
    if args.flag("--json") {
        println!("{}", serde_json::to_string(&value)?);
    } else if let Some(list) = value["sources"].as_array() {
        for (i, s) in list.iter().enumerate() {
            println!(
                "{:>2}. {}  {}  {}{}",
                i + 1,
                s["id"].as_str().unwrap_or_default(),
                if s["enabled"] == true {
                    "启用"
                } else {
                    "停用"
                },
                s["name"].as_str().unwrap_or_default(),
                if s["builtin"] == true {
                    "（内置）"
                } else {
                    ""
                }
            );
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}
pub fn run(raw: Vec<String>) -> Result<()> {
    let args = Args::parse(raw)?;
    if args.flag("--version") {
        println!("wocli-music {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let command = args.pos.first().map(String::as_str).unwrap_or("tui");
    if args.flag("--help") || args.flag("-h") || matches!(command, "help" | "thelp") {
        let topic = if matches!(command, "help" | "thelp") {
            args.pos.get(1).map(String::as_str)
        } else {
            args.pos.first().map(String::as_str)
        };
        let help = crate::help::text(topic)?;
        if args.flag("--json") {
            println!(
                "{}",
                json!({"topic":topic.unwrap_or("help"),"text":help,"commands":crate::help::COMMANDS})
            );
        } else {
            print!("{help}");
        }
        return Ok(());
    }
    let paths = Paths::discover_opt(args.value("--home"))?;
    if args.flag("--read-only") && command != "mcp" {
        return Err(Error::new("--read-only 仅用于 mcp 命令"));
    }
    if command == "mcp" {
        if args.pos.len() > 2 {
            return Err(Error::new(
                "用法：wocli mcp [serve|prompt|config|tools] [--read-only]",
            ));
        }
        let value = match args.pos.get(1).map(String::as_str).unwrap_or("serve") {
            "serve" => {
                return crate::mcp::run(paths, args.flag("--read-only"), args.flag("--direct"))
            }
            "config" => {
                crate::mcp::config(&paths, args.flag("--read-only"), args.flag("--direct"))?
            }
            "prompt" => {
                let prompt =
                    crate::mcp::prompt(&paths, args.flag("--read-only"), args.flag("--direct"))?;
                if args.flag("--json") {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "prompt": prompt,
                            "config": crate::mcp::config(&paths, args.flag("--read-only"), args.flag("--direct"))?,
                        }))?
                    );
                } else {
                    println!("{prompt}");
                }
                return Ok(());
            }
            "tools" => crate::mcp::tools(args.flag("--read-only"))?,
            _ => return Err(Error::new("未知 MCP 操作，运行 wocli help mcp")),
        };
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    if matches!(command, "source" | "sources") {
        return source_command(&paths, &args);
    }
    if command == "update" {
        let direct = args.flag("--direct") || paths.settings().is_ok_and(|s| s.direct);
        let report =
            crate::updater::run(&paths, args.flag("--check"), args.flag("--force"), direct)?;
        if args.flag("--json") {
            println!("{}", serde_json::to_string(&report)?);
        } else {
            match report.status.as_str() {
                "updated" => println!(
                    "已更新至 v{} · {}",
                    report.latest_version,
                    report.executable.display()
                ),
                "available" => println!(
                    "发现新版本 v{}（当前 v{}），运行 wocli update 安装",
                    report.latest_version, report.current_version
                ),
                _ => println!(
                    "当前 v{} 已是最新稳定版（发行版 v{}）",
                    report.current_version, report.latest_version
                ),
            }
            if !report.warning.is_empty() {
                eprintln!("{}", report.warning);
            }
        }
        return Ok(());
    }
    if command == "__daemon" {
        return service::run(paths);
    }
    if command == "tui" {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(Error::new("终端界面需要真实终端；脚本中请使用 --json 命令"));
        }
        return crate::tui::run(paths);
    }
    let settings = paths.settings()?;
    let provider = Provider::new(args.flag("--direct") || settings.direct);
    let store = Store::open(&paths)?;
    let data = match command {
        "search" => {
            let keyword = args.pos.get(1..).unwrap_or_default().join(" ");
            if keyword.is_empty() {
                return Err(Error::new("请提供搜索关键词"));
            }
            let page = args.integer("--page", 1)?;
            let limit = args.integer("--limit", 20)?;
            if page == 0 || !(1..=100).contains(&limit) {
                return Err(Error::new("页码应大于零，每页 1–100 首"));
            }
            let (total, tracks) = provider.search(&keyword, page - 1, limit)?;
            store.remember(&tracks)?;
            if !args.flag("--json") {
                table(&tracks);
                println!("共 {total} 首 · 第 {page} 页");
            }
            json!({"total":total,"page":page,"tracks":tracks})
        }
        "play" => {
            let tracks = if args.value("--id").is_some() || args.pos.len() > 1 {
                vec![choose(&args, 1, &store, &provider)?]
            } else {
                vec![]
            };
            let value = request(
                &paths,
                json!({"op":"play","tracks":tracks,"quality":args.value("--quality"),"direct":args.flag("--direct")}),
            )?;
            let value = if args.flag("--no-wait") {
                value
            } else {
                wait(&paths)?
            };
            if !args.flag("--json") {
                print_status(&value);
            }
            value
        }
        "retry" => {
            let value = request(
                &paths,
                json!({"op":"retry","quality":args.value("--quality"),"direct":args.flag("--direct") || settings.direct}),
            )?;
            let value = if args.flag("--no-wait") {
                value
            } else {
                wait(&paths)?
            };
            if !args.flag("--json") {
                print_status(&value);
            }
            value
        }
        "status" => {
            let value = ipc::request(&paths, json!({"op":"status"}), false)?.unwrap_or(Value::Null);
            if !args.flag("--json") {
                print_status(&value);
            }
            value
        }
        "pause" | "resume" | "toggle" | "play-pause" => {
            let op = if command == "play-pause" {
                "toggle"
            } else {
                command
            };
            let value = request(&paths, json!({"op":op}))?;
            if !args.flag("--json") {
                print_status(&value);
            }
            value
        }
        "next" | "skip" | "prev" | "previous" => {
            let op = if command == "prev" || command == "previous" {
                "prev"
            } else {
                "next"
            };
            request(&paths, json!({"op":op}))?;
            let value = if args.flag("--no-wait") {
                request(&paths, json!({"op":"status"}))?
            } else {
                wait(&paths)?
            };
            if !args.flag("--json") {
                print_status(&value);
            }
            value
        }
        "stop" => {
            let value = request(&paths, json!({"op":"stop"}))?;
            if !args.flag("--json") {
                print_status(&value);
            }
            value
        }
        "lyrics-bar" => {
            let sub = args.pos.get(1).map(String::as_str).unwrap_or("status");
            let current = paths.settings()?.lyrics_bar;
            let target = match sub {
                "on" | "enable" | "1" | "true" => true,
                "off" | "disable" | "0" | "false" => false,
                "toggle" => !current,
                "status" => current,
                _ => return Err(Error::new("用法：wocli lyrics-bar [on|off|toggle|status]")),
            };
            if sub != "status" {
                if ipc::request(&paths, json!({"op":"status"}), false)?.is_some() {
                    request(
                        &paths,
                        json!({"op":"config","key":"lyrics_bar","value":target}),
                    )?;
                } else {
                    let mut s = paths.settings()?;
                    s.lyrics_bar = target;
                    paths.save_settings(&s)?;
                }
            }
            let res = json!({"lyrics_bar": target});
            if !args.flag("--json") {
                println!("任务栏歌词：{}", if target { "已开启" } else { "已关闭" });
            }
            res
        }
        "widgets" => {
            let action = args.pos.get(1).map(String::as_str).unwrap_or("status");
            if !["on", "off", "status"].contains(&action) {
                return Err(Error::new("用法：wocli widgets [on|off|status]"));
            }
            if action != "status" {
                let enabled = action == "on";
                crate::widgets::configure(&paths, enabled)?;
                if ipc::request(&paths, json!({"op":"status"}), false)?.is_some() {
                    request(
                        &paths,
                        json!({"op":"config","key":"lyrics_bar","value":enabled}),
                    )?;
                } else {
                    let mut settings = paths.settings()?;
                    settings.lyrics_bar = enabled;
                    paths.save_settings(&settings)?;
                }
            }
            let value = crate::widgets::status(&paths)?;
            if !args.flag("--json") {
                println!(
                    "Windows 系统歌词卡片（预览）：{}",
                    if value["enabled"] == true {
                        "已启用配置"
                    } else {
                        "未启用"
                    }
                );
                println!("{}", value["note"].as_str().unwrap_or_default());
                if action == "on" {
                    println!("安装配套 MSIX 组件后，按 Win+W，在小组件面板添加 wocli 歌词卡片。歌词和按钮由 Windows 渲染，仅在展开的面板内显示；关闭面板后停止实时同步。");
                }
            }
            value
        }
        "boss" => {
            let sub = args.pos.get(1).map(String::as_str);
            let target = match sub {
                Some("on" | "enable" | "1" | "true") => Some(true),
                Some("off" | "disable" | "0" | "false") => Some(false),
                Some("toggle") | None => None,
                Some(other) => {
                    return Err(Error::new(format!(
                        "未知参数 '{other}'。用法：wocli boss [on|off|toggle]"
                    )))
                }
            };
            let value = if let Some(active) = target {
                request(&paths, json!({"op":"boss","active":active}))?
            } else {
                request(&paths, json!({"op":"boss"}))?
            };
            if !args.flag("--json") {
                let is_boss = value["boss"].as_bool().unwrap_or(false);
                if is_boss {
                    println!("老板模式已开启：音频已暂停，歌词和歌曲信息已隐藏（托盘需运行）。");
                } else {
                    println!("老板模式已退出：界面已恢复；仅此前播放/加载中的歌曲会继续。");
                }
            }
            value
        }
        "tray" => {
            let action = args.pos.get(1).map(String::as_str).unwrap_or("status");
            match action {
                "run" => {
                    crate::tray::run(paths)?;
                    return Ok(());
                }
                "start" => {
                    crate::tray::start(&paths)?;
                    let res = json!({"tray": "running"});
                    if !args.flag("--json") {
                        println!("托盘服务运行中");
                    }
                    res
                }
                "stop" => {
                    crate::tray::stop(&paths)?;
                    let res = json!({"tray": "stopped"});
                    if !args.flag("--json") {
                        println!("托盘服务已停止");
                    }
                    res
                }
                "status" => {
                    let running = crate::tray::status(&paths)?;
                    let shortcuts = crate::tray::hotkeys(&paths);
                    let mut res = json!({"tray_running": running,"global_shortcuts":shortcuts});
                    if cfg!(windows) {
                        res["lyrics_surface"] = crate::tray::lyrics_surface(&paths);
                    }
                    if !args.flag("--json") {
                        println!(
                            "{}",
                            if running {
                                "托盘运行中"
                            } else {
                                "托盘未运行"
                            }
                        );
                        if let Some(keys) = res["global_shortcuts"]["keys"]
                            .as_array()
                            .filter(|keys| !keys.is_empty())
                        {
                            for key in keys {
                                println!(
                                    "{}：{}",
                                    key["shortcut"].as_str().unwrap_or_default(),
                                    if key["registered"] == true {
                                        "已注册"
                                    } else {
                                        "注册失败，请检查其他应用的按键占用"
                                    }
                                );
                            }
                        } else if let Some(note) = res["global_shortcuts"]["note"].as_str() {
                            println!("{note}");
                        }
                        if let Some(note) = res["lyrics_surface"]["note"].as_str() {
                            println!("任务栏歌词：{note}");
                        }
                    }
                    res
                }
                _ => {
                    return Err(Error::new(
                        "未知托盘操作，请使用：start | stop | status | run",
                    ))
                }
            }
        }
        "seek" | "volume" | "mode" => {
            let value = if command == "mode" {
                json!(args.arg(1)?)
            } else if command == "volume" {
                json!(args
                    .arg(1)?
                    .parse::<u64>()
                    .map_err(|_| Error::new("音量应为 0–100"))?)
            } else {
                let n = args
                    .arg(1)?
                    .parse::<f64>()
                    .map_err(|_| Error::new("跳转秒数无效"))?;
                if !n.is_finite() {
                    return Err(Error::new("跳转秒数无效"));
                }
                json!(n)
            };
            let value = request(
                &paths,
                if command == "seek" {
                    json!({"op":command,"seconds":value})
                } else {
                    json!({"op":command,"value":value})
                },
            )?;
            if !args.flag("--json") {
                print_status(&value);
            }
            value
        }
        "like" => {
            let track = args
                .value("--id")
                .map(|id| by_id(&store, &provider, id))
                .transpose()?;
            let value = request(&paths, json!({"op":"like","track":track}))?;
            if !args.flag("--json") {
                println!("已更新我喜欢");
            }
            value
        }
        "favorites" | "history" => {
            let tracks = if command == "favorites" {
                store.favorites()?
            } else {
                store.history()?
            };
            if !args.flag("--json") {
                table(&tracks);
            }
            json!(tracks)
        }
        "daily" => {
            let action = args.pos.get(1).map(String::as_str).unwrap_or("show");
            match action {
                "dislike" | "undo" => {
                    let id = args
                        .value("--id")
                        .ok_or_else(|| Error::new("请使用 --id 指定歌曲"))?;
                    let track = by_id(&store, &provider, id)?;
                    crate::recommend::set_blocked(&store, &track.id, action == "dislike")?;
                    if !args.flag("--json") {
                        println!(
                            "{}：{} - {}",
                            if action == "dislike" {
                                "日推已排除"
                            } else {
                                "已恢复日推"
                            },
                            track.artist,
                            track.title
                        );
                    }
                    json!({"track_id":track.id,"blocked":action == "dislike"})
                }
                "show" | "play" | "add" => {
                    let play_token = if action == "play" {
                        Some(request(&paths, json!({"op":"prepare_play"}))?["play_token"].clone())
                    } else {
                        None
                    };
                    let daily = crate::recommend::daily(
                        &store,
                        &provider,
                        args.flag("--offline"),
                        args.flag("--refresh"),
                    )?;
                    let tracks: Vec<Track> = daily.items.iter().map(|i| i.track.clone()).collect();
                    if action != "show" {
                        if tracks.is_empty() {
                            return Err(Error::new(
                                "今日推荐为空；先收藏歌曲或导入歌单，再使用 --refresh",
                            ));
                        }
                        if action == "play" {
                            let result = ipc::request(
                                &paths,
                                json!({"op":"play","tracks":tracks,"index":0,"quality":args.value("--quality"),"direct":args.flag("--direct") || settings.direct,"play_token":play_token}),
                                false,
                            )?.ok_or_else(|| Error::coded("cancelled", "后台已退出，已取消推荐播放"))?;
                            let result = if args.flag("--no-wait") {
                                result
                            } else {
                                wait(&paths)?
                            };
                            if !args.flag("--json") {
                                print_status(&result);
                            }
                            result
                        } else {
                            let result = request(
                                &paths,
                                json!({"op":"queue_add","tracks":tracks,"next":false}),
                            )?;
                            if !args.flag("--json") {
                                println!("已将 {} 首推荐加入队列", daily.items.len());
                            }
                            result
                        }
                    } else {
                        if !args.flag("--json") {
                            println!(
                                "每日推荐 {}{}",
                                daily.date,
                                if daily.offline {
                                    "（本地候选）"
                                } else {
                                    ""
                                }
                            );
                            for (i, item) in daily.items.iter().enumerate() {
                                println!(
                                    "{:>2}. {} - {}  [{}]  {}",
                                    i + 1,
                                    item.track.artist,
                                    item.track.title,
                                    item.track.id,
                                    item.reason
                                );
                            }
                            for warning in &daily.warnings {
                                eprintln!("{warning}");
                            }
                            if daily.items.is_empty() {
                                println!(
                                    "暂无推荐；收藏歌曲或导入歌单后运行 wocli daily --refresh。"
                                );
                            }
                        }
                        json!(daily)
                    }
                }
                _ => return Err(Error::new("未知每日推荐操作，请使用 wocli help daily")),
            }
        }
        "queue" => {
            let action = args.arg(1)?;
            let value = match action {
                "add" => request(
                    &paths,
                    json!({"op":"queue_add","tracks":[choose(&args,2,&store,&provider)?],"next":args.flag("--next")}),
                )?,
                "clear" => request(&paths, json!({"op":"queue_clear"}))?,
                "remove" => {
                    let index = args
                        .arg(2)?
                        .parse::<usize>()
                        .ok()
                        .and_then(|v| v.checked_sub(1))
                        .ok_or_else(|| Error::new("序号从 1 开始"))?;
                    request(&paths, json!({"op":"queue_remove","index":index}))?
                }
                "list" => {
                    ipc::request(&paths, json!({"op":"status"}), false)?.unwrap_or_else(|| {
                        let saved = store
                            .load::<Value>("queue")
                            .ok()
                            .flatten()
                            .unwrap_or(json!({"tracks":[],"index":0}));
                        json!({"queue":saved["tracks"],"index":saved["index"]})
                    })
                }
                _ => return Err(Error::new("未知队列操作")),
            };
            if !args.flag("--json") {
                table(&serde_json::from_value::<Vec<Track>>(
                    value["queue"].clone(),
                )?);
            }
            value
        }
        "playlist" => {
            let action = args.arg(1)?;
            let transfer = [
                "scan", "import", "export", "inspect", "resolve", "match", "drop", "backups",
                "restore",
            ]
            .contains(&action);
            let value = if transfer {
                playlist_transfer(&args, &store, &paths, &provider)?
            } else if action == "list" {
                json!(store
                    .folders()?
                    .iter()
                    .map(|(name, count)| json!({"name":name,"count":count}))
                    .collect::<Vec<_>>())
            } else {
                let name = args.arg(2)?;
                match action {
                    "create" => {
                        store.create_folder(name)?;
                        json!({"name":name})
                    }
                    "show" => {
                        let tracks = store.folder(name)?;
                        if !args.flag("--json") {
                            table(&tracks);
                        }
                        json!(tracks)
                    }
                    "play" => {
                        let tracks = store.folder(name)?;
                        if tracks.is_empty() {
                            return Err(Error::new("收藏夹为空"));
                        }
                        let value = request(
                            &paths,
                            json!({"op":"play","tracks":tracks,"index":0,"quality":args.value("--quality"),"direct":args.flag("--direct") || settings.direct}),
                        )?;
                        let value = if args.flag("--no-wait") {
                            value
                        } else {
                            wait(&paths)?
                        };
                        if !args.flag("--json") {
                            print_status(&value);
                        }
                        value
                    }
                    "add" => {
                        let track = by_id(
                            &store,
                            &provider,
                            args.value("--id").ok_or_else(|| Error::new("需要 --id"))?,
                        )?;
                        store.collect(name, &track)?;
                        json!({"name":name,"track":track})
                    }
                    "remove" => {
                        let id = args.value("--id").ok_or_else(|| Error::new("需要 --id"))?;
                        let id = if id.starts_with("local:") {
                            id.into()
                        } else {
                            normalize_id(id)?
                        };
                        store.remove(name, &id)?;
                        json!({"name":name,"removed":id})
                    }
                    "rename" => {
                        store.rename(name, args.arg(3)?)?;
                        json!({"name":args.arg(3)?,"previous":name})
                    }
                    "delete" => {
                        store.delete(name)?;
                        json!({"deleted":name})
                    }
                    _ => return Err(Error::new("未知收藏夹操作")),
                }
            };
            if !args.flag("--json") && !transfer && !["show", "play"].contains(&action) {
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            value
        }
        "lyrics" => {
            let track = if let Some(id) = args.value("--id") {
                by_id(&store, &provider, id)?
            } else {
                let value = ipc::request(&paths, json!({"op":"status"}), false)?
                    .ok_or_else(|| Error::new("请先播放歌曲或提供 --id"))?;
                serde_json::from_value(value["track"].clone())
                    .map_err(|_| Error::new("请先播放歌曲或提供 --id"))?
            };
            let mut text = if args.flag("--refresh") {
                String::new()
            } else {
                store.lyric(&track.id)?
            };
            if lyrics::parse(&text).is_empty() {
                text = provider.lyrics(&track)?;
            }
            if lyrics::parse(&text).is_empty() {
                return Err(Error::new("暂无匹配歌词"));
            }
            store.save_lyric(&track.id, &text)?;
            if let Some(output) = args.value("--output") {
                let path = expand_path(output);
                if path.exists() && !args.flag("--force") {
                    return Err(Error::new("文件已存在，覆盖需 --force"));
                }
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, &text)?;
            }
            if !args.flag("--json") {
                println!("{text}");
            }
            json!({"id":track.id,"lyrics":text})
        }
        "download" => {
            let track = choose(&args, 1, &store, &provider)?;
            let default_dir =
                std::env::var("WOCLI_DOWNLOAD_DIR").unwrap_or_else(|_| "downloads".into());
            let directory = expand_path(args.value("--output").unwrap_or(&default_dir));
            let directory = if directory.is_absolute() {
                directory
            } else {
                std::env::current_dir()?.join(directory)
            };
            let value = request(
                &paths,
                json!({"op":"download","track":track,"directory":directory,"quality":args.value("--quality").unwrap_or(&settings.quality),"lyrics":args.flag("--lyrics"),"force":args.flag("--force"),"direct":args.flag("--direct") || settings.direct}),
            )?;
            let state: Status = serde_json::from_value(value)?;
            let id = state
                .downloads
                .iter()
                .rev()
                .find(|j| j.track.id == track.id)
                .ok_or_else(|| Error::new("下载任务创建失败"))?
                .id
                .clone();
            let started = Instant::now();
            loop {
                let state: Status =
                    serde_json::from_value(request(&paths, json!({"op":"status"}))?)?;
                let job = state
                    .downloads
                    .iter()
                    .find(|j| j.id == id)
                    .ok_or_else(|| Error::new("下载任务已结束"))?;
                if job.phase == "done" {
                    if !args.flag("--json") {
                        println!("已保存 {}", job.path);
                        if !job.warning.is_empty() {
                            eprintln!("{}", job.warning);
                        }
                    }
                    break json!({"path":job.path,"quality":job.quality,"bytes":job.received,"warning":job.warning});
                }
                if ["error", "cancelled"].contains(&job.phase.as_str()) {
                    return Err(Error::new(&job.error));
                }
                if started.elapsed() > Duration::from_secs(360) {
                    return Err(Error::new("下载仍在后台进行，请在界面查看进度"));
                }
                thread::sleep(Duration::from_millis(250));
            }
        }
        "config" => {
            let mut settings = settings;
            if args.pos.len() > 2 {
                let key = args.arg(1)?;
                let raw = args.arg(2)?;
                let value = match key {
                    "volume" | "lyric_offset" => {
                        json!(raw.parse::<i64>().map_err(|_| Error::new("配置应为整数"))?)
                    }
                    "direct" | "lyrics_bar" | "tray" => json!(raw
                        .parse::<bool>()
                        .map_err(|_| Error::new(format!("{key} 应为 true 或 false")))?),
                    _ => json!(raw),
                };
                if ipc::request(&paths, json!({"op":"status"}), false)?.is_some() {
                    request(&paths, json!({"op":"config","key":key,"value":value}))?;
                } else {
                    let mut object = serde_json::to_value(&settings)?;
                    if object.get(key).is_none() {
                        return Err(Error::new("未知配置项"));
                    }
                    object[key] = value;
                    settings = serde_json::from_value(object)?;
                    paths.save_settings(&settings)?;
                }
            }
            let value = serde_json::to_value(paths.settings()?)?;
            let value = if args.pos.len() > 1 {
                let key = args.arg(1)?;
                if value.get(key).is_none() {
                    return Err(Error::new("未知配置项"));
                }
                json!({key:value[key]})
            } else {
                value
            };
            if !args.flag("--json") {
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            value
        }
        "doctor" => {
            let mut active_env = serde_json::Map::new();
            for key in [
                "WOCLI_HOME",
                "WOCLI_RUNTIME_DIR",
                "WOCLI_DOWNLOAD_DIR",
                "WOCLI_AUDIO_OUTPUT",
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "NO_PROXY",
            ] {
                if let Ok(v) = std::env::var(key) {
                    active_env.insert(key.into(), json!(v));
                }
            }
            let value = json!({
                "version": env!("CARGO_PKG_VERSION"),
                "engine": "rodio/symphonia",
                "runtime": "native",
                "platform": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
                "home": paths.home,
                "runtime_dir": paths.runtime,
                "socket": paths.socket(),
                "env": active_env,
                "service_running": ipc::request(&paths, json!({"op":"status"}), false)?.is_some(),
            });
            if !args.flag("--json") {
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            value
        }
        "daemon" => {
            let action = args.arg(1)?;
            let value = match action {
                "start" => {
                    let v = request(&paths, json!({"op":"status"}))?;
                    if args.flag("--tray") || settings.tray {
                        let _ = crate::tray::start(&paths);
                    }
                    v
                }
                "stop" => {
                    let _ = crate::tray::stop(&paths);
                    ipc::request(&paths, json!({"op":"shutdown"}), false)?.unwrap_or(Value::Null)
                }
                "status" => {
                    ipc::request(&paths, json!({"op":"status"}), false)?.unwrap_or(Value::Null)
                }
                _ => return Err(Error::new("未知后台操作")),
            };
            if !args.flag("--json") {
                println!(
                    "{}",
                    if action == "stop" {
                        "后台已停止"
                    } else if value.is_null() {
                        "后台未运行"
                    } else {
                        "后台运行中"
                    }
                );
            }
            value
        }
        _ => return Err(Error::new("未知命令，请使用 --help")),
    };
    if args.flag("--json") {
        println!("{}", serde_json::to_string(&data)?);
    }
    Ok(())
}

fn playlist_transfer(
    args: &Args,
    store: &Store,
    paths: &Paths,
    provider: &Provider,
) -> Result<Value> {
    use crate::playlists::{self, Conflict, MatchState};
    use std::sync::atomic::AtomicBool;
    let action = args.arg(1)?;
    let name = args.arg(2)?;
    let conflict = Conflict::parse(args.value("--on-conflict").unwrap_or("rename"))?;
    let value = match action {
        "scan" => json!(playlists::read(
            &expand_path(name),
            args.value("--text-order").unwrap_or("artist-title")
        )?
        .iter()
        .enumerate()
        .map(
            |(i, p)| json!({"index":i+1,"name":p.name,"format":p.format,"entries":p.entries.len()})
        )
        .collect::<Vec<_>>()),
        "import" | "resolve" => {
            let mut list = if action == "import" {
                playlists::select(
                    playlists::read(
                        &expand_path(name),
                        args.value("--text-order").unwrap_or("artist-title"),
                    )?,
                    args.value("--list"),
                    args.value("--name"),
                )?
            } else {
                playlists::stored(store, name)?
            };
            let cancel = AtomicBool::new(false);
            let known = store.known_tracks()?;
            if !args.flag("--json") {
                eprintln!(
                    "解析 {}：{} 条；正在{}匹配…",
                    list.name,
                    list.entries.len(),
                    if args.flag("--offline") {
                        "离线"
                    } else {
                        "跨平台"
                    }
                );
            }
            playlists::resolve(
                &mut list,
                &known,
                paths,
                playlists::ResolveOptions {
                    offline: args.flag("--offline"),
                    direct: args.flag("--direct"),
                    retry: action == "resolve",
                },
                &cancel,
                |done, total| {
                    if !args.flag("--json") && (done == total || done % 10 == 0) {
                        eprintln!("已匹配处理 {done}/{total}");
                    }
                },
            )?;
            if args.flag("--dry-run") {
                json!({"dry_run":true,"name":list.name,"summary":list.summary(),"playlist":list})
            } else {
                json!(playlists::save(
                    store,
                    list,
                    if action == "resolve" {
                        Conflict::Replace
                    } else {
                        conflict
                    }
                )?)
            }
        }
        "inspect" => {
            let list = playlists::stored(store, name)?;
            json!({"name":name,"summary":list.summary(),"playlist":list})
        }
        "export" => {
            let path = expand_path(
                args.value("--output")
                    .ok_or_else(|| Error::new("需要 --output 文件路径"))?,
            );
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("json");
            let format = args.value("--format").unwrap_or(ext);
            let list = playlists::stored(store, name)?;
            playlists::write(&list, format, &path, args.flag("--force"))?;
            json!({"name":name,"format":format,"output":path,"summary":list.summary()})
        }
        "match" | "drop" => {
            let mut list = playlists::stored(store, name)?;
            let index = args
                .integer("--entry", 0)?
                .checked_sub(1)
                .filter(|&n| n < list.entries.len())
                .ok_or_else(|| Error::new("--entry 应为 inspect 中的条目序号（从 1 开始）"))?;
            if action == "drop" {
                list.entries.remove(index);
            } else {
                let track = by_id(
                    store,
                    provider,
                    args.value("--id")
                        .ok_or_else(|| Error::new("需要 --id 目标酷我/本地歌曲 ID"))?,
                )?;
                let entry = &mut list.entries[index];
                entry.resolved = Some(track);
                entry.status = MatchState::Matched;
                entry.reason = "用户手动确认".into();
                entry.manual = true;
                entry.candidates.clear();
            }
            json!(playlists::save(store, list, Conflict::Replace)?)
        }
        "backups" => json!(playlists::backups(store, name)?),
        "restore" => json!(playlists::restore(
            store,
            name,
            args.value("--id")
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| Error::new("需要 --id 快照编号"))?
        )?),
        _ => return Err(Error::new("未知导入导出操作")),
    };
    if !args.flag("--json") {
        if action == "scan" || action == "backups" {
            println!("{}", serde_json::to_string_pretty(&value)?);
        } else {
            if let Some(summary) = value.get("summary") {
                println!(
                    "{}{}：总计 {}，可播放 {}，未匹配 {}，待确认 {}，无效 {}，重复 {}",
                    if args.flag("--dry-run") {
                        "预览（未保存） "
                    } else {
                        ""
                    },
                    value["name"].as_str().unwrap_or(name),
                    summary["total"],
                    summary["matched"],
                    summary["pending"],
                    summary["ambiguous"],
                    summary["invalid"],
                    summary["duplicates"]
                );
            }
            if let Some(entries) = value["playlist"]["entries"].as_array() {
                for (i, e) in entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e["status"] != "matched" && e["duplicate"] != true)
                    .take(20)
                {
                    println!(
                        "  #{} {} · {}：{}",
                        i + 1,
                        crate::model::clean(e["title"].as_str().unwrap_or("")),
                        crate::model::clean(e["artist"].as_str().unwrap_or("")),
                        e["reason"].as_str().unwrap_or("")
                    );
                }
            }
            if action == "export" {
                println!("已导出到 {}", value["output"].as_str().unwrap_or(""));
            }
            if let Some(id) = value["backup"].as_i64() {
                println!(
                    "已保留快照 #{id}；恢复：wocli playlist restore \"{}\" --id {id}",
                    value["name"].as_str().unwrap_or(name)
                );
            }
            if value["summary"]["pending"].as_u64().unwrap_or(0)
                + value["summary"]["ambiguous"].as_u64().unwrap_or(0)
                > 0
            {
                println!("未匹配条目已保留；playlist inspect 查看，playlist resolve 重试，playlist match 手动指定。");
            }
        }
    }
    Ok(value)
}
