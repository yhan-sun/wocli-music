use super::params::*;
use crate::{
    ipc, lyrics,
    model::{normalize_id, Error, Result, Track},
    provider::Provider,
    sources,
    store::{Paths, Store},
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
pub struct Backend {
    pub paths: Paths,
    pub direct: bool,
    pub read_only: bool,
}
fn parse<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value)
        .map_err(|e| Error::coded("invalid_arguments", format!("工具参数无效：{e}")))
}
fn check(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        Err(Error::coded("cancelled", "MCP 请求已取消"))
    } else {
        Ok(())
    }
}
fn text(value: &str, label: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(Error::coded(
            "invalid_arguments",
            format!("{label} 不能为空、过长或包含控制字符"),
        ));
    }
    Ok(())
}
fn normalized(id: &str) -> Result<String> {
    text(id, "track_id", 128)?;
    if id.starts_with("local:") {
        Ok(id.into())
    } else {
        normalize_id(id)
    }
}
fn page(entries: Vec<Value>, offset: usize, limit: usize) -> Result<Value> {
    if offset > 100000 || !(1..=100).contains(&limit) {
        return Err(Error::coded(
            "invalid_arguments",
            "offset 应为 0–100000，limit 应为 1–100",
        ));
    }
    let total = entries.len();
    let items: Vec<Value> = entries.into_iter().skip(offset).take(limit).collect();
    let next = (offset.saturating_add(items.len()) < total).then_some(offset + items.len());
    Ok(json!({"items":items,"offset":offset,"limit":limit,"total":total,"next_offset":next}))
}
fn track_values(tracks: Vec<Track>) -> Result<Vec<Value>> {
    tracks
        .into_iter()
        .map(|t| serde_json::to_value(t).map_err(Into::into))
        .collect()
}

