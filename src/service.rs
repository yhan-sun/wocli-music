use crate::{
    lyrics,
    model::{Error, Result, Settings, Track, MODES, QUALITIES},
    order::Shuffle,
    player::Player,
    provider::Provider,
    store::{now, Paths, Store},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Job {
    pub id: String,
    pub track: Track,
    pub phase: String,
    pub quality: String,
    pub requested_quality: String,
    pub received: u64,
    pub total: u64,
    pub error: String,
    pub warning: String,
    pub path: String,
    pub directory: String,
    pub lyrics: bool,
    pub overwrite: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub track: Option<Track>,
    pub queue: Vec<Track>,
    pub index: usize,
    pub phase: String,
    pub position: f64,
    pub duration: f64,
    pub quality: String,
    pub requested_quality: String,
    pub default_quality: String,
    pub volume: u8,
    pub mode: String,
    pub liked: bool,
    pub error: String,
    pub error_code: String,
    pub error_details: Vec<String>,
    pub retry_attempt: u8,
    pub lyrics: String,
    pub lyrics_status: String,
    pub lyrics_error: String,
    pub lyric_offset: i64,
    pub received: u64,
    pub total: u64,
    pub downloads: Vec<Job>,
}
#[derive(Serialize, Deserialize, Default)]
struct Queue {
    tracks: Vec<Track>,
    index: i64,
}
struct Call {
    value: Value,
    reply: Sender<Result<Value>>,
}
enum Event {
    Call(Call),
    Ready(u64, Result<(PathBuf, String)>),
    Lyrics(u64, u64, Result<String>),
    Buffer(u64, u64, u64),
    Attempt(u64, u8),
    JobProgress(String, u64, u64, u64),
    JobPhase(String, u64, String, String),
    JobDone(String, u64, Result<(String, String)>),
}
struct JobControl {
    cancelled: Arc<AtomicBool>,
    serial: u64,
}
struct Service {
    paths: Paths,
    store: Store,
    settings: Settings,
    player: Player,
    queue: Vec<Track>,
    index: usize,
    phase: String,
    quality: String,
    requested_quality: String,
    error: Option<Error>,
    received: u64,
    total: u64,
    retry_attempt: u8,
    lyric_text: String,
    lyric_status: String,
    lyric_error: String,
    lyric_serial: u64,
    generation: u64,
    direct_override: bool,
    cancelled: Arc<AtomicBool>,
    tx: Sender<Event>,
    shuffle: Shuffle,
    cache: Option<PathBuf>,
    jobs: Vec<Job>,
    job_controls: HashMap<String, JobControl>,
    job_queue: VecDeque<String>,
    job_active: Option<(String, u64)>,
    job_sequence: u64,
    workers: Vec<JoinHandle<()>>,
    quit: Arc<AtomicBool>,
}
impl Service {
    fn new(paths: Paths, tx: Sender<Event>, quit: Arc<AtomicBool>) -> Result<Self> {
        let store = Store::open(&paths)?;
        let settings = paths.settings()?;
        let saved = store.load::<Queue>("queue")?.unwrap_or_default();
        let index = (saved.index.max(0) as usize).min(saved.tracks.len().saturating_sub(1));
        let mut shuffle = Shuffle::default();
        shuffle.reset(saved.tracks.len(), index);
        let cache_dir = paths.home.join("cache-rust");
        fs::create_dir_all(&cache_dir)?;
        for file in fs::read_dir(&cache_dir)?.flatten() {
            if file.file_type().is_ok_and(|t| t.is_file()) {
                let _ = fs::remove_file(file.path());
            }
        }
        Ok(Self {
            paths,
            store,
            requested_quality: settings.quality.clone(),
            settings,
            player: Player::default(),
            queue: saved.tracks,
            index,
            phase: "stopped".into(),
            quality: String::new(),
            error: None,
            received: 0,
            total: 0,
            retry_attempt: 1,
            lyric_text: String::new(),
            lyric_status: "idle".into(),
            lyric_error: String::new(),
            lyric_serial: 0,
            generation: 0,
            direct_override: false,
            cancelled: Arc::new(AtomicBool::new(false)),
            tx,
            shuffle,
            cache: None,
            jobs: vec![],
            job_controls: HashMap::new(),
            job_queue: VecDeque::new(),
            job_active: None,
            job_sequence: 0,
            workers: vec![],
            quit,
        })
    }
    fn current(&self) -> Option<&Track> {
        self.queue.get(self.index)
    }
    fn status(&self) -> Status {
        let track = self.current().cloned();
        let duration = if self.player.duration() > 0. {
            self.player.duration()
        } else {
            track.as_ref().map(|v| v.duration).unwrap_or(0.)
        };
        Status {
            liked: track
                .as_ref()
                .is_some_and(|v| self.store.is_favorite(&v.id)),
            track,
            queue: self.queue.clone(),
            index: self.index,
            phase: self.phase.clone(),
            position: self.player.position(),
            duration,
            quality: self.quality.clone(),
            requested_quality: self.requested_quality.clone(),
            default_quality: self.settings.quality.clone(),
            volume: self.settings.volume,
            mode: self.settings.mode.clone(),
            error: self
                .error
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            error_code: self
                .error
                .as_ref()
                .map(|e| e.code.clone())
                .unwrap_or_default(),
            error_details: self
                .error
                .as_ref()
                .map(|e| e.details.clone())
                .unwrap_or_default(),
            lyrics: self.lyric_text.clone(),
            lyrics_status: self.lyric_status.clone(),
            lyrics_error: self.lyric_error.clone(),
            lyric_offset: self.settings.lyric_offset,
            received: self.received,
            total: self.total,
            retry_attempt: self.retry_attempt,
            downloads: self.jobs.clone(),
        }
    }
    fn persist(&self) -> Result<()> {
        self.store.save(
            "queue",
            &Queue {
                tracks: self.queue.clone(),
                index: self.index as i64,
            },
        )
    }
    fn stop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.generation += 1;
        self.player.stop();
        self.phase = "stopped".into();
        self.error = None;
        if let Some(path) = self.cache.take() {
            let _ = fs::remove_file(path);
        }
    }
    fn start_lyrics(&mut self) {
        let Some(track) = self.current().cloned() else {
            return;
        };
        self.lyric_serial += 1;
        let serial = self.lyric_serial;
        let generation = self.generation;
        self.lyric_status = "loading".into();
        self.lyric_error.clear();
        let provider = Provider::new(self.settings.direct || self.direct_override);
        let tx = self.tx.clone();
        self.workers.push(thread::spawn(move || {
            let result = provider.lyrics(&track);
            let _ = tx.send(Event::Lyrics(generation, serial, result));
        }));
    }
    fn begin(&mut self, index: usize, quality: String) -> Result<()> {
        if index >= self.queue.len() {
            return Err(Error::new("队列序号无效"));
        }
        if !QUALITIES.contains(&quality.as_str()) {
            return Err(Error::new("音质无效"));
        }
        self.stop();
        self.index = index;
        self.requested_quality = quality.clone();
        self.phase = "loading".into();
        self.quality.clear();
        self.received = 0;
        self.total = 0;
        self.retry_attempt = 1;
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = self.cancelled.clone();
        let generation = self.generation;
        let tx = self.tx.clone();
        let track = self.current().unwrap().clone();
        let provider = Provider::new(self.settings.direct || self.direct_override);
        self.lyric_text = self.store.lyric(&track.id)?;
        if lyrics::parse(&self.lyric_text).is_empty() {
            self.lyric_text.clear();
            self.start_lyrics();
        } else {
            self.lyric_status = "ready".into();
            self.lyric_error.clear();
        }
        let target = self
            .paths
            .home
            .join("cache-rust")
            .join(format!("play-{}-{generation}", std::process::id()));
        self.persist()?;
        self.workers.push(thread::spawn(move || {
            let result = (|| {
                if !track.path.is_empty() {
                    return Ok((PathBuf::from(&track.path), "local".into()));
                }
                let mut last_error = Error::new("音源不可用");
                for attempt in 1..=2 {
                    let _ = tx.send(Event::Attempt(generation, attempt));
                    let resolved = provider.stream(&track, &quality, &cancelled);
                    match resolved {
                        Ok(stream) => {
                            let result = provider.download(
                                &stream,
                                &target,
                                false,
                                &cancelled,
                                |received, total| {
                                    let _ = tx.send(Event::Buffer(generation, received, total));
                                },
                            );
                            match result {
                                Ok(_) => return Ok((target.clone(), stream.quality)),
                                Err(error) => {
                                    last_error = error;
                                }
                            }
                        }
                        Err(error) => last_error = error,
                    }
                    if last_error.code != "network" || cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                }
                Err(last_error)
            })();
            if cancelled.load(Ordering::Relaxed) {
                let _ = fs::remove_file(&target);
                return;
            }
            if tx.send(Event::Ready(generation, result)).is_err() {
                let _ = fs::remove_file(&target);
            }
        }));
        Ok(())
    }
    fn advance(&mut self, previous: bool, automatic: bool) -> Result<()> {
        if self.queue.is_empty() {
            return Ok(());
        }
        let index = if self.settings.mode == "shuffle" {
            if previous {
                self.shuffle
                    .previous()
                    .ok_or_else(|| Error::new("没有上一首播放记录"))?
            } else {
                self.shuffle.next(self.queue.len(), self.index)
            }
        } else if automatic && self.settings.mode == "single" {
            self.index
        } else if previous {
            (self.index + self.queue.len() - 1) % self.queue.len()
        } else if automatic && self.settings.mode == "list" && self.index + 1 == self.queue.len() {
            self.stop();
            return Ok(());
        } else {
            (self.index + 1) % self.queue.len()
        };
        self.begin(index, self.settings.quality.clone())
    }
    fn enqueue_job(&mut self, request: &Value) -> Result<()> {
        let track = if request["track"].is_object() {
            serde_json::from_value(request["track"].clone())?
        } else {
            self.current()
                .cloned()
                .ok_or_else(|| Error::new("请先选择歌曲"))?
        };
        if !track.path.is_empty() {
            return Err(Error::new("该歌曲已经在本地"));
        }
        let quality = request["quality"]
            .as_str()
            .unwrap_or(&self.settings.quality)
            .to_string();
        if !QUALITIES.contains(&quality.as_str()) {
            return Err(Error::new("音质无效"));
        }
        let directory = request["directory"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| {
                self.paths
                    .home
                    .parent()
                    .unwrap_or(&self.paths.home)
                    .join("downloads")
                    .to_string_lossy()
                    .into()
            });
        let save_lyrics = request["lyrics"].as_bool().unwrap_or(true);
        if self.jobs.iter().any(|j| {
            j.track.id == track.id
                && j.requested_quality == quality
                && j.directory == directory
                && j.lyrics == save_lyrics
                && ["queued", "resolving", "downloading", "lyrics", "cancelling"]
                    .contains(&j.phase.as_str())
        }) {
            return Ok(());
        }
        if self
            .jobs
            .iter()
            .filter(|j| {
                ["queued", "resolving", "downloading", "lyrics", "cancelling"]
                    .contains(&j.phase.as_str())
            })
            .count()
            >= 10
        {
            return Err(Error::new("最多同时保留 10 项未完成下载"));
        }
        self.job_sequence += 1;
        let id = format!("d{}", self.job_sequence);
        self.jobs.push(Job {
            id: id.clone(),
            track,
            phase: "queued".into(),
            requested_quality: quality,
            directory,
            lyrics: save_lyrics,
            overwrite: request["force"].as_bool().unwrap_or(false),
            ..Default::default()
        });
        self.job_controls.insert(
            id.clone(),
            JobControl {
                cancelled: Arc::new(AtomicBool::new(false)),
                serial: 1,
            },
        );
        self.job_queue.push_back(id);
        self.kick_download();
        Ok(())
    }
    fn kick_download(&mut self) {
        if self.job_active.is_some() {
            return;
        }
        let Some(id) = self.job_queue.pop_front() else {
            return;
        };
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        if job.phase != "queued" {
            self.kick_download();
            return;
        }
        job.phase = "resolving".into();
        let job = job.clone();
        let control = &self.job_controls[&id];
        let serial = control.serial;
        let cancelled = control.cancelled.clone();
        self.job_active = Some((id.clone(), serial));
        let tx = self.tx.clone();
        let provider = Provider::new(self.settings.direct || self.direct_override);
        self.workers.push(thread::spawn(move || {
            let result = (|| {
                let stream = provider.stream(&job.track, &job.requested_quality, &cancelled)?;
                let basename = format!(
                    "{} - {} [{}-{}]",
                    job.track.artist,
                    job.track.title,
                    job.track.id.replace(':', "-"),
                    stream.quality
                );
                let safe: String = basename
                    .chars()
                    .map(|c| {
                        if c.is_control() || "\\/:*?\"<>|".contains(c) {
                            '_'
                        } else {
                            c
                        }
                    })
                    .collect();
                let path = PathBuf::from(&job.directory).join(format!(
                    "{}{}",
                    safe.trim_matches(['.', ' ']),
                    stream.extension
                ));
                let _ = tx.send(Event::JobPhase(
                    id.clone(),
                    serial,
                    "downloading".into(),
                    stream.quality.clone(),
                ));
                provider.download(
                    &stream,
                    &path,
                    job.overwrite,
                    &cancelled,
                    |received, total| {
                        let _ = tx.send(Event::JobProgress(id.clone(), serial, received, total));
                    },
                )?;
                let mut warning = String::new();
                if job.lyrics && !cancelled.load(Ordering::Relaxed) {
                    let _ = tx.send(Event::JobPhase(
                        id.clone(),
                        serial,
                        "lyrics".into(),
                        String::new(),
                    ));
                    match provider.lyrics(&job.track) {
                        Ok(text) if !lyrics::parse(&text).is_empty() => {
                            let write_lyric = (|| -> std::io::Result<()> {
                                let mut options = OpenOptions::new();
                                options.write(true);
                                if job.overwrite {
                                    options.create(true).truncate(true);
                                } else {
                                    options.create_new(true);
                                }
                                options
                                    .open(path.with_extension("lrc"))?
                                    .write_all(text.as_bytes())
                            })();
                            if write_lyric.is_err() {
                                warning = "音频已保存，歌词文件已存在或无法写入".into();
                            }
                        }
                        _ => warning = "音频已保存，未取得歌词".into(),
                    }
                }
                Ok((path.to_string_lossy().into_owned(), warning))
            })();
            let _ = tx.send(Event::JobDone(id, serial, result));
        }));
    }
    fn dispatch(&mut self, request: Value) -> Result<Value> {
        let op = request["op"].as_str().unwrap_or_default();
        match op {
            "status" => {}
            "play" => {
                self.direct_override = request["direct"].as_bool().unwrap_or(false);
                if let Some(values) = request["tracks"].as_array().filter(|v| !v.is_empty()) {
                    let tracks: Vec<Track> = values
                        .iter()
                        .map(|v| serde_json::from_value(v.clone()).map_err(Into::into))
                        .collect::<Result<_>>()?;
                    self.store.remember(&tracks)?;
                    self.queue = tracks;
                    self.index = 0;
                    self.shuffle.reset(self.queue.len(), 0);
                }
                let index = request["index"]
                    .as_u64()
                    .map(|v| v as usize)
                    .unwrap_or(self.index);
                if index >= self.queue.len() {
                    return Err(Error::new("队列序号无效"));
                }
                self.shuffle.reset(self.queue.len(), index);
                self.begin(
                    index,
                    request["quality"]
                        .as_str()
                        .unwrap_or(&self.settings.quality)
                        .to_string(),
                )?;
            }
            "retry" => {
                if self.phase != "loading" {
                    self.begin(
                        self.index,
                        request["quality"]
                            .as_str()
                            .unwrap_or(&self.requested_quality)
                            .to_string(),
                    )?;
                }
            }
            "lyrics_retry" => {
                if self.lyric_status != "loading" {
                    self.start_lyrics();
                }
            }
            "pause" | "resume" | "toggle" => {
                if self.phase == "loading" && op == "toggle" {
                    self.stop();
                } else if ["playing", "paused"].contains(&self.phase.as_str()) {
                    let paused = op == "pause" || (op == "toggle" && self.phase == "playing");
                    self.player.pause(paused);
                    self.phase = if paused { "paused" } else { "playing" }.into();
                } else {
                    return Err(Error::new("当前没有可暂停或继续的音频"));
                }
            }
            "stop" => self.stop(),
            "next" => self.advance(false, false)?,
            "prev" => self.advance(true, false)?,
            "seek" | "seek_to" => {
                if !["playing", "paused"].contains(&self.phase.as_str()) {
                    return Err(Error::new("当前没有可跳转的音频"));
                }
                if let Some(id) = request["track_id"].as_str() {
                    if self.current().map(|t| t.id.as_str()) != Some(id) {
                        return Ok(serde_json::to_value(self.status())?);
                    }
                }
                let value = request[if op == "seek" { "seconds" } else { "position" }]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| Error::new("无效的跳转位置"))?;
                self.player.seek(
                    value
                        + if op == "seek" {
                            self.player.position()
                        } else {
                            0.
                        },
                )?;
            }
            "volume" | "mode" | "config" => {
                let key = if op == "config" {
                    request["key"].as_str().unwrap_or_default()
                } else {
                    op
                };
                let mut settings = self.settings.clone();
                match key {
                    "volume" => {
                        settings.volume = request["value"]
                            .as_u64()
                            .filter(|v| *v <= 100)
                            .ok_or_else(|| Error::new("音量应为 0–100"))?
                            as u8;
                    }
                    "quality" => {
                        settings.quality = request["value"]
                            .as_str()
                            .ok_or_else(|| Error::new("音质无效"))?
                            .into()
                    }
                    "mode" => {
                        settings.mode = request["value"]
                            .as_str()
                            .ok_or_else(|| Error::new("播放方式无效"))?
                            .into()
                    }
                    "direct" => {
                        settings.direct = request["value"]
                            .as_bool()
                            .ok_or_else(|| Error::new("direct 应为 true 或 false"))?
                    }
                    "lyric_offset" => {
                        settings.lyric_offset = request["value"]
                            .as_i64()
                            .ok_or_else(|| Error::new("歌词偏移应为毫秒整数"))?
                    }
                    _ => return Err(Error::new("未知配置项")),
                }
                if !MODES.contains(&settings.mode.as_str()) {
                    return Err(Error::new("播放方式无效"));
                }
                self.paths.save_settings(&settings)?;
                if settings.mode != self.settings.mode {
                    self.shuffle.reset(self.queue.len(), self.index);
                }
                self.player.volume(settings.volume);
                self.settings = settings;
            }
            "like" => {
                let track = if request["track"].is_object() {
                    serde_json::from_value(request["track"].clone())?
                } else {
                    self.current()
                        .cloned()
                        .ok_or_else(|| Error::new("请先选择歌曲"))?
                };
                self.store.favorite(&track)?;
            }
            "queue_add" => {
                let tracks: Vec<Track> = serde_json::from_value(request["tracks"].clone())?;
                self.store.remember(&tracks)?;
                if request["next"].as_bool().unwrap_or(false) {
                    let at = (self.index + 1).min(self.queue.len());
                    self.queue.splice(at..at, tracks);
                } else {
                    self.queue.extend(tracks);
                }
                self.shuffle.reset(self.queue.len(), self.index);
                self.persist()?;
            }
            "queue_remove" => {
                let index = request["index"]
                    .as_u64()
                    .ok_or_else(|| Error::new("队列序号无效"))?
                    as usize;
                if index >= self.queue.len() {
                    return Err(Error::new("队列序号无效"));
                }
                if request["track_id"]
                    .as_str()
                    .is_some_and(|id| id != self.queue[index].id)
                {
                    return Err(Error::new("队列已变化，请重新选择歌曲"));
                }
                let current = index == self.index;
                self.queue.remove(index);
                if index < self.index {
                    self.index -= 1;
                }
                if self.queue.is_empty() {
                    self.index = 0;
                    self.stop();
                } else if current {
                    self.begin(
                        self.index.min(self.queue.len() - 1),
                        self.settings.quality.clone(),
                    )?;
                }
                self.shuffle.reset(self.queue.len(), self.index);
                self.persist()?;
            }
            "queue_clear" => {
                self.stop();
                self.queue.clear();
                self.index = 0;
                self.shuffle.reset(0, 0);
                self.persist()?;
            }
            "download" => self.enqueue_job(&request)?,
            "download_cancel" | "download_retry" => {
                let id = request["id"].as_str().unwrap_or_default();
                let job = self
                    .jobs
                    .iter_mut()
                    .find(|j| j.id == id)
                    .ok_or_else(|| Error::new("找不到下载任务"))?;
                let control = self.job_controls.get_mut(id).unwrap();
                if op == "download_cancel" {
                    if job.phase == "queued" {
                        control.cancelled.store(true, Ordering::Relaxed);
                        job.phase = "cancelled".into();
                    } else if ["resolving", "downloading", "lyrics"].contains(&job.phase.as_str()) {
                        control.cancelled.store(true, Ordering::Relaxed);
                        job.phase = "cancelling".into();
                    }
                } else if ["error", "cancelled"].contains(&job.phase.as_str()) {
                    control.serial += 1;
                    control.cancelled = Arc::new(AtomicBool::new(false));
                    job.phase = "queued".into();
                    job.received = 0;
                    job.total = 0;
                    job.error.clear();
                    job.warning.clear();
                    self.job_queue.push_back(id.into());
                }
                self.kick_download();
            }
            "shutdown" => {
                self.stop();
                self.quit.store(true, Ordering::Relaxed);
            }
            _ => return Err(Error::new("未知后台操作")),
        }
        Ok(serde_json::to_value(self.status())?)
    }
    fn event(&mut self, event: Event) {
        match event {
            Event::Call(call) => {
                let result = self.dispatch(call.value);
                let _ = call.reply.send(result);
            }
            Event::Ready(generation, result) => {
                if generation != self.generation {
                    if let Ok((path, _)) = result {
                        if path.starts_with(self.paths.home.join("cache-rust")) {
                            let _ = fs::remove_file(path);
                        }
                    }
                    return;
                }
                let result = result.and_then(|(path, quality)| {
                    let track = self
                        .current()
                        .cloned()
                        .ok_or_else(|| Error::new("队列为空"))?;
                    if track.path.is_empty() {
                        self.cache = Some(path.clone());
                    }
                    self.player.load(&path, &track, self.settings.volume)?;
                    self.store.played(&track)?;
                    self.quality = quality;
                    Ok(())
                });
                match result {
                    Ok(()) => self.phase = "playing".into(),
                    Err(error) => {
                        self.phase = "error".into();
                        self.error = Some(error);
                    }
                }
            }
            Event::Lyrics(generation, serial, result) => {
                if generation != self.generation || serial != self.lyric_serial {
                    return;
                }
                match result {
                    Ok(text) if !lyrics::parse(&text).is_empty() => {
                        if let Some(track) = self.current() {
                            let _ = self.store.save_lyric(&track.id, &text);
                        }
                        self.lyric_text = text;
                        self.lyric_status = "ready".into();
                        self.lyric_error.clear();
                    }
                    other => {
                        self.lyric_error = other
                            .err()
                            .map(|e| e.message)
                            .unwrap_or_else(|| "暂无匹配歌词".into());
                        self.lyric_status = if self.lyric_text.is_empty() {
                            "unavailable"
                        } else {
                            "ready"
                        }
                        .into();
                    }
                }
            }
            Event::Buffer(generation, received, total) => {
                if generation == self.generation {
                    self.received = received;
                    self.total = total;
                }
            }
            Event::Attempt(generation, attempt) => {
                if generation == self.generation {
                    self.retry_attempt = attempt;
                }
            }
            Event::JobProgress(id, serial, received, total) => {
                if self
                    .job_controls
                    .get(&id)
                    .is_some_and(|c| c.serial == serial)
                {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.received = received;
                        job.total = total;
                    }
                }
            }
            Event::JobPhase(id, serial, phase, quality) => {
                if self
                    .job_controls
                    .get(&id)
                    .is_some_and(|c| c.serial == serial && !c.cancelled.load(Ordering::Relaxed))
                {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.phase = phase;
                        if !quality.is_empty() {
                            job.quality = quality;
                        }
                    }
                }
            }
            Event::JobDone(id, serial, result) => {
                if self.job_active == Some((id.clone(), serial)) {
                    self.job_active = None;
                }
                if self
                    .job_controls
                    .get(&id)
                    .is_some_and(|c| c.serial == serial)
                {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        match result {
                            Ok((path, warning)) => {
                                job.phase = "done".into();
                                job.path = path;
                                job.warning = warning;
                            }
                            Err(error) => {
                                job.phase = if error.code == "cancelled" {
                                    "cancelled"
                                } else {
                                    "error"
                                }
                                .into();
                                job.error = error.message;
                            }
                        }
                    }
                }
                self.kick_download();
            }
        }
    }
}
fn handle(mut stream: UnixStream, tx: Sender<Event>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let result = (|| -> Result<Value> {
        let mut line = String::new();
        BufReader::new(stream.try_clone()?)
            .take(2 * 1024 * 1024)
            .read_line(&mut line)?;
        let value = serde_json::from_str(&line)?;
        let (reply, rx) = mpsc::channel();
        tx.send(Event::Call(Call { value, reply }))
            .map_err(|_| Error::new("后台服务正在退出"))?;
        rx.recv_timeout(Duration::from_secs(12))
            .map_err(|_| Error::new("后台响应超时"))?
    })();
    let response = match result {
        Ok(data) => json!({"ok":true,"data":data}),
        Err(error) => {
            json!({"ok":false,"error":error.message,"code":error.code,"details":error.details})
        }
    };
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let _ = serde_json::to_writer(&mut stream, &response);
    let _ = stream.write_all(b"\n");
}
pub fn run(paths: Paths) -> Result<()> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.runtime.join("service.lock"))?;
    if lock.try_lock_exclusive().is_err() {
        return Ok(());
    }
    let _ = fs::remove_file(paths.socket());
    let listener = UnixListener::bind(paths.socket())?;
    fs::set_permissions(paths.socket(), fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let quit = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, quit.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, quit.clone())?;
    let (tx, rx): (Sender<Event>, Receiver<Event>) = mpsc::channel();
    let mut service = Service::new(paths.clone(), tx.clone(), quit.clone())?;
    let listening = quit.clone();
    let acceptor = thread::spawn(move || {
        while !listening.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let tx = tx.clone();
                    thread::spawn(move || handle(stream, tx));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(15))
                }
                Err(_) => break,
            }
        }
    });
    while !quit.load(Ordering::Relaxed) {
        if let Ok(event) = rx.recv_timeout(Duration::from_millis(30)) {
            service.event(event);
        }
        if service.phase == "playing" && service.player.finished() {
            if let Err(error) = service.advance(false, true) {
                service.phase = "error".into();
                service.error = Some(error);
            }
        }
        service.workers.retain(|w| !w.is_finished());
    }
    service.stop();
    for control in service.job_controls.values() {
        control.cancelled.store(true, Ordering::Relaxed);
    }
    let _ = service.persist();
    let _ = fs::remove_file(paths.socket());
    for worker in service.workers {
        let _ = worker.join();
    }
    let _ = acceptor.join();
    eprintln!("{} 后台已退出", now());
    Ok(())
}
