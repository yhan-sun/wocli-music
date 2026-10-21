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

pub const HELP:&str="wocli — 原生终端音乐播放器\n\n用法：wocli [命令] [参数]\n  tui                               打开终端界面（默认）\n  search 关键词 [--page 1 --limit 20] 搜索歌曲\n  play 关键词 [--pick N]             选择并播放\n  play --id ID [--quality best]      按 ID 播放\n  play /path/song.flac               播放本地音频\n  pause | resume | toggle | stop     播放控制\n  next | prev | seek 秒数            切歌和相对跳转\n  volume 0-100 | mode 播放方式        list / repeat / single / shuffle\n  status | favorites | history       状态、喜欢、历史\n  queue add|list|remove|clear         管理队列\n  playlist create|list|show|add|remove|rename|delete|play\n  like [--id ID]                     喜欢或取消喜欢\n  lyrics [--id ID --refresh --output song.lrc]\n  download [--id ID | 关键词 --pick N] [--quality best --lyrics --output 目录]\n  config [配置项 值]                 quality / volume / mode / direct / lyric_offset\n  doctor | daemon start|stop|status   诊断及后台服务\n\n--json 输出 JSON；--direct 忽略环境代理；--version 查看版本。\n";
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
    if let Some(rest) = value.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
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
        thread::sleep(Duration::from_millis(120));
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
}
pub fn run(raw: Vec<String>) -> Result<()> {
    let args = Args::parse(raw)?;
    if args.flag("--version") {
        println!("wocli-music {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.flag("--help") || args.flag("-h") {
        print!("{HELP}");
        return Ok(());
    }
    let paths = Paths::discover()?;
    let command = args.pos.first().map(String::as_str).unwrap_or("tui");
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
        "status" => {
            let value = ipc::request(&paths, json!({"op":"status"}), false)?.unwrap_or(Value::Null);
            if !args.flag("--json") {
                print_status(&value);
            }
            value
        }
        "pause" | "resume" | "toggle" | "next" | "prev" | "stop" => {
            let value = request(&paths, json!({"op":command}))?;
            if !args.flag("--json") {
                print_status(&value);
            }
            value
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
            let value = if action == "list" {
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
                        request(&paths, json!({"op":"play","tracks":tracks,"index":0}))?;
                        let value = wait(&paths)?;
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
            if !args.flag("--json") && !["show", "play"].contains(&action) {
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
            let directory = expand_path(args.value("--output").unwrap_or("downloads"));
            let directory = if directory.is_absolute() {
                directory
            } else {
                std::env::current_dir()?.join(directory)
            };
            let value = request(
                &paths,
                json!({"op":"download","track":track,"directory":directory,"quality":args.value("--quality").unwrap_or(&settings.quality),"lyrics":args.flag("--lyrics"),"force":args.flag("--force")}),
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
                    "direct" => json!(raw
                        .parse::<bool>()
                        .map_err(|_| Error::new("direct 应为 true 或 false"))?),
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
            let value = json!({"version":env!("CARGO_PKG_VERSION"),"engine":"rodio/symphonia","runtime":"native","platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"home":paths.home,"socket":paths.socket(),"service_running":ipc::request(&paths,json!({"op":"status"}),false)?.is_some()});
            if !args.flag("--json") {
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            value
        }
        "daemon" => {
            let action = args.arg(1)?;
            let value = match action {
                "start" => request(&paths, json!({"op":"status"}))?,
                "stop" => {
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