impl Backend {
    fn provider(&self) -> Result<Provider> {
        Ok(Provider::new(self.direct || self.paths.settings()?.direct))
    }
    fn request(&self, value: Value, autostart: bool, cancelled: &AtomicBool) -> Result<Value> {
        check(cancelled)?;
        ipc::request(&self.paths, value, autostart)?
            .ok_or_else(|| Error::coded("daemon_stopped", "播放器后台未运行，请先播放歌曲"))
    }
    fn raw_status(&self) -> Result<Option<Value>> {
        ipc::request(&self.paths, json!({"op":"status"}), false)
    }
    fn track(
        &self,
        id: &str,
        store: &Store,
        provider: &Provider,
        cancelled: &AtomicBool,
    ) -> Result<Track> {
        check(cancelled)?;
        let id = normalized(id)?;
        if let Some(track) = store.track(&id)? {
            return Ok(track);
        }
        if id.starts_with("local:") {
            return Err(Error::new("本地歌曲未记录，请使用 play_music 的 path 参数"));
        }
        let track = provider.details(&id)?;
        check(cancelled)?;
        if !self.read_only {
            store.remember(std::slice::from_ref(&track))?;
        }
        Ok(track)
    }
    fn selected_track(
        &self,
        id: Option<&str>,
        store: &Store,
        provider: &Provider,
        cancelled: &AtomicBool,
    ) -> Result<Track> {
        if let Some(id) = id {
            return self.track(id, store, provider, cancelled);
        }
        let value = self
            .raw_status()?
            .ok_or_else(|| Error::new("没有当前歌曲，请指定 track_id"))?;
        serde_json::from_value(value["track"].clone())
            .map_err(|_| Error::new("没有当前歌曲，请指定 track_id"))
    }
    fn summary(&self, state: Option<Value>) -> Result<Value> {
        if let Some(state) = state {
            let mut value = state
                .as_object()
                .cloned()
                .ok_or_else(|| Error::new("播放状态无效"))?;
            let length = value
                .remove("queue")
                .and_then(|q| q.as_array().map(Vec::len))
                .unwrap_or_default();
            value.remove("lyrics");
            let jobs = value
                .remove("downloads")
                .and_then(|q| q.as_array().map(Vec::len))
                .unwrap_or_default();
            let index = if length > 0 {
                state["index"].as_u64().map(|i| i + 1)
            } else {
                None
            };
            value.insert("running".into(), json!(true));
            value.insert("queue_length".into(), json!(length));
            value.insert("queue_index".into(), json!(index));
            value.remove("index");
            value.insert("download_count".into(), json!(jobs));
            Ok(Value::Object(value))
        } else {
            let settings = self.paths.settings()?;
            Ok(
                json!({"running":false,"phase":"stopped","track":null,"queue_index":null,"volume":settings.volume,"mode":settings.mode,"default_quality":settings.quality}),
            )
        }
    }
    pub fn execute(&self, name: &str, args: Value, cancelled: &AtomicBool) -> Result<Value> {
        check(cancelled)?;
        // Check again here as well as in tools/list/call: hidden write tools cannot be invoked in read-only mode.
        if self.read_only && !super::catalog::tools(true).iter().any(|t| t.name == name) {
            return Err(Error::coded("read_only", "此 MCP 服务以只读模式运行"));
        }
        match name {
            "get_playback_status" => {
                let _: Empty = parse(args)?;
                self.summary(self.raw_status()?)
            }
            "get_player_config" => {
                let _: Empty = parse(args)?;
                Ok(serde_json::to_value(self.paths.settings()?)?)
            }
            "search_music" => {
                let a: Search = parse(args)?;
                text(&a.keyword, "keyword", 500)?;
                if !(1..=1000).contains(&a.page) || !(1..=100).contains(&a.limit) {
                    return Err(Error::coded(
                        "invalid_arguments",
                        "page 应为 1–1000，limit 应为 1–100",
                    ));
                }
                let (total, tracks) = self.provider()?.search(&a.keyword, a.page - 1, a.limit)?;
                check(cancelled)?;
                if !self.read_only {
                    Store::open(&self.paths)?.remember(&tracks)?;
                }
                Ok(json!({"total":total,"page":a.page,"tracks":tracks}))
            }
            "get_queue" => {
                let a: Page = parse(args)?;
                let state = self.raw_status()?;
                let (tracks, index) = if let Some(s) = state {
                    (
                        s["queue"].as_array().cloned().unwrap_or_default(),
                        s["index"].as_i64().unwrap_or(0),
                    )
                } else {
                    let saved = Store::open(&self.paths)?
                        .load::<Value>("queue")?
                        .unwrap_or(json!({"tracks":[],"index":0}));
                    (
                        saved["tracks"].as_array().cloned().unwrap_or_default(),
                        saved["index"].as_i64().unwrap_or(0),
                    )
                };
                let index = if tracks.is_empty() {
                    None
                } else {
                    Some((index.max(0) as usize).min(tracks.len() - 1) + 1)
                };
                let mut result = page(tracks, a.offset, a.limit)?;
                result["current_index"] = json!(index);
                Ok(result)
            }
            "get_library" => {
                let a: Library = parse(args)?;
                let store = Store::open(&self.paths)?;
                let items = match a.kind {
                    LibraryKind::Favorites => track_values(store.favorites()?)?,
                    LibraryKind::History => track_values(store.history()?)?,
                    LibraryKind::Playlists => store
                        .folders()?
                        .into_iter()
                        .map(|(name, count)| json!({"name":name,"count":count}))
                        .collect(),
                    LibraryKind::Playlist => {
                        let name = a
                            .name
                            .ok_or_else(|| Error::new("kind=playlist 时需要 name"))?;
                        text(&name, "name", 300)?;
                        track_values(store.folder(&name)?)?
                    }
                };
                page(items, a.offset, a.limit)
            }
            "list_music_sources" => {
                let _: Empty = parse(args)?;
                let all = sources::Sources::load(&self.paths.home)?;
                Ok(
                    json!({"sources":all.sources.iter().map(|s|json!({"id":s.id,"name":s.name,"enabled":s.enabled,"builtin":s.builtin,"qualities":s.qualities,"kind":match s.kind{sources::SourceKind::Native=>"native",sources::SourceKind::Lx{..}=>"lx",sources::SourceKind::Http{..}=>"http"}})).collect::<Vec<_>>(),"scope":"kuwo"}),
                )
            }
            "get_downloads" => {
                let a: Page = parse(args)?;
                let state = self.raw_status()?;
                let jobs = state
                    .as_ref()
                    .and_then(|s| s["downloads"].as_array())
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .rev()
                    .collect();
                let mut value = page(jobs, a.offset, a.limit)?;
                value["running"] = json!(state.is_some());
                Ok(value)
            }
            "get_lyrics" => {
                let a: Lyrics = parse(args)?;
                let store = Store::open(&self.paths)?;
                let provider = self.provider()?;
                let track =
                    self.selected_track(a.track_id.as_deref(), &store, &provider, cancelled)?;
                let mut value = if a.refresh {
                    String::new()
                } else {
                    store.lyric(&track.id)?
                };
                if lyrics::parse(&value).is_empty() {
                    value = provider.lyrics(&track)?;
                    check(cancelled)?;
                    if !self.read_only && !lyrics::parse(&value).is_empty() {
                        store.save_lyric(&track.id, &value)?;
                    }
                }
                Ok(
                    json!({"track_id":track.id,"available":!lyrics::parse(&value).is_empty(),"lyrics":value}),
                )
            }
            "play_music" => {
                let a: Play = parse(args)?;
                if a.track_id.is_some() && a.path.is_some() {
                    return Err(Error::coded(
                        "invalid_arguments",
                        "track_id 和 path 只能提供一个",
                    ));
                }
                let store = Store::open(&self.paths)?;
                let provider = self.provider()?;
                let tracks = if let Some(id) = a.track_id {
                    vec![self.track(&id, &store, &provider, cancelled)?]
                } else if let Some(path) = a.path {
                    text(&path, "path", 4096)?;
                    let track = Track::local(&crate::cli::expand_path(&path))?;
                    check(cancelled)?;
                    store.remember(std::slice::from_ref(&track))?;
                    vec![track]
                } else {
                    vec![]
                };
                let result=self.request(json!({"op":"play","tracks":tracks,"quality":a.quality.map(Quality::as_str),"direct":self.direct}),true,cancelled)?;
                self.summary(Some(result))
            }
            "control_playback" => {
                let a: Control = parse(args)?;
                let value = match a {
                    Control::Pause => json!({"op":"pause"}),
                    Control::Resume => json!({"op":"resume"}),
                    Control::Toggle => json!({"op":"toggle"}),
                    Control::Stop => json!({"op":"stop"}),
                    Control::Next => json!({"op":"next"}),
                    Control::Previous => json!({"op":"prev"}),
                    Control::Retry { quality } => {
                        json!({"op":"retry","quality":quality.map(Quality::as_str),"direct":self.direct})
                    }
                    Control::Seek { seconds } => {
                        if !seconds.is_finite() || seconds.abs() > 86400. {
                            return Err(Error::new("seconds 应为 -86400 至 86400 的有限数字"));
                        }
                        json!({"op":"seek","seconds":seconds})
                    }
                    Control::Volume { value } => {
                        if value > 100 {
                            return Err(Error::new("音量应为 0–100"));
                        }
                        json!({"op":"volume","value":value})
                    }
                    Control::Mode { value } => json!({"op":"mode","value":value}),
                };
                if value["op"] == "stop" && self.raw_status()?.is_none() {
                    return self.summary(None);
                }
                let result = self.request(value, false, cancelled)?;
                self.summary(Some(result))
            }
            "manage_queue" => {
                let a: Queue = parse(args)?;
                let request = match a {
                    Queue::Add { track_ids, next } => {
                        if track_ids.is_empty() || track_ids.len() > 100 {
                            return Err(Error::new("track_ids 需要包含 1–100 个 ID"));
                        }
                        let store = Store::open(&self.paths)?;
                        let provider = self.provider()?;
                        let tracks = track_ids
                            .iter()
                            .map(|id| self.track(id, &store, &provider, cancelled))
                            .collect::<Result<Vec<_>>>()?;
                        json!({"op":"queue_add","tracks":tracks,"next":next})
                    }
                    Queue::Remove { index, track_id } => {
                        if index == 0 {
                            return Err(Error::new("index 从 1 开始"));
                        }
                        json!({"op":"queue_remove","index":index-1,"track_id":normalized(&track_id)?})
                    }
                    Queue::Clear => json!({"op":"queue_clear"}),
                };
                let state = self.request(request, true, cancelled)?;
                self.summary(Some(state))
            }
            "set_favorite" => {
                let a: Favorite = parse(args)?;
                let store = Store::open(&self.paths)?;
                let provider = self.provider()?;
                let track =
                    self.selected_track(a.track_id.as_deref(), &store, &provider, cancelled)?;
                check(cancelled)?;
                store.set_favorite(&track, a.liked)?;
                Ok(json!({"track_id":track.id,"liked":a.liked}))
            }
            "manage_playlist" => self.playlist(parse(args)?, cancelled),
            "download_music" => {
                let a: Download = parse(args)?;
                let store = Store::open(&self.paths)?;
                let provider = self.provider()?;
                let track = self.track(&a.track_id, &store, &provider, cancelled)?;
                let directory = a
                    .directory
                    .or_else(|| std::env::var("WOCLI_DOWNLOAD_DIR").ok())
                    .unwrap_or_else(|| "~/Music/wocli".into());
                text(&directory, "directory", 4096)?;
                let directory = crate::cli::expand_path(&directory);
                let directory = if directory.is_absolute() {
                    directory
                } else {
                    std::env::current_dir()?.join(directory)
                };
                let quality = a
                    .quality
                    .map(Quality::as_str)
                    .map(str::to_owned)
                    .unwrap_or(self.paths.settings()?.quality);
                let result=self.request(json!({"op":"download","track":track,"directory":directory,"quality":quality,"lyrics":a.lyrics,"force":a.overwrite,"direct":self.direct}),true,cancelled)?;
                let job = result["downloads"]
                    .as_array()
                    .and_then(|jobs| {
                        jobs.iter().rev().find(|j| {
                            j["track"]["id"] == track.id
                                && j["requested_quality"] == quality
                                && j["directory"] == directory.to_string_lossy().as_ref()
                                && j["lyrics"] == a.lyrics
                        })
                    })
                    .cloned()
                    .ok_or_else(|| Error::new("下载任务未创建"))?;
                Ok(json!({"job_id":job["id"],"job":job}))
            }
            "manage_download" => {
                let a: DownloadControl = parse(args)?;
                let (op, id) = match a {
                    DownloadControl::Cancel { job_id } => ("download_cancel", job_id),
                    DownloadControl::Retry { job_id } => ("download_retry", job_id),
                };
                text(&id, "job_id", 100)?;
                let result = self.request(json!({"op":op,"id":id}), false, cancelled)?;
                let job = result["downloads"]
                    .as_array()
                    .and_then(|jobs| jobs.iter().find(|j| j["id"] == id))
                    .cloned()
                    .ok_or_else(|| Error::new("找不到下载任务"))?;
                Ok(json!({"job_id":id,"job":job}))
            }
            "configure_player" => self.configure(parse(args)?, cancelled),
            "manage_music_sources" => {
                let a: SourceControl = parse(args)?;
                check(cancelled)?;
                match a {
                    SourceControl::Enable { source_id } => {
                        sources::enable(&self.paths.home, &source_id, true)?;
                    }
                    SourceControl::Disable { source_id } => {
                        sources::enable(&self.paths.home, &source_id, false)?;
                    }
                    SourceControl::Prefer { source_id } => {
                        sources::prefer(&self.paths.home, &source_id)?;
                    }
                }
                self.execute("list_music_sources", json!({}), cancelled)
            }
            _ => Err(Error::coded("unknown_tool", "未知 MCP 工具")),
        }
    }
    fn playlist(&self, args: Playlist, cancelled: &AtomicBool) -> Result<Value> {
        let store = Store::open(&self.paths)?;
        let provider = self.provider()?;
        let name = match &args {
            Playlist::Create { name }
            | Playlist::Rename { name, .. }
            | Playlist::Delete { name }
            | Playlist::Add { name, .. }
            | Playlist::Remove { name, .. }
            | Playlist::Play { name, .. } => name,
        };
        text(name, "name", 300)?;
        check(cancelled)?;
        match args {
            Playlist::Create { name } => {
                store.create_folder(&name)?;
                Ok(json!({"name":name}))
            }
            Playlist::Rename { name, new_name } => {
                text(&new_name, "new_name", 300)?;
                store.rename(&name, &new_name)?;
                Ok(json!({"name":new_name,"previous":name}))
            }
            Playlist::Delete { name } => {
                store.folder(&name)?;
                store.delete(&name)?;
                Ok(json!({"deleted":name}))
            }
            Playlist::Add { name, track_id } => {
                let track = self.track(&track_id, &store, &provider, cancelled)?;
                check(cancelled)?;
                store.collect(&name, &track)?;
                Ok(json!({"name":name,"track":track}))
            }
            Playlist::Remove { name, track_id } => {
                store.folder(&name)?;
                let id = normalized(&track_id)?;
                store.remove(&name, &id)?;
                Ok(json!({"name":name,"removed":id}))
            }
            Playlist::Play { name, quality } => {
                let tracks = store.folder(&name)?;
                if tracks.is_empty() {
                    return Err(Error::new("歌单为空"));
                }
                let result=self.request(json!({"op":"play","tracks":tracks,"index":0,"quality":quality.map(Quality::as_str),"direct":self.direct}),true,cancelled)?;
                self.summary(Some(result))
            }
        }
    }
    fn configure(&self, args: Config, cancelled: &AtomicBool) -> Result<Value> {
        static CONFIG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = CONFIG_LOCK.lock().map_err(|_| Error::new("配置锁不可用"))?;
        check(cancelled)?;
        let (key, value) = match args {
            Config::Quality(v) => ("quality", json!(v.as_str())),
            Config::Volume(v) => ("volume", json!(v)),
            Config::Mode(v) => ("mode", json!(v)),
            Config::Direct(v) => ("direct", json!(v)),
            Config::LyricOffset(v) => ("lyric_offset", json!(v)),
            Config::LyricsBar(v) => ("lyrics_bar", json!(v)),
            Config::Tray(v) => ("tray", json!(v)),
        };
        let mut object = serde_json::to_value(self.paths.settings()?)?;
        object[key] = value.clone();
        let settings = serde_json::from_value(object)?;
        Paths::validate(&settings)?;
        check(cancelled)?;
        if self.raw_status()?.is_some() {
            self.request(
                json!({"op":"config","key":key,"value":value}),
                false,
                cancelled,
            )?;
        } else {
            self.paths.save_settings(&settings)?;
        }
        Ok(serde_json::to_value(self.paths.settings()?)?)
    }
}
