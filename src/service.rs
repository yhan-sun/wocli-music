use crate::{
    ipc::Stream,
    lyrics,
    model::{Error, Result, Settings, Track, MODES, QUALITIES},
    order::Shuffle,
    player::{AudioInput, Player, PreparedAudio},
    provider::Provider,
    store::{now, Paths, Store},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const MAX_PENDING_DOWNLOADS: usize = 10;
const DOWNLOAD_HISTORY_LIMIT: usize = 100;

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
    pub direct: bool,
}
impl Job {
    fn is_pending(&self) -> bool {
        ["queued", "resolving", "downloading", "lyrics", "cancelling"]
            .contains(&self.phase.as_str())
    }
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
    pub source: String,
    pub source_failures: Vec<String>,
    pub lyrics: String,
    pub lyrics_status: String,
    pub lyrics_error: String,
    pub lyric_offset: i64,
    pub received: u64,
    pub total: u64,
    pub downloads: Vec<Job>,
    pub lyrics_bar: bool,
    pub tray: bool,
    pub boss: bool,
    pub buffering: bool,
    pub spectrum: [f32; 8],
    pub buffered_seconds: f64,
    pub buffer_underruns: u64,
    pub audio_device: String,
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
    StreamReady(u64, Result<(String, String, crate::stream::StreamHandle)>),
    StreamFinished(u64, Result<u64>),
    Loaded(u64, Result<PreparedAudio>),
    Reattached(u64, u64, bool, Result<PreparedAudio>),
    SeekReady(u64, u64, Result<PreparedAudio>),
    Lyrics(u64, u64, Result<String>),
    Buffer(u64, u64, u64),
    Attempt(u64, u8),
    Source(u64, String, HashSet<String>),
    SourceFailure(u64, String, String),
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
    pause_after_load: bool,
    quality: String,
    requested_quality: String,
    error: Option<Error>,
    received: u64,
    total: u64,
    retry_attempt: u8,
    current_source: String,
    rejected_sources: HashSet<String>,
    source_failures: Vec<String>,
    lyric_text: String,
    lyric_lines: Vec<lyrics::Line>,
    lyric_status: String,
    lyric_error: String,
    lyric_serial: u64,
    lyric_session: u128,
    lyric_cancelled: Arc<AtomicBool>,
    generation: u64,
    direct_override: bool,
    cancelled: Arc<AtomicBool>,
    tx: Sender<Event>,
    shuffle: Shuffle,
    cache: Option<PathBuf>,
    audio_input: Option<AudioInput>,
    prepare_cancelled: Arc<AtomicBool>,
    reattach_serial: u64,
    reattach_pending: bool,
    seek_cancelled: Arc<AtomicBool>,
    seek_serial: u64,
    pending_seek: Option<(Instant, Sender<Result<Value>>)>,
    stream_retries: u8,
    failed_indices: HashSet<usize>,
    jobs: Vec<Job>,
    job_controls: HashMap<String, JobControl>,
    job_queue: VecDeque<String>,
    job_active: Option<(String, u64)>,
    job_sequence: u64,
    pool: crate::pool::WorkerPool,
    quit: Arc<AtomicBool>,
    boss: bool,
    boss_previous_phase: Option<String>,
    buffering_since: Option<Instant>,
    last_device_check: Instant,
    visualizer_seen: Option<Instant>,
    play_token: Option<String>,
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
            pause_after_load: false,
            quality: String::new(),
            error: None,
            received: 0,
            total: 0,
            retry_attempt: 1,
            current_source: String::new(),
            rejected_sources: HashSet::new(),
            source_failures: vec![],
            lyric_text: String::new(),
            lyric_lines: vec![],
            lyric_status: "idle".into(),
            lyric_error: String::new(),
            lyric_serial: 0,
            lyric_session: rand::random(),
            lyric_cancelled: Arc::new(AtomicBool::new(false)),
            generation: 0,
            direct_override: false,
            cancelled: Arc::new(AtomicBool::new(false)),
            tx,
            shuffle,
            cache: None,
            audio_input: None,
            prepare_cancelled: Arc::new(AtomicBool::new(false)),
            reattach_serial: 0,
            reattach_pending: false,
            seek_cancelled: Arc::new(AtomicBool::new(false)),
            seek_serial: 0,
            pending_seek: None,
            stream_retries: 0,
            failed_indices: HashSet::new(),
            jobs: vec![],
            job_controls: HashMap::new(),
            job_queue: VecDeque::new(),
            job_active: None,
            job_sequence: 0,
            pool: crate::pool::WorkerPool::new(4),
            quit,
            boss: false,
            boss_previous_phase: None,
            buffering_since: None,
            last_device_check: Instant::now(),
            visualizer_seen: None,
            play_token: None,
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
            lyrics_bar: self.settings.lyrics_bar,
            tray: self.settings.tray,
            boss: self.boss,
            buffering: self.phase == "playing" && self.player.buffering(),
            spectrum: if self.phase == "playing" && !self.boss {
                self.player
                    .spectrum()
                    .map(|v| v * self.settings.volume as f32 / 100.)
            } else {
                [0.; 8]
            },
            buffered_seconds: self.player.buffered_seconds(),
            buffer_underruns: self.player.underruns(),
            audio_device: self
                .player
                .stream_device_name()
                .unwrap_or_default()
                .to_string(),
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
            source: self.current_source.clone(),
            source_failures: self.source_failures.clone(),
            downloads: self.jobs.clone(),
        }
    }
    // The system component needs one current line, not the queue, download
    // history or full LRC on every poll. Keep concealed/stopped metadata out of
    // this response, even if a provider has a stale display request in flight.
    fn lyrics_snapshot(&self) -> Value {
        let visible = self.settings.lyrics_bar
            && !self.boss
            && matches!(self.phase.as_str(), "playing" | "paused");
        let track = self.current().filter(|_| visible);
        let clip = |text: &str| text.chars().take(240).collect::<String>();
        let line = if track.is_some() {
            lyrics::current(
                &self.lyric_lines,
                self.player.position(),
                self.settings.lyric_offset,
            )
            .and_then(|index| self.lyric_lines.get(index))
            .map(|line| clip(line.text.trim()))
            .unwrap_or_default()
        } else {
            String::new()
        };
        json!({
            "enabled": self.settings.lyrics_bar,
            "boss": self.boss,
            "state": self.phase,
            "track_id": track.map(|track| format!("{:032x}:{}:{}", self.lyric_session, self.generation, track.id.chars().take(256).collect::<String>())).unwrap_or_default(),
            "title": track.map(|track| clip(&track.title)).unwrap_or_default(),
            "artist": track.map(|track| clip(&track.artist)).unwrap_or_default(),
            "line": line,
        })
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
        self.lyric_cancelled.store(true, Ordering::Relaxed);
        self.prepare_cancelled.store(true, Ordering::Relaxed);
        self.cancel_reattach();
        self.cancel_seek("播放已变化，已取消跳转");
        if let Some(coord) = self
            .audio_input
            .as_ref()
            .and_then(|input| input.stream.as_ref())
        {
            coord.mark_cancelled();
        }
        self.audio_input = None;
        self.buffering_since = None;
        self.generation += 1;
        self.player.stop();
        self.phase = "stopped".into();
        self.pause_after_load = false;
        self.error = None;
        if self.lyric_status == "loading" {
            self.lyric_status = if self.lyric_text.is_empty() {
                "idle"
            } else {
                "ready"
            }
            .into();
            self.lyric_error.clear();
        }
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
        self.lyric_cancelled.store(true, Ordering::Relaxed);
        self.lyric_cancelled = Arc::new(AtomicBool::new(false));
        let provider = Provider::new(self.settings.direct || self.direct_override);
        let tx = self.tx.clone();
        let cancelled = self.lyric_cancelled.clone();
        if !self.pool.execute(move || {
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
            let result = provider.lyrics(&track);
            let _ = tx.send(Event::Lyrics(generation, serial, result));
        }) {
            self.lyric_status = "error".into();
            self.lyric_error = worker_busy().message;
        }
    }
    fn begin(&mut self, index: usize, quality: String) -> Result<()> {
        self.begin_paused(index, quality, false)
    }
    fn begin_paused(&mut self, index: usize, quality: String, paused: bool) -> Result<()> {
        self.source_failures.clear();
        self.begin_attempt(index, quality, HashSet::new(), paused)
    }
    fn begin_attempt(
        &mut self,
        index: usize,
        quality: String,
        rejected: HashSet<String>,
        paused: bool,
    ) -> Result<()> {
        if index >= self.queue.len() {
            return Err(Error::new("队列序号无效"));
        }
        if !QUALITIES.contains(&quality.as_str()) {
            return Err(Error::new("音质无效"));
        }
        self.stop();
        self.stream_retries = 0;
        self.current_source.clear();
        self.rejected_sources = rejected.clone();
        self.index = index;
        self.requested_quality = quality.clone();
        self.phase = "loading".into();
        self.pause_after_load = paused;
        self.quality.clear();
        self.received = 0;
        self.total = 0;
        self.retry_attempt = 1;
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = self.cancelled.clone();
        let generation = self.generation;
        let tx = self.tx.clone();
        let track = self.current().unwrap().clone();
        let provider = Provider::with_sources(
            self.settings.direct || self.direct_override,
            self.paths.home.clone(),
        );
        self.lyric_text = self.store.lyric(&track.id)?;
        self.lyric_lines = lyrics::parse(&self.lyric_text);
        if self.lyric_lines.is_empty() {
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
        if !self.pool.execute(move || {
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
            let result = (|| {
                if !track.path.is_empty() {
                    return Ok(Some((PathBuf::from(&track.path), "local".into())));
                }
                let mut last_error = Error::new("音源不可用");
                let mut rejected = rejected;
                for attempt in 1..=32 {
                    let _ = tx.send(Event::Attempt(generation, attempt));
                    let resolved = provider.resolve(&track, &quality, cancelled.clone(), &rejected);
                    match resolved {
                        Ok((source, stream)) => {
                            let _ = tx.send(Event::Source(
                                generation,
                                source.clone(),
                                rejected.clone(),
                            ));
                            rejected.insert(source.clone());
                            if stream.url.starts_with("http://")
                                || stream.url.starts_with("https://")
                            {
                                let stream_tx = tx.clone();
                                let on_buffer = move |received, total| {
                                    let _ =
                                        stream_tx.send(Event::Buffer(generation, received, total));
                                };
                                let complete_tx = tx.clone();
                                // Publish readiness before completion. Failed prebuffer attempts
                                // must not race a later source with a stale completion event.
                                let completion = Arc::new(std::sync::Mutex::new((false, None)));
                                let complete_state = completion.clone();
                                let on_complete = move |result| {
                                    let mut state = complete_state.lock().unwrap();
                                    if state.0 {
                                        let _ = complete_tx
                                            .send(Event::StreamFinished(generation, result));
                                    } else {
                                        state.1 = Some(result);
                                    }
                                };
                                match crate::stream::start_stream_buffered(
                                    provider.agent(),
                                    &stream.url,
                                    stream.size,
                                    track.duration,
                                    target.clone(),
                                    cancelled.clone(),
                                    (on_buffer, on_complete),
                                ) {
                                    Ok(stream_handle) => {
                                        let _ = tx.send(Event::StreamReady(
                                            generation,
                                            Ok((
                                                stream.quality.clone(),
                                                stream.extension.clone(),
                                                stream_handle,
                                            )),
                                        ));
                                        let mut state = completion.lock().unwrap();
                                        state.0 = true;
                                        if let Some(result) = state.1.take() {
                                            let _ =
                                                tx.send(Event::StreamFinished(generation, result));
                                        }
                                        return Ok(None);
                                    }
                                    Err(_) => {
                                        // Streaming failed, fallback below
                                    }
                                }
                            }
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
                                Ok(_) => return Ok(Some((target.clone(), stream.quality))),
                                Err(error) => {
                                    let _ = tx.send(Event::SourceFailure(
                                        generation,
                                        source,
                                        error.message.clone(),
                                    ));
                                    last_error = error;
                                }
                            }
                        }
                        Err(error) => {
                            last_error = error;
                            break;
                        }
                    }
                    if cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                }
                Err(last_error)
            })();
            if cancelled.load(Ordering::Relaxed) {
                let _ = fs::remove_file(&target);
                return;
            }
            let result = match result {
                Ok(Some(value)) => Ok(value),
                Ok(None) => return,
                Err(error) => Err(error),
            };
            if tx.send(Event::Ready(generation, result)).is_err() {
                let _ = fs::remove_file(&target);
            }
        }) {
            let error = worker_busy();
            self.loading_rejected(error.clone());
            return Err(error);
        }
        Ok(())
    }
    fn loading_rejected(&mut self, error: Error) {
        self.stop();
        self.phase = "error".into();
        self.error = Some(error);
    }
    fn advance(&mut self, previous: bool, automatic: bool) -> Result<()> {
        if self.queue.is_empty() {
            let favs = self.store.favorites().unwrap_or_default();
            if !favs.is_empty() {
                self.queue = favs;
                self.index = 0;
                self.shuffle.reset(self.queue.len(), self.index);
                return self.begin(0, self.settings.quality.clone());
            }
            return Ok(());
        }
        if automatic {
            self.failed_indices.clear();
        }
        let index = if self.settings.mode == "shuffle" {
            if previous {
                if let Some(index) = self.shuffle.previous() {
                    index
                } else {
                    let index = (self.index + self.queue.len() - 1) % self.queue.len();
                    self.shuffle.reset(self.queue.len(), index);
                    index
                }
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
    fn prepare_input(&mut self, input: AudioInput) {
        self.prepare_cancelled.store(true, Ordering::Relaxed);
        self.prepare_cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = self.prepare_cancelled.clone();
        self.audio_input = Some(input.clone());
        let track = self.current().unwrap().clone();
        let null = self.player.null;
        let generation = self.generation;
        let tx = self.tx.clone();
        if !self.pool.execute(move || {
            let result = input.prepare(&track, 0., cancelled, null);
            let _ = tx.send(Event::Loaded(generation, result));
        }) {
            self.loading_rejected(worker_busy());
        }
    }
    fn reattach_player(&mut self, target_paused: bool) -> Result<()> {
        let position = self.player.position();
        let input = self
            .audio_input
            .clone()
            .ok_or_else(|| Error::new("当前没有可播放的音频"))?;
        let track = self
            .current()
            .cloned()
            .ok_or_else(|| Error::new("当前没有可播放的音频"))?;
        self.cancel_reattach();
        self.cancel_seek("音频设备已变化，已取消跳转");
        self.prepare_cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = self.prepare_cancelled.clone();
        self.reattach_serial += 1;
        self.reattach_pending = true;
        let serial = self.reattach_serial;
        let generation = self.generation;
        let null = self.player.null;
        let tx = self.tx.clone();
        if !self.pool.execute(move || {
            let result = input.prepare(&track, position, cancelled, null);
            let _ = tx.send(Event::Reattached(generation, serial, target_paused, result));
        }) {
            self.cancel_reattach();
            return Err(worker_busy());
        }
        Ok(())
    }
    fn cancel_reattach(&mut self) {
        if self.reattach_pending {
            self.prepare_cancelled.store(true, Ordering::Relaxed);
            self.reattach_serial += 1;
            self.reattach_pending = false;
        }
    }
    fn cancel_seek(&mut self, message: &str) {
        if let Some((_, reply)) = self.pending_seek.take() {
            self.seek_cancelled.store(true, Ordering::Relaxed);
            self.seek_serial += 1;
            let _ = reply.send(Err(Error::coded("cancelled", message)));
        }
    }
    fn seek_request(&mut self, request: Value, reply: Sender<Result<Value>>) {
        self.play_token = None;
        let result = (|| {
            if !["playing", "paused"].contains(&self.phase.as_str()) {
                return Err(Error::new("当前没有可跳转的音频"));
            }
            if request["track_id"]
                .as_str()
                .is_some_and(|id| self.current().map(|t| t.id.as_str()) != Some(id))
            {
                let _ = reply.send(Ok(serde_json::to_value(self.status())?));
                return Ok(());
            }
            let relative = request["op"] == "seek";
            let value = request[if relative { "seconds" } else { "position" }]
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| Error::new("无效的跳转位置"))?;
            let position = value + if relative { self.player.position() } else { 0. };
            if !position.is_finite() {
                return Err(Error::new("无效的跳转位置"));
            }
            let input = self
                .audio_input
                .clone()
                .ok_or_else(|| Error::new("当前没有可跳转的音频"))?;
            let track = self.current().unwrap().clone();
            self.cancel_reattach();
            self.cancel_seek("已取消上一次跳转");
            self.seek_cancelled = Arc::new(AtomicBool::new(false));
            let cancelled = self.seek_cancelled.clone();
            self.seek_serial += 1;
            let serial = self.seek_serial;
            let generation = self.generation;
            let tx = self.tx.clone();
            self.pending_seek = Some((Instant::now() + Duration::from_secs(10), reply.clone()));
            let null = self.player.null;
            if !self.pool.execute(move || {
                let result = input.prepare(&track, position, cancelled, null);
                let _ = tx.send(Event::SeekReady(generation, serial, result));
            }) {
                self.pending_seek = None;
                self.seek_cancelled.store(true, Ordering::Relaxed);
                return Err(worker_busy());
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = reply.send(Err(error));
        }
    }
    fn playback_failed(&mut self, mut error: Error) {
        let paused = self.phase == "paused" || self.pause_after_load;
        if error.code != "audio_device"
            && error.code != "cancelled"
            && self.current().is_some_and(|t| t.path.is_empty())
            && !self.current_source.is_empty()
            && self.rejected_sources.insert(self.current_source.clone())
        {
            self.source_failures
                .push(format!("{}: {}", self.current_source, error));
            let excluded = self.rejected_sources.clone();
            if self
                .begin_attempt(self.index, self.requested_quality.clone(), excluded, paused)
                .is_ok()
            {
                return;
            }
        }
        error.details.extend(self.source_failures.clone());
        eprintln!(
            "播放失败，{}: {}",
            self.current().map(|t| t.title.as_str()).unwrap_or_default(),
            error
        );
        self.failed_indices.insert(self.index);
        let next = if error.code == "audio_device" || self.failed_indices.len() >= self.queue.len()
        {
            None
        } else if self.settings.mode == "shuffle" {
            (0..self.queue.len() * 2)
                .map(|_| self.shuffle.next(self.queue.len(), self.index))
                .find(|i| !self.failed_indices.contains(i))
                .or_else(|| (0..self.queue.len()).find(|i| !self.failed_indices.contains(i)))
        } else {
            (1..self.queue.len())
                .map(|offset| self.index + offset)
                .take_while(|i| self.settings.mode != "list" || *i < self.queue.len())
                .map(|i| i % self.queue.len())
                .find(|i| !self.failed_indices.contains(i))
        };
        if let Some(index) = next {
            match self.begin_paused(index, self.settings.quality.clone(), paused) {
                Ok(()) => return,
                Err(next_error) => error = next_error,
            }
        }
        self.stop();
        self.phase = "error".into();
        if self.failed_indices.len() > 1 {
            error.details.push(format!(
                "已跳过 {} 首无法播放的歌曲",
                self.failed_indices.len()
            ));
        }
        self.error = Some(error);
    }
    fn tick(&mut self) {
        if self
            .visualizer_seen
            .is_some_and(|seen| seen.elapsed() > Duration::from_secs(2))
        {
            self.player.set_visualizer(false);
            self.visualizer_seen = None;
        }
        if self
            .pending_seek
            .as_ref()
            .is_some_and(|(deadline, _)| Instant::now() >= *deadline)
        {
            self.cancel_seek("等待跳转位置缓冲超时，请稍后重试");
        }
        if !self.player.null
            && self.phase == "playing"
            && self.last_device_check.elapsed() >= Duration::from_millis(500)
        {
            self.last_device_check = Instant::now();
            let current_dev = crate::player::current_default_device_name();
            let dev_changed = self.player.stream_device_name().is_some()
                && self.player.stream_device_name() != current_dev.as_deref();
            let dev_stalled = self.player.is_device_stalled();
            if dev_changed || dev_stalled {
                eprintln!(
                    "音频输出设备已变动或中断（{} -> {}），自动暂停播放",
                    self.player.stream_device_name().unwrap_or("未知"),
                    current_dev.as_deref().unwrap_or("无可用设备")
                );
                self.player.pause(true);
                self.player.reset_stream();
                self.phase = "paused".into();
                return;
            }
        }
        if self.phase == "playing" && self.player.buffering() {
            let since = self.buffering_since.get_or_insert_with(|| {
                eprintln!(
                    "音频缓冲不足，等待供数（当前歌曲第 {} 次）",
                    self.player.underruns()
                );
                Instant::now()
            });
            if since.elapsed() >= Duration::from_secs(15) {
                self.playback_failed(Error::coded(
                    "playback_stall",
                    "音频供数持续中断超过 15 秒，尝试重试或备用音源",
                ));
                return;
            }
        } else if let Some(since) = self.buffering_since.take() {
            if self.phase == "playing" {
                eprintln!(
                    "音频缓冲恢复（等待 {:.2} 秒）",
                    since.elapsed().as_secs_f64()
                );
            }
        }
        if self.phase != "playing" || !self.player.finished() {
            return;
        }
        if let Some(coord) = self.audio_input.as_ref().and_then(|i| i.stream.as_ref()) {
            // Download failures arrive as events; an unfinished or invalid body
            // must not masquerade as a natural end and reset the skip history.
            if !coord.completed.load(Ordering::Acquire) || coord.failed.load(Ordering::Acquire) {
                return;
            }
        }
        if let Some(error) = self.player.end_error() {
            self.playback_failed(error);
            return;
        }
        if let Err(error) = self.advance(false, true) {
            self.playback_failed(error);
        }
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
        if directory.trim().is_empty() {
            return Err(Error::coded("download_directory", "下载目录不能为空"));
        }
        let directory = crate::cli::expand_path(directory.trim());
        let directory = if directory.is_absolute() {
            directory
        } else {
            std::env::current_dir()?.join(directory)
        };
        let directory = directory.to_string_lossy().into_owned();
        let save_lyrics = request["lyrics"].as_bool().unwrap_or(true);
        let direct = self.settings.direct
            || self.direct_override
            || request["direct"].as_bool().unwrap_or(false);
        if self.jobs.iter().any(|j| {
            j.track.id == track.id
                && j.requested_quality == quality
                && j.directory == directory
                && j.lyrics == save_lyrics
                && j.direct == direct
                && j.is_pending()
        }) {
            return Ok(());
        }
        if self.jobs.iter().filter(|j| j.is_pending()).count() >= MAX_PENDING_DOWNLOADS {
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
            direct,
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
    fn prune_download_history(&mut self) {
        let mut excess = self
            .jobs
            .iter()
            .filter(|job| !job.is_pending())
            .count()
            .saturating_sub(DOWNLOAD_HISTORY_LIMIT);
        if excess == 0 {
            return;
        }
        let mut removed = HashSet::new();
        self.jobs.retain(|job| {
            if excess > 0 && !job.is_pending() {
                excess -= 1;
                removed.insert(job.id.clone());
                false
            } else {
                true
            }
        });
        self.job_controls.retain(|id, _| !removed.contains(id));
        self.job_queue.retain(|id| !removed.contains(id));
    }
    fn kick_download(&mut self) {
        if self.job_active.is_some() {
            return;
        }
        // Cancellations and expired history must not block later queued jobs,
        // nor recurse through stale entries on the service thread's stack.
        let index = loop {
            let Some(id) = self.job_queue.pop_front() else {
                return;
            };
            if let Some(index) = self
                .jobs
                .iter()
                .position(|job| job.id == id && job.phase == "queued")
            {
                break index;
            }
        };
        let job = &mut self.jobs[index];
        let id = job.id.clone();
        job.phase = "resolving".into();
        let job = job.clone();
        let control = &self.job_controls[&id];
        let serial = control.serial;
        let cancelled = control.cancelled.clone();
        self.job_active = Some((id.clone(), serial));
        let tx = self.tx.clone();
        let provider = Provider::with_sources(job.direct, self.paths.home.clone());
        let rejected_id = id.clone();
        if !self.pool.execute(move || {
            let result = (|| {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(Error::coded("cancelled", "已取消"));
                }
                prepare_download_directory(&job.directory)?;
                let mut excluded = HashSet::new();
                let path = loop {
                    let (source, stream) = provider.resolve(
                        &job.track,
                        &job.requested_quality,
                        cancelled.clone(),
                        &excluded,
                    )?;
                    excluded.insert(source);
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
                    if path.exists() && !job.overwrite {
                        return Err(Error::new("文件已存在，覆盖需 --force"));
                    }
                    let staging = tempfile::Builder::new()
                        .prefix(".wocli-download-")
                        .tempdir_in(&job.directory)?;
                    let candidate = staging.path().join(format!("audio{}", stream.extension));
                    let _ = tx.send(Event::JobPhase(
                        id.clone(),
                        serial,
                        "downloading".into(),
                        stream.quality.clone(),
                    ));
                    let downloaded = provider
                        .download(&stream, &candidate, false, &cancelled, |received, total| {
                            let _ =
                                tx.send(Event::JobProgress(id.clone(), serial, received, total));
                        })
                        .and_then(|bytes| {
                            // An endpoint can pass a probe and later return a preview/error body.
                            let validation = AudioInput {
                                path: candidate.clone(),
                                hint: stream.extension.trim_start_matches('.').into(),
                                stream: None,
                            }
                            .prepare(
                                &job.track,
                                0.,
                                cancelled.clone(),
                                true,
                            );
                            match validation {
                                Ok(audio) => {
                                    drop(audio);
                                    if cancelled.load(Ordering::Relaxed) {
                                        return Err(Error::coded("cancelled", "已取消"));
                                    }
                                    if job.overwrite {
                                        fs::rename(&candidate, &path)?;
                                    } else {
                                        fs::hard_link(&candidate, &path)?;
                                    }
                                    Ok(bytes)
                                }
                                Err(error) => Err(error),
                            }
                        });
                    match downloaded {
                        Ok(_) => break path,
                        Err(error)
                            if ["network", "incomplete_audio", "decode_error"]
                                .contains(&error.code.as_str()) =>
                        {
                            continue
                        }
                        Err(error) => return Err(error),
                    }
                };
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
        }) {
            self.event(Event::JobDone(rejected_id, serial, Err(worker_busy())));
        }
    }
    fn dispatch(&mut self, request: Value) -> Result<Value> {
        let from_widget = request["op"] == "widget_action";
        let op = if from_widget {
            let action = request["action"].as_str().unwrap_or_default();
            if !["prev", "next", "pause", "resume"].contains(&action) {
                return Err(Error::coded("invalid_widget_action", "不支持的小组件操作"));
            }
            // Validate and execute in this same service event. A card can remain
            // cached by Windows after the track, playback state or privacy mode
            // changes; its old buttons must not control the new playback.
            let snapshot = self.lyrics_snapshot();
            if !self.settings.lyrics_bar
                || self.boss
                || !matches!(self.phase.as_str(), "playing" | "paused")
                || snapshot["track_id"].as_str().unwrap_or_default().is_empty()
                || request["track_id"] != snapshot["track_id"]
                || request["state"].as_str() != Some(self.phase.as_str())
                || (action == "pause" && self.phase != "playing")
                || (action == "resume" && self.phase != "paused")
            {
                return Err(Error::coded(
                    "stale_widget_action",
                    "播放或隐私状态已变化，请等待小组件刷新",
                ));
            }
            action
        } else {
            request["op"].as_str().unwrap_or_default()
        };
        if op == "prepare_play" {
            let token = format!("{:032x}", rand::random::<u128>());
            self.play_token = Some(token.clone());
            return Ok(json!({"play_token": token}));
        }
        if op == "play"
            && request.get("play_token").is_some()
            && (request["play_token"].as_str().is_none()
                || request["play_token"].as_str() != self.play_token.as_deref())
        {
            return Err(Error::coded("cancelled", "播放操作已变化，已取消过期推荐"));
        }
        if [
            "play",
            "pause",
            "resume",
            "toggle",
            "stop",
            "next",
            "prev",
            "retry",
            "queue_add",
            "queue_remove",
            "queue_clear",
            "boss",
            "shutdown",
        ]
        .contains(&op)
        {
            self.play_token = None;
        }
        if [
            "play",
            "retry",
            "next",
            "prev",
            "queue_add",
            "queue_remove",
            "queue_clear",
        ]
        .contains(&op)
        {
            self.failed_indices.clear();
        }
        match op {
            "lyrics_snapshot" => return Ok(self.lyrics_snapshot()),
            "status" => {
                if request["visualizer"].as_bool() == Some(true) {
                    self.player.set_visualizer(true);
                    self.visualizer_seen = Some(Instant::now());
                }
            }
            "play" => {
                // Validate the entire replacement before changing the current
                // queue: the old audio and its metadata must stay together.
                let tracks: Option<Vec<Track>> = request
                    .get("tracks")
                    .map(|value| serde_json::from_value(value.clone()))
                    .transpose()?;
                let index = match request.get("index") {
                    Some(value) => value
                        .as_u64()
                        .and_then(|v| usize::try_from(v).ok())
                        .ok_or_else(|| Error::new("队列序号无效"))?,
                    None => {
                        if tracks.is_some() {
                            0
                        } else {
                            self.index
                        }
                    }
                };
                if index >= tracks.as_ref().unwrap_or(&self.queue).len() {
                    return Err(Error::new("队列序号无效"));
                }
                let quality = match request.get("quality").filter(|value| !value.is_null()) {
                    Some(value) => value.as_str().ok_or_else(|| Error::new("音质无效"))?,
                    None => &self.settings.quality,
                }
                .to_string();
                if !QUALITIES.contains(&quality.as_str()) {
                    return Err(Error::new("音质无效"));
                }
                if let Some(tracks) = tracks {
                    self.store.remember(&tracks)?;
                    self.queue = tracks;
                }
                self.boss = false;
                self.boss_previous_phase = None;
                self.direct_override = request["direct"].as_bool().unwrap_or(false);
                self.shuffle.reset(self.queue.len(), index);
                self.begin(index, quality)?;
            }
            "retry" => {
                if self.phase != "loading" {
                    if let Some(direct) = request["direct"].as_bool() {
                        self.direct_override = direct;
                    }
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
                if self.boss {
                    // Media keys must not restart sound while the screen is concealed.
                    // An explicit pause/stop also cancels automatic resume on exit.
                    if op == "pause" {
                        self.boss_previous_phase = None;
                    }
                    return Ok(serde_json::to_value(self.status())?);
                }
                if self.phase == "loading" && op == "resume" {
                    // Resume may arrive while a paused track is being retried
                    // or replaced. Apply it to the pending load as well.
                    self.pause_after_load = false;
                } else if self.phase == "loading" && (op == "toggle" || op == "pause") {
                    self.stop();
                } else if ["playing", "paused"].contains(&self.phase.as_str()) {
                    let paused = op == "pause" || (op == "toggle" && self.phase == "playing");
                    if paused {
                        self.cancel_reattach();
                        self.player.pause(true);
                        self.phase = "paused".into();
                    } else {
                        let need_reattach = if self.player.null {
                            false
                        } else {
                            let current_dev = crate::player::current_default_device_name();
                            let dev_changed = self.player.stream_device_name().is_some()
                                && self.player.stream_device_name() != current_dev.as_deref();
                            self.player.stream_is_none() || dev_changed
                        };
                        if need_reattach && self.audio_input.is_some() && self.current().is_some() {
                            self.reattach_player(false)?;
                        } else {
                            self.player.pause(false);
                            self.phase = "playing".into();
                        }
                    }
                } else if op == "pause" && self.phase == "stopped" {
                    // 已经处于停止状态，无需重复报错
                } else if (op == "toggle" || op == "resume") && self.phase == "stopped" {
                    if !self.queue.is_empty() {
                        let idx = self.index.min(self.queue.len().saturating_sub(1));
                        self.begin(idx, self.settings.quality.clone())?;
                    } else {
                        let favs = self.store.favorites().unwrap_or_default();
                        if !favs.is_empty() {
                            self.queue = favs;
                            self.index = 0;
                            self.shuffle.reset(self.queue.len(), self.index);
                            self.begin(0, self.settings.quality.clone())?;
                        } else {
                            return Err(Error::new("当前没有可暂停或继续的音频"));
                        }
                    }
                } else if (op == "toggle" || op == "resume") && self.phase == "error" {
                    if !self.queue.is_empty() {
                        let idx = self.index.min(self.queue.len().saturating_sub(1));
                        self.begin(idx, self.settings.quality.clone())?;
                    } else {
                        return Err(Error::new("当前没有可暂停或继续的音频"));
                    }
                } else {
                    return Err(Error::new("当前没有可暂停或继续的音频"));
                }
            }
            "stop" => {
                self.boss_previous_phase = None;
                self.stop();
            }
            "next" => {
                self.boss = false;
                self.boss_previous_phase = None;
                self.advance(false, false)?;
            }
            "prev" => {
                self.boss = false;
                self.boss_previous_phase = None;
                self.advance(true, false)?;
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
                    "lyrics_bar" => {
                        settings.lyrics_bar = request["value"]
                            .as_bool()
                            .ok_or_else(|| Error::new("lyrics_bar 应为 true 或 false"))?
                    }
                    "tray" => {
                        settings.tray = request["value"]
                            .as_bool()
                            .ok_or_else(|| Error::new("tray 应为 true 或 false"))?
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
                let active = ["playing", "paused", "loading"].contains(&self.phase.as_str());
                let paused = self.phase == "paused" || self.pause_after_load;
                self.queue.remove(index);
                if index < self.index {
                    self.index -= 1;
                }
                if self.queue.is_empty() {
                    self.index = 0;
                    self.stop();
                } else if current {
                    self.index = self.index.min(self.queue.len() - 1);
                    if active {
                        self.begin_paused(self.index, self.settings.quality.clone(), paused)?;
                    } else {
                        self.stop();
                    }
                }
                if current && self.phase == "stopped" {
                    self.lyric_text.clear();
                    self.lyric_lines.clear();
                    self.lyric_status = "idle".into();
                    self.lyric_error.clear();
                    self.quality.clear();
                }
                self.shuffle.reset(self.queue.len(), self.index);
                self.persist()?;
            }
            "queue_clear" => {
                self.boss_previous_phase = None;
                self.stop();
                self.queue.clear();
                self.lyric_text.clear();
                self.lyric_lines.clear();
                self.lyric_status = "idle".into();
                self.lyric_error.clear();
                self.index = 0;
                self.shuffle.reset(0, 0);
                self.persist()?;
            }
            "download" => self.enqueue_job(&request)?,
            "download_cancel" | "download_retry" => {
                let id = request["id"].as_str().unwrap_or_default();
                let index = self
                    .jobs
                    .iter()
                    .position(|j| j.id == id)
                    .ok_or_else(|| Error::new("找不到下载任务"))?;
                if op == "download_retry"
                    && ["error", "cancelled"].contains(&self.jobs[index].phase.as_str())
                    && self.jobs.iter().filter(|job| job.is_pending()).count()
                        >= MAX_PENDING_DOWNLOADS
                {
                    return Err(Error::new("最多同时保留 10 项未完成下载"));
                }
                let job = &mut self.jobs[index];
                let control = self.job_controls.get_mut(id).unwrap();
                if op == "download_cancel" {
                    if job.phase == "queued" {
                        control.cancelled.store(true, Ordering::Relaxed);
                        job.phase = "cancelled".into();
                        self.job_queue.retain(|queued| queued != id);
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
                    self.job_queue.retain(|queued| queued != id);
                    self.job_queue.push_back(id.into());
                }
                self.prune_download_history();
                self.kick_download();
            }
            "boss" => {
                if request.get("active").is_some_and(|v| !v.is_boolean()) {
                    return Err(Error::new("老板模式 active 应为 true 或 false"));
                }
                let target = if let Some(v) = request["active"].as_bool() {
                    v
                } else {
                    !self.boss
                };
                if target == self.boss {
                    return Ok(serde_json::to_value(self.status())?);
                }
                self.boss = target;
                if self.boss {
                    self.cancel_reattach();
                    if self.phase == "playing" {
                        self.boss_previous_phase = Some("playing".into());
                        self.player.pause(true);
                        self.phase = "paused".into();
                    } else if self.phase == "loading" {
                        // A background retry can be preparing audio that must
                        // stay paused. Preserve that intent across stop().
                        self.boss_previous_phase = Some(
                            if self.pause_after_load {
                                "paused_loading"
                            } else {
                                "loading"
                            }
                            .into(),
                        );
                        self.stop();
                    } else {
                        self.boss_previous_phase = Some(self.phase.clone());
                    }
                } else if let Some(prev) = self.boss_previous_phase.take() {
                    if prev == "playing" {
                        if self.phase == "paused" {
                            let need_reattach = if self.player.null {
                                false
                            } else {
                                let current_dev = crate::player::current_default_device_name();
                                let dev_changed = self.player.stream_device_name().is_some()
                                    && self.player.stream_device_name() != current_dev.as_deref();
                                self.player.stream_is_none() || dev_changed
                            };
                            if need_reattach
                                && self.audio_input.is_some()
                                && self.current().is_some()
                            {
                                self.reattach_player(false)?;
                            } else {
                                self.player.pause(false);
                                self.phase = "playing".into();
                            }
                        }
                    } else if prev == "loading" {
                        if self.phase == "paused" {
                            let need_reattach = if self.player.null {
                                false
                            } else {
                                let current_dev = crate::player::current_default_device_name();
                                let dev_changed = self.player.stream_device_name().is_some()
                                    && self.player.stream_device_name() != current_dev.as_deref();
                                self.player.stream_is_none() || dev_changed
                            };
                            if need_reattach
                                && self.audio_input.is_some()
                                && self.current().is_some()
                            {
                                self.reattach_player(false)?;
                            } else {
                                self.player.pause(false);
                                self.phase = "playing".into();
                            }
                        } else if self.phase == "stopped" && !self.queue.is_empty() {
                            self.begin(self.index, self.requested_quality.clone())?;
                        }
                    } else if prev == "paused_loading"
                        && self.phase == "stopped"
                        && !self.queue.is_empty()
                    {
                        self.begin_paused(self.index, self.requested_quality.clone(), true)?;
                    }
                }
            }
            "shutdown" => {
                self.stop();
                self.quit.store(true, Ordering::Relaxed);
            }
            _ => return Err(Error::new("未知后台操作")),
        }
        if from_widget {
            Ok(self.lyrics_snapshot())
        } else {
            Ok(serde_json::to_value(self.status())?)
        }
    }
    fn event(&mut self, event: Event) {
        match event {
            Event::Call(call) => {
                if matches!(call.value["op"].as_str(), Some("seek" | "seek_to")) {
                    self.seek_request(call.value, call.reply);
                } else {
                    let result = self.dispatch(call.value);
                    let _ = call.reply.send(result);
                }
            }
            Event::StreamReady(generation, result) => {
                if generation != self.generation {
                    if let Ok((_, _, handle)) = result {
                        handle.coord.mark_cancelled();
                        let _ = fs::remove_file(handle.cache_path);
                    }
                    return;
                }
                match result {
                    Ok((quality, hint, handle)) => {
                        self.cache = Some(handle.cache_path.clone());
                        self.quality = quality;
                        self.prepare_input(AudioInput {
                            path: handle.cache_path,
                            hint,
                            stream: Some(handle.coord),
                        });
                    }
                    Err(error) => self.playback_failed(error),
                }
            }
            Event::StreamFinished(generation, result) => {
                if generation != self.generation {
                    return;
                }
                if let Err(error) = result {
                    if error.code == "network" && self.stream_retries == 0 {
                        let quality = self.requested_quality.clone();
                        let paused = self.phase == "paused" || self.pause_after_load;
                        if let Err(error) = self.begin_attempt(
                            self.index,
                            quality,
                            self.rejected_sources.clone(),
                            paused,
                        ) {
                            self.playback_failed(error);
                        } else {
                            self.stream_retries = 1;
                        }
                    } else if error.code != "cancelled" {
                        self.playback_failed(error);
                    }
                }
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
                match result {
                    Ok((path, quality)) => {
                        if self.current().is_some_and(|t| t.path.is_empty()) {
                            self.cache = Some(path.clone());
                        }
                        self.quality = quality;
                        let hint = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or_default()
                            .to_string();
                        self.prepare_input(AudioInput {
                            path,
                            hint,
                            stream: None,
                        });
                    }
                    Err(error) => self.playback_failed(error),
                }
            }
            Event::Loaded(generation, result) => {
                if generation != self.generation {
                    return;
                }
                let paused = self.boss || self.pause_after_load;
                let result =
                    result.and_then(|audio| self.player.load(audio, self.settings.volume, paused));
                self.prepare_cancelled = Arc::new(AtomicBool::new(false));
                match result {
                    Ok(()) => {
                        if let Some(track) = self.current() {
                            let _ = self.store.played(track);
                        }
                        self.phase = if paused { "paused" } else { "playing" }.into();
                        self.pause_after_load = false;
                        self.error = None;
                    }
                    Err(error) => self.playback_failed(error),
                }
            }
            Event::Reattached(generation, serial, target_paused, result) => {
                if generation != self.generation
                    || serial != self.reattach_serial
                    || !self.reattach_pending
                {
                    return;
                }
                self.reattach_pending = false;
                self.prepare_cancelled = Arc::new(AtomicBool::new(false));
                let paused = target_paused || self.boss;
                let result =
                    result.and_then(|audio| self.player.load(audio, self.settings.volume, paused));
                match result {
                    Ok(()) => {
                        self.phase = if paused { "paused" } else { "playing" }.into();
                        self.error = None;
                    }
                    Err(error) => self.playback_failed(error),
                }
            }
            Event::SeekReady(generation, serial, result) => {
                if generation != self.generation || serial != self.seek_serial {
                    return;
                }
                let Some((_, reply)) = self.pending_seek.take() else {
                    return;
                };
                // Once applied, the source owns this token. Future seeks cancel
                // pending preparation, not the currently playing source.
                self.seek_cancelled = Arc::new(AtomicBool::new(false));
                let result = result
                    .and_then(|audio| {
                        if let Err(error) = self.player.load(
                            audio,
                            self.settings.volume,
                            self.phase == "paused" || self.boss,
                        ) {
                            self.playback_failed(error.clone());
                            return Err(error);
                        }
                        Ok(())
                    })
                    .and_then(|()| Ok(serde_json::to_value(self.status())?));
                let _ = reply.send(result);
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
                        self.lyric_lines = lyrics::parse(&self.lyric_text);
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
            Event::Source(generation, source, rejected) => {
                if generation == self.generation {
                    self.current_source = source;
                    self.rejected_sources = rejected;
                }
            }
            Event::SourceFailure(generation, source, message) => {
                if generation == self.generation {
                    self.rejected_sources.insert(source.clone());
                    self.source_failures.push(format!("{source}: {message}"));
                    self.current_source.clear();
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
                                job.error = if error.details.is_empty() {
                                    error.message
                                } else {
                                    format!("{}；{}", error.message, error.details.join("；"))
                                };
                            }
                        }
                    }
                }
                self.prune_download_history();
                self.kick_download();
            }
        }
    }
}
fn worker_busy() -> Error {
    Error::coded("worker_busy", "后台任务过多，请稍后重试")
}
fn prepare_download_directory(directory: &str) -> Result<()> {
    (|| -> std::io::Result<()> {
        fs::create_dir_all(directory)?;
        let _probe = tempfile::NamedTempFile::new_in(directory)?;
        Ok(())
    })()
    .map_err(|error| {
        Error::coded(
            "download_directory",
            format!("下载目录不可写 {directory}: {error}"),
        )
    })
}
fn handle(mut stream: Stream, tx: Sender<Event>) {
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
    let quit = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    {
        signal_hook::flag::register(signal_hook::consts::SIGTERM, quit.clone())?;
        signal_hook::flag::register(signal_hook::consts::SIGINT, quit.clone())?;
    }
    let (tx, rx): (Sender<Event>, Receiver<Event>) = mpsc::channel();
    let mut service = Service::new(paths.clone(), tx.clone(), quit.clone())?;
    let listening = quit.clone();
    let limiter = Arc::new(crate::pool::ConnectionLimiter::new(16));

    #[cfg(unix)]
    let acceptor = {
        let listener = std::os::unix::net::UnixListener::bind(paths.socket())?;
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(paths.socket(), fs::Permissions::from_mode(0o600));
        listener.set_nonblocking(true)?;
        let tx = tx.clone();
        let limiter_clone = limiter.clone();
        thread::spawn(move || {
            while !listening.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        if let Some(guard) = limiter_clone.acquire() {
                            let tx = tx.clone();
                            thread::spawn(move || {
                                let _guard = guard;
                                handle(Stream::Unix(stream), tx);
                            });
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(15))
                    }
                    Err(_) => break,
                }
            }
        })
    };

    #[cfg(windows)]
    let acceptor = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let token: String = (0..32)
            .map(|_| format!("{:02x}", rand::random::<u8>()))
            .collect();
        fs::write(paths.socket(), format!("{port}\n{token}\n"))?;
        listener.set_nonblocking(true)?;
        let tx = tx.clone();
        let expected_token = token;
        let limiter_clone = limiter.clone();
        thread::spawn(move || {
            while !listening.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_nodelay(true);
                        if let Some(guard) = limiter_clone.acquire() {
                            let tx = tx.clone();
                            let token_auth = expected_token.clone();
                            thread::spawn(move || {
                                let _guard = guard;
                                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                                let mut bytes = Vec::new();
                                let mut b = [0u8; 1];
                                let mut ok = false;
                                while stream.read(&mut b).is_ok_and(|n| n > 0) {
                                    if b[0] == b'\n' {
                                        ok = true;
                                        break;
                                    }
                                    if b[0] != b'\r' {
                                        bytes.push(b[0]);
                                    }
                                    if bytes.len() > 128 {
                                        break;
                                    }
                                }
                                if ok && String::from_utf8_lossy(&bytes) == token_auth {
                                    handle(Stream::Tcp(stream), tx);
                                }
                            });
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(15))
                    }
                    Err(_) => break,
                }
            }
        })
    };
    while !quit.load(Ordering::Relaxed) {
        if let Ok(event) = rx.recv_timeout(Duration::from_millis(30)) {
            service.event(event);
        }
        service.tick();
    }
    service.stop();
    for control in service.job_controls.values() {
        control.cancelled.store(true, Ordering::Relaxed);
    }
    let _ = service.persist();
    let _ = fs::remove_file(paths.socket());
    drop(service);
    let _ = acceptor.join();
    eprintln!("{} 后台已退出", now());
    Ok(())
}

#[cfg(test)]
mod playback_tests {
    use super::*;
    use std::time::Instant;
    struct Harness {
        service: Service,
        rx: Receiver<Event>,
        dir: tempfile::TempDir,
    }
    impl Harness {
        fn new(mode: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("data")).unwrap();
            let (tx, rx) = mpsc::channel();
            let mut service = Service::new(
                Paths {
                    home: dir.path().join("data"),
                    runtime: dir.path().join("run"),
                },
                tx,
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            service.player.null = true;
            service.settings.mode = mode.into();
            Self { service, rx, dir }
        }
        fn track(&self, name: &str, good: bool) -> Track {
            let path = self.dir.path().join(format!("{name}.wav"));
            fs::write(
                &path,
                if good {
                    crate::player::test_wav(30)
                } else {
                    b"not audio".to_vec()
                },
            )
            .unwrap();
            fs::write(path.with_extension("lrc"), "[00:00]test").unwrap();
            Track::local(&path).unwrap()
        }
        fn pump(&mut self, phase: &str) {
            let deadline = Instant::now() + Duration::from_secs(4);
            while self.service.phase != phase {
                assert!(
                    Instant::now() < deadline,
                    "wanted {phase}, got {}",
                    self.service.phase
                );
                if let Ok(event) = self.rx.recv_timeout(Duration::from_millis(20)) {
                    self.service.event(event);
                }
                self.service.tick();
            }
        }
        fn reattach_event(&mut self) -> Event {
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                assert!(Instant::now() < deadline, "audio reattach timed out");
                if let Ok(event) = self.rx.recv_timeout(Duration::from_millis(20)) {
                    if matches!(event, Event::Reattached(..)) {
                        return event;
                    }
                    self.service.event(event);
                }
            }
        }
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            self.service.stop();
        }
    }
    struct BusyPool(Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);
    impl BusyPool {
        fn fill(pool: &crate::pool::WorkerPool) -> Self {
            let gate = Self(Arc::new((
                std::sync::Mutex::new(false),
                std::sync::Condvar::new(),
            )));
            let (tx, rx) = mpsc::channel();
            for _ in 0..4 {
                let state = gate.0.clone();
                let tx = tx.clone();
                assert!(pool.execute(move || {
                    let _ = tx.send(());
                    let (lock, wake) = &*state;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        released = wake.wait(released).unwrap();
                    }
                }));
            }
            for _ in 0..4 {
                rx.recv_timeout(Duration::from_secs(2)).unwrap();
            }
            let mut queued = 0;
            while pool.execute(|| {}) {
                queued += 1;
                assert!(queued < 1_000, "worker queue must be bounded");
            }
            gate
        }
    }
    impl Drop for BusyPool {
        fn drop(&mut self) {
            let (lock, wake) = &*self.0;
            *lock.lock().unwrap() = true;
            wake.notify_all();
        }
    }

    #[test]
    fn widget_buttons_from_a_previous_daemon_session_are_rejected() {
        let mut first = Harness::new("list");
        let mut restarted = Harness::new("list");
        let mut track = first.track("widget-restart", true);
        track.id = "歌".repeat(256);
        for harness in [&mut first, &mut restarted] {
            harness.service.queue = vec![track.clone()];
            harness.service.phase = "playing".into();
            harness.service.generation = u64::MAX;
        }
        let old = first.service.lyrics_snapshot();
        let current = restarted.service.lyrics_snapshot();
        assert_ne!(old["track_id"], current["track_id"]);
        assert!(current["track_id"].as_str().unwrap().chars().count() <= 320);
        let error = restarted.service.dispatch(json!({
            "op":"widget_action", "action":"pause", "track_id":old["track_id"], "state":"playing",
        })).unwrap_err();
        assert_eq!(error.code, "stale_widget_action");
        assert_eq!(restarted.service.phase, "playing");
    }

    #[test]
    fn widget_buttons_apply_once_to_the_displayed_track_and_state() {
        let mut h = Harness::new("list");
        let tracks = [
            h.track("widget-first", true),
            h.track("widget-second", true),
        ];
        h.service
            .dispatch(json!({"op":"play", "tracks":tracks}))
            .unwrap();
        h.pump("playing");
        let action = |snapshot: &Value, action: &str| {
            json!({
                "op":"widget_action", "action":action,
                "track_id":snapshot["track_id"], "state":snapshot["state"],
            })
        };
        let first = h.service.lyrics_snapshot();
        let paused = h.service.dispatch(action(&first, "pause")).unwrap();
        assert_eq!(paused["state"], "paused");
        assert!(paused.get("queue").is_none());
        assert!(paused.get("lyrics").is_none());
        assert_eq!(
            h.service
                .dispatch(action(&first, "pause"))
                .unwrap_err()
                .code,
            "stale_widget_action"
        );
        let resumed = h.service.dispatch(action(&paused, "resume")).unwrap();
        assert_eq!(resumed["state"], "playing");
        h.service.dispatch(action(&resumed, "next")).unwrap();
        h.pump("playing");
        assert_eq!(h.service.index, 1);
        assert_eq!(
            h.service
                .dispatch(action(&resumed, "next"))
                .unwrap_err()
                .code,
            "stale_widget_action"
        );
        assert_eq!(h.service.index, 1);
        let second = h.service.lyrics_snapshot();
        h.service.dispatch(action(&second, "prev")).unwrap();
        h.pump("playing");
        assert_eq!(h.service.index, 0);
        // Returning to the same song does not revalidate an old card.
        assert_eq!(
            h.service
                .dispatch(action(&first, "pause"))
                .unwrap_err()
                .code,
            "stale_widget_action"
        );
    }

    #[test]
    fn widget_buttons_cannot_bypass_privacy_or_execute_arbitrary_operations() {
        let mut h = Harness::new("list");
        let track = h.track("widget-private", true);
        h.service
            .dispatch(json!({"op":"play", "tracks":[track]}))
            .unwrap();
        h.pump("playing");
        let snapshot = h.service.lyrics_snapshot();
        for action in ["toggle", "shutdown", "play", "boss", "config", ""] {
            let error = h
                .service
                .dispatch(json!({"op":"widget_action", "action":action,
                "track_id":snapshot["track_id"], "state":snapshot["state"]}))
                .unwrap_err();
            assert_eq!(error.code, "invalid_widget_action");
        }
        assert!(!h.service.quit.load(Ordering::Relaxed));
        assert_eq!(h.service.phase, "playing");
        for change in [
            json!({"op":"boss", "active":true}),
            json!({"op":"config", "key":"lyrics_bar", "value":false}),
            json!({"op":"stop"}),
        ] {
            h.service
                .dispatch(json!({"op":"boss", "active":false}))
                .unwrap();
            h.service
                .dispatch(json!({"op":"config", "key":"lyrics_bar", "value":true}))
                .unwrap();
            h.service.dispatch(change).unwrap();
            let state = h.service.phase.clone();
            let boss = h.service.boss;
            let play_token = h.service.dispatch(json!({"op":"prepare_play"})).unwrap();
            for action in ["prev", "next", "pause", "resume"] {
                let error = h
                    .service
                    .dispatch(json!({"op":"widget_action", "action":action,
                    "track_id":snapshot["track_id"], "state":snapshot["state"]}))
                    .unwrap_err();
                assert_eq!(error.code, "stale_widget_action");
            }
            assert_eq!(h.service.phase, state);
            assert_eq!(h.service.boss, boss);
            assert_eq!(
                h.service.play_token.as_deref(),
                play_token["play_token"].as_str()
            );
        }
        assert_eq!(
            h.service
                .dispatch(json!({"op":"widget_action", "action":"next"}))
                .unwrap_err()
                .code,
            "stale_widget_action"
        );
    }

    #[test]
    fn system_lyrics_snapshot_tracks_offsets_and_conceals_inactive_metadata() {
        let mut h = Harness::new("list");
        let mut track = h.track("system-widget", true);
        track.title = "歌".repeat(400);
        h.service
            .store
            .save_lyric(&track.id, "[00:00]第一句\n[00:10]第二句")
            .unwrap();
        h.service
            .dispatch(json!({"op":"play", "tracks":[track.clone()]}))
            .unwrap();
        h.pump("playing");
        let first = h.service.dispatch(json!({"op":"lyrics_snapshot"})).unwrap();
        assert_eq!(first["line"], "第一句");
        assert_eq!(first["title"].as_str().unwrap().chars().count(), 240);
        assert!(first.get("queue").is_none());
        assert!(first.get("lyrics").is_none());
        h.service
            .dispatch(json!({"op":"config", "key":"lyric_offset", "value":10000}))
            .unwrap();
        h.service.dispatch(json!({"op":"pause"})).unwrap();
        let paused = h.service.dispatch(json!({"op":"lyrics_snapshot"})).unwrap();
        assert_eq!(paused["line"], "第二句");
        assert_eq!(paused["state"], "paused");
        h.service
            .dispatch(json!({"op":"boss", "active":true}))
            .unwrap();
        let concealed = h.service.dispatch(json!({"op":"lyrics_snapshot"})).unwrap();
        assert_eq!(concealed["boss"], true);
        for field in ["track_id", "title", "artist", "line"] {
            assert_eq!(concealed[field], "");
        }
        h.service
            .dispatch(json!({"op":"boss", "active":false}))
            .unwrap();
        h.service
            .dispatch(json!({"op":"config", "key":"lyrics_bar", "value":false}))
            .unwrap();
        assert_eq!(h.service.lyrics_snapshot()["line"], "");
        h.service
            .dispatch(json!({"op":"config", "key":"lyrics_bar", "value":true}))
            .unwrap();
        h.service.dispatch(json!({"op":"stop"})).unwrap();
        assert_eq!(h.service.lyrics_snapshot()["title"], "");
        h.service
            .dispatch(json!({"op":"play", "tracks":[track]}))
            .unwrap();
        h.pump("playing");
        assert_ne!(h.service.lyrics_snapshot()["track_id"], first["track_id"]);
    }

    #[test]
    fn system_lyrics_cache_accepts_current_results_and_rejects_old_tracks() {
        let mut h = Harness::new("list");
        let track = h.track("system-widget-cache", true);
        h.service
            .store
            .save_lyric(&track.id, "[00:00]cached")
            .unwrap();
        h.service
            .dispatch(json!({"op":"play", "tracks":[track]}))
            .unwrap();
        h.pump("playing");
        let generation = h.service.generation;
        let serial = h.service.lyric_serial;
        h.service
            .event(Event::Lyrics(generation, serial, Ok("[00:00]fresh".into())));
        assert_eq!(h.service.lyrics_snapshot()["line"], "fresh");
        h.service.event(Event::Lyrics(
            generation.wrapping_sub(1),
            serial,
            Ok("[00:00]stale".into()),
        ));
        assert_eq!(h.service.lyrics_snapshot()["line"], "fresh");
        h.service.dispatch(json!({"op":"queue_clear"})).unwrap();
        assert!(h.service.lyric_lines.is_empty());
        assert_eq!(h.service.lyrics_snapshot()["line"], "");
    }

    #[test]
    fn a_full_worker_queue_reports_errors_instead_of_sticking_in_loading() {
        let mut h = Harness::new("list");
        let track = h.track("busy", true);
        let _busy = BusyPool::fill(&h.service.pool);
        let error = h
            .service
            .dispatch(json!({"op":"play", "tracks":[track]}))
            .unwrap_err();
        assert_eq!(error.code, "worker_busy");
        assert_eq!(h.service.phase, "error");
        h.service.start_lyrics();
        assert_eq!(h.service.lyric_status, "error");
        h.service
            .enqueue_job(&json!({"track":Track { id:"kuwo:busy".into(), ..Default::default() }}))
            .unwrap();
        assert_eq!(h.service.jobs[0].phase, "error");
        assert!(h.service.jobs[0].error.contains("后台任务过多"));
        assert!(h.service.job_active.is_none());
    }
    #[test]
    fn a_full_worker_queue_rejects_seek_and_reattach_without_stale_pending_flags() {
        let mut h = Harness::new("list");
        let track = h.track("busy-seek", true);
        h.service
            .dispatch(json!({"op":"play", "tracks":[track]}))
            .unwrap();
        h.pump("playing");
        let _busy = BusyPool::fill(&h.service.pool);
        let (reply, rx) = mpsc::channel();
        h.service
            .seek_request(json!({"op":"seek_to", "position":10}), reply);
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(100))
                .unwrap()
                .unwrap_err()
                .code,
            "worker_busy"
        );
        assert!(h.service.pending_seek.is_none());
        assert_eq!(h.service.phase, "playing");
        assert_eq!(
            h.service.reattach_player(false).unwrap_err().code,
            "worker_busy"
        );
        assert!(!h.service.reattach_pending);
        let input = h.service.audio_input.clone().unwrap();
        h.service.prepare_input(input);
        assert_eq!(h.service.phase, "error");
        assert_eq!(h.service.error.as_ref().unwrap().code, "worker_busy");
    }
    #[test]
    fn delayed_recommendations_cannot_override_a_newer_playback_action() {
        for op in [
            "play",
            "pause",
            "resume",
            "toggle",
            "stop",
            "next",
            "prev",
            "retry",
            "queue_clear",
            "boss",
        ] {
            let mut h = Harness::new("list");
            let track = h.track("current", true);
            h.service
                .dispatch(json!({"op":"play", "tracks":[track]}))
                .unwrap();
            h.pump("playing");
            let token =
                h.service.dispatch(json!({"op":"prepare_play"})).unwrap()["play_token"].clone();
            let _ = h.service.dispatch(json!({"op":op}));
            let before = serde_json::to_value(h.service.status()).unwrap();
            let error = h
                .service
                .dispatch(json!({"op":"play", "play_token":token}))
                .unwrap_err();
            assert_eq!(error.code, "cancelled", "{op}");
            let after = serde_json::to_value(h.service.status()).unwrap();
            assert_eq!(before["track"], after["track"], "{op}");
            assert_eq!(before["phase"], after["phase"], "{op}");
            assert_eq!(before["queue"], after["queue"], "{op}");
        }
        let mut h = Harness::new("list");
        let track = h.track("recommendation", true);
        let first = h.service.dispatch(json!({"op":"prepare_play"})).unwrap()["play_token"].clone();
        let token = h.service.dispatch(json!({"op":"prepare_play"})).unwrap()["play_token"].clone();
        assert_eq!(
            h.service
                .dispatch(json!({"op":"play", "play_token":first}))
                .unwrap_err()
                .code,
            "cancelled"
        );
        h.service.dispatch(json!({"op":"status"})).unwrap();
        h.service
            .dispatch(json!({"op":"play", "play_token":token, "tracks":[track]}))
            .unwrap();
        h.pump("playing");
        assert_eq!(
            h.service
                .dispatch(json!({"op":"play", "play_token":token}))
                .unwrap_err()
                .code,
            "cancelled"
        );
        let token = h.service.dispatch(json!({"op":"prepare_play"})).unwrap()["play_token"].clone();
        let (reply, _rx) = mpsc::channel();
        h.service
            .seek_request(json!({"op":"seek_to", "position":2}), reply);
        assert_eq!(
            h.service
                .dispatch(json!({"op":"play", "play_token":token}))
                .unwrap_err()
                .code,
            "cancelled"
        );
    }

    #[test]
    fn bad_tracks_are_skipped_in_every_mode_and_all_bad_queues_terminate() {
        for mode in ["list", "repeat", "single", "shuffle"] {
            let mut h = Harness::new(mode);
            let bad = h.track("bad", false);
            let bad2 = h.track("bad2", false);
            let good = h.track("good", true);
            h.service
                .dispatch(json!({"op":"play", "tracks":[bad, bad2, good]}))
                .unwrap();
            h.pump("playing");
            assert_eq!(h.service.current().unwrap().title, "good", "{mode}");
            let bad = h.track("bad", false);
            let bad2 = h.track("bad2", false);
            h.service
                .dispatch(json!({"op":"play", "tracks":[bad, bad2]}))
                .unwrap();
            h.pump("error");
            assert_eq!(h.service.failed_indices.len(), 2, "{mode}");
            assert!(h.service.error.is_some());
        }
    }
    #[test]
    fn a_device_failure_does_not_skip_the_whole_queue() {
        let mut h = Harness::new("repeat");
        h.service.queue = vec![h.track("one", true), h.track("two", true)];
        h.service
            .playback_failed(Error::coded("audio_device", "no device"));
        assert_eq!(h.service.phase, "error");
        assert_eq!(h.service.index, 0);
    }
    #[test]
    fn sustained_buffer_starvation_is_reported_and_does_not_wait_for_a_120_second_http_timeout() {
        let mut h = Harness::new("single");
        let path = h.dir.path().join("starved.wav");
        let bytes = crate::player::test_wav(20);
        fs::write(&path, &bytes[..40000]).unwrap();
        let coord = Arc::new(crate::stream::StreamCoord::new(bytes.len() as u64));
        coord.notify_progress(40000);
        let input = AudioInput {
            path,
            hint: "wav".into(),
            stream: Some(coord.clone()),
        };
        let track = Track {
            duration: 20.0,
            ..Track::default()
        };
        let audio = input
            .prepare(&track, 0.0, Arc::new(AtomicBool::new(false)), false)
            .unwrap();
        let (player, mut output) = Player::test_output(audio);
        h.service.player = player;
        h.service.audio_input = Some(input);
        h.service.queue = vec![track];
        h.service.phase = "playing".into();
        let ready = Instant::now() + Duration::from_secs(2);
        while h.service.player.buffered_seconds() < 2.25 {
            assert!(Instant::now() < ready);
            thread::sleep(Duration::from_millis(2));
        }
        for _ in 0..32000 {
            output.next();
        }
        assert!(h.service.status().buffering);
        assert_eq!(h.service.status().buffer_underruns, 1);
        h.service.buffering_since = Some(Instant::now() - Duration::from_secs(16));
        h.service.tick();
        assert_eq!(h.service.phase, "error");
        assert_eq!(h.service.error.as_ref().unwrap().code, "playback_stall");
        assert!(coord.cancelled.load(Ordering::Relaxed));
        assert!(!h.service.status().buffering);
    }
    #[test]
    fn midstream_network_failure_retries_once_then_skips() {
        let mut h = Harness::new("single");
        let one = h.track("one", true);
        let two = h.track("two", true);
        h.service
            .dispatch(json!({"op":"play", "tracks":[one, two]}))
            .unwrap();
        h.pump("playing");
        h.service.event(Event::StreamFinished(
            h.service.generation,
            Err(Error::coded("network", "broken connection")),
        ));
        assert_eq!(h.service.stream_retries, 1);
        h.pump("playing");
        assert_eq!(h.service.index, 0);
        h.service.event(Event::StreamFinished(
            h.service.generation,
            Err(Error::coded("network", "broken again")),
        ));
        h.pump("playing");
        assert_eq!(h.service.index, 1);
    }
    #[test]
    fn stalled_seek_leaves_controls_responsive_and_stop_cancels_it() {
        let mut h = Harness::new("list");
        let track = h.track("stream", true);
        let wav = crate::player::test_wav(30);
        fs::write(&track.path, &wav[..40000]).unwrap();
        let coord = Arc::new(crate::stream::StreamCoord::new(wav.len() as u64));
        coord.notify_progress(40000);
        let input = AudioInput {
            path: PathBuf::from(&track.path),
            hint: "wav".into(),
            stream: Some(coord),
        };
        let audio = input
            .prepare(&track, 0., Arc::new(AtomicBool::new(false)), true)
            .unwrap();
        h.service.player.load(audio, 0, false).unwrap();
        h.service.queue = vec![track];
        h.service.audio_input = Some(input);
        h.service.phase = "playing".into();
        let (reply, seek_rx) = mpsc::channel();
        h.service.event(Event::Call(Call {
            value: json!({"op":"seek_to","position":20}),
            reply,
        }));
        assert!(matches!(
            seek_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        let (reply, status_rx) = mpsc::channel();
        h.service.event(Event::Call(Call {
            value: json!({"op":"status"}),
            reply,
        }));
        assert_eq!(
            status_rx
                .recv_timeout(Duration::from_millis(100))
                .unwrap()
                .unwrap()["phase"],
            "playing"
        );
        h.service.dispatch(json!({"op":"stop"})).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(result) = seek_rx.try_recv() {
                assert!(result.is_err());
                break;
            }
            assert!(Instant::now() < deadline);
            if let Ok(event) = h.rx.recv_timeout(Duration::from_millis(20)) {
                h.service.event(event);
            }
        }
        assert_eq!(h.service.phase, "stopped");
    }
    #[test]
    fn repeated_midstream_failures_stop_after_every_track_has_failed() {
        let mut h = Harness::new("repeat");
        let one = h.track("one", true);
        let two = h.track("two", true);
        h.service
            .dispatch(json!({"op":"play", "tracks":[one, two]}))
            .unwrap();
        h.pump("playing");
        h.service.event(Event::StreamFinished(
            h.service.generation,
            Err(Error::coded("incomplete_audio", "truncated")),
        ));
        h.pump("playing");
        assert_eq!(h.service.index, 1);
        h.service.event(Event::StreamFinished(
            h.service.generation,
            Err(Error::coded("incomplete_audio", "truncated")),
        ));
        assert_eq!(h.service.phase, "error");
        assert_eq!(h.service.failed_indices.len(), 2);
    }
    #[test]
    fn seek_timeout_replies_promptly_and_ignores_a_late_result() {
        let mut h = Harness::new("list");
        let track = h.track("stream", true);
        let wav = crate::player::test_wav(30);
        fs::write(&track.path, &wav[..40000]).unwrap();
        let coord = Arc::new(crate::stream::StreamCoord::new(wav.len() as u64));
        coord.notify_progress(40000);
        let input = AudioInput {
            path: PathBuf::from(&track.path),
            hint: "wav".into(),
            stream: Some(coord),
        };
        h.service
            .player
            .load(
                input
                    .prepare(&track, 0., Arc::new(AtomicBool::new(false)), true)
                    .unwrap(),
                0,
                false,
            )
            .unwrap();
        h.service.queue = vec![track];
        h.service.audio_input = Some(input);
        h.service.phase = "playing".into();
        let (reply, rx) = mpsc::channel();
        h.service
            .seek_request(json!({"op":"seek_to", "position":20}), reply);
        let serial = h.service.seek_serial;
        h.service.pending_seek.as_mut().unwrap().0 = Instant::now();
        h.service.tick();
        assert!(rx
            .recv_timeout(Duration::from_millis(100))
            .unwrap()
            .unwrap_err()
            .message
            .contains("超时"));
        assert_eq!(h.service.phase, "playing");
        let local = h.track("late", true);
        let audio = AudioInput {
            path: PathBuf::from(&local.path),
            hint: "wav".into(),
            stream: None,
        }
        .prepare(&local, 20., Arc::new(AtomicBool::new(false)), true)
        .unwrap();
        h.service
            .event(Event::SeekReady(h.service.generation, serial, Ok(audio)));
        assert!(h.service.player.position() < 1.);
    }
    #[test]
    fn cancelled_download_history_stays_bounded_and_preserves_pending_jobs() {
        let mut h = Harness::new("list");
        h.service.job_active = Some(("busy".into(), 1));
        for index in 0..1_000 {
            h.service
                .enqueue_job(&json!({"track": Track {
                    id: format!("kuwo:{index}"), ..Default::default()
                }}))
                .unwrap();
            if index != 0 {
                let id = h.service.jobs.last().unwrap().id.clone();
                h.service
                    .dispatch(json!({"op":"download_cancel", "id": id}))
                    .unwrap();
            }
        }
        assert_eq!(h.service.jobs.len(), DOWNLOAD_HISTORY_LIMIT + 1);
        assert_eq!(h.service.job_controls.len(), DOWNLOAD_HISTORY_LIMIT + 1);
        assert_eq!(h.service.job_queue, VecDeque::from(["d1".to_string()]));
        assert_eq!(h.service.jobs[0].phase, "queued");
        assert_eq!(h.service.jobs.last().unwrap().id, "d1000");
    }
    #[test]
    fn download_retry_does_not_accumulate_queue_entries_or_bypass_capacity() {
        let mut h = Harness::new("list");
        h.service.job_active = Some(("busy".into(), 1));
        h.service
            .enqueue_job(&json!({"track": Track { id: "kuwo:1".into(), ..Default::default() }}))
            .unwrap();
        for _ in 0..1_000 {
            h.service
                .dispatch(json!({"op":"download_cancel", "id":"d1"}))
                .unwrap();
            assert!(h.service.job_queue.is_empty());
            h.service
                .dispatch(json!({"op":"download_retry", "id":"d1"}))
                .unwrap();
            assert_eq!(h.service.job_queue.len(), 1);
        }
        h.service
            .dispatch(json!({"op":"download_cancel", "id":"d1"}))
            .unwrap();
        for index in 2..=MAX_PENDING_DOWNLOADS + 1 {
            h.service
                .enqueue_job(&json!({"track": Track {
                    id: format!("kuwo:{index}"), ..Default::default()
                }}))
                .unwrap();
        }
        let before = h.service.job_controls["d1"].serial;
        let error = h
            .service
            .dispatch(json!({"op":"download_retry", "id":"d1"}))
            .unwrap_err();
        assert!(error.message.contains("10"));
        assert_eq!(h.service.jobs[0].phase, "cancelled");
        assert_eq!(h.service.job_controls["d1"].serial, before);
        assert_eq!(h.service.job_queue.len(), MAX_PENDING_DOWNLOADS);
    }
    #[test]
    fn completed_download_history_releases_old_controls() {
        let mut h = Harness::new("list");
        for index in 0..DOWNLOAD_HISTORY_LIMIT + 20 {
            h.service.job_active = Some(("busy".into(), 1));
            h.service
                .enqueue_job(&json!({"track": Track {
                    id: format!("kuwo:{index}"), ..Default::default()
                }}))
                .unwrap();
            let id = h.service.job_queue.pop_front().unwrap();
            h.service.job_active = Some((id.clone(), 1));
            h.service.event(Event::JobDone(
                id,
                1,
                Ok(("audio.mp3".into(), String::new())),
            ));
        }
        assert_eq!(h.service.jobs.len(), DOWNLOAD_HISTORY_LIMIT);
        assert_eq!(h.service.job_controls.len(), DOWNLOAD_HISTORY_LIMIT);
        assert!(h.service.job_queue.is_empty());
        assert!(h.service.job_active.is_none());
        assert!(h.service.jobs.iter().all(|job| job.phase == "done"));
        assert!(!h.service.job_controls.contains_key("d1"));
    }
    #[test]
    fn stale_download_entries_do_not_block_the_next_job() {
        let mut h = Harness::new("list");
        h.service.job_active = Some(("busy".into(), 1));
        let directory = h.dir.path().join("not-a-directory");
        fs::write(&directory, b"file").unwrap();
        h.service
            .job_queue
            .extend((0..10_000).map(|_| "expired".into()));
        h.service
            .enqueue_job(&json!({"track": Track {
                id: "kuwo:1".into(), ..Default::default()
            }, "directory": directory}))
            .unwrap();
        h.service.job_active = None;
        h.service.kick_download();
        assert!(h.service.job_queue.is_empty());
        assert_eq!(h.service.job_active, Some(("d1".into(), 1)));
        let deadline = Instant::now() + Duration::from_secs(4);
        while h.service.job_active.is_some() {
            assert!(Instant::now() < deadline);
            if let Ok(event) = h.rx.recv_timeout(Duration::from_millis(20)) {
                h.service.event(event);
            }
        }
        assert_eq!(h.service.jobs[0].phase, "error");
    }
    #[test]
    fn download_rejects_an_unwritable_directory_before_resolving_sources() {
        let mut h = Harness::new("list");
        let directory = h.dir.path().join("not-a-directory");
        fs::write(&directory, b"file").unwrap();
        h.service.dispatch(json!({"op":"download", "track":Track { id: "kuwo:123".into(), ..Default::default() }, "directory":directory, "direct":true})).unwrap();
        assert!(h.service.jobs[0].direct);
        let deadline = Instant::now() + Duration::from_secs(1);
        while h.service.jobs[0].phase != "error" {
            assert!(Instant::now() < deadline);
            if let Ok(event) = h.rx.recv_timeout(Duration::from_millis(20)) {
                h.service.event(event);
            }
        }
        assert!(h.service.jobs[0].error.contains("下载目录不可写"));
        assert!(h.service.job_active.is_none());
    }
    #[test]
    fn download_paths_and_direct_mode_are_attached_to_the_queued_job() {
        let mut h = Harness::new("list");
        h.service.job_active = Some(("busy".into(), 1));
        let track = Track {
            id: "kuwo:123".into(),
            ..Default::default()
        };
        assert!(h
            .service
            .dispatch(json!({"op":"download", "track":track, "directory":""}))
            .is_err());
        h.service
            .dispatch(
                json!({"op":"download", "track":track, "directory":"downloads", "direct":true}),
            )
            .unwrap();
        assert!(PathBuf::from(&h.service.jobs[0].directory).is_absolute());
        assert!(h.service.jobs[0].direct);
        h.service
            .dispatch(
                json!({"op":"download", "track":track, "directory":"downloads", "direct":true}),
            )
            .unwrap();
        assert_eq!(h.service.jobs.len(), 1);
    }
    #[test]
    fn successive_paused_seeks_keep_pause_and_latest_position() {
        let mut h = Harness::new("list");
        let track = h.track("song", true);
        h.service
            .dispatch(json!({"op":"play", "tracks":[track]}))
            .unwrap();
        h.pump("playing");
        h.service.dispatch(json!({"op":"pause"})).unwrap();
        for position in [12., 3., 0.] {
            let (reply, rx) = mpsc::channel();
            h.service
                .seek_request(json!({"op":"seek_to", "position": position}), reply);
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Ok(result) = rx.try_recv() {
                    let status = result.unwrap();
                    assert_eq!(status["phase"], "paused");
                    assert!((status["position"].as_f64().unwrap() - position).abs() < 0.01);
                    break;
                }
                assert!(Instant::now() < deadline);
                if let Ok(event) = h.rx.recv_timeout(Duration::from_millis(20)) {
                    h.service.event(event);
                }
            }
        }
    }
    #[test]
    fn stopped_toggle_plays_queue_and_shuffle_prev_falls_back() {
        let mut h = Harness::new("shuffle");
        let t1 = h.track("one", true);
        let t2 = h.track("two", true);
        h.service.queue = vec![t1, t2];
        h.service.phase = "stopped".into();

        // 1. Stopped + pause succeeds cleanly as no-op
        h.service.dispatch(json!({"op":"pause"})).unwrap();
        assert_eq!(h.service.phase, "stopped");

        // 2. Stopped + toggle starts playing current queue track
        h.service.dispatch(json!({"op":"toggle"})).unwrap();
        assert_eq!(h.service.phase, "loading");
        h.pump("playing");
        assert_eq!(h.service.phase, "playing");

        // 3. Shuffle prev without previous history falls back to previous track instead of error
        h.service.shuffle.reset(2, 0);
        h.service.dispatch(json!({"op":"prev"})).unwrap();
        assert_eq!(h.service.phase, "loading");
        assert_eq!(h.service.index, 1);
        h.service.dispatch(json!({"op":"next"})).unwrap();
        assert_eq!(h.service.index, 0);
    }
    #[test]
    fn invalid_play_requests_preserve_the_current_queue_audio_and_boss_state() {
        let mut h = Harness::new("list");
        let first = h.track("first", true);
        let second = h.track("second", true);
        h.service
            .dispatch(json!({"op":"play", "tracks":[first, second], "index":1}))
            .unwrap();
        h.pump("playing");
        h.service
            .dispatch(json!({"op":"boss", "active":true}))
            .unwrap();
        let before = serde_json::to_value(h.service.status()).unwrap();
        let generation = h.service.generation;
        let replacement = h.track("replacement", true);
        for request in [
            json!({"op":"play", "tracks":[replacement], "index":5}),
            json!({"op":"play", "tracks":[replacement], "quality":"invalid"}),
            json!({"op":"play", "tracks":[replacement], "quality":100}),
            json!({"op":"play", "tracks":[]}),
            json!({"op":"play", "tracks":{}}),
            json!({"op":"play", "index":-1}),
            json!({"op":"play", "index":"1"}),
        ] {
            assert!(h.service.dispatch(request).is_err());
            assert_eq!(serde_json::to_value(h.service.status()).unwrap(), before);
            assert_eq!(h.service.generation, generation);
        }
    }
    #[test]
    fn play_accepts_missing_or_null_quality_from_cli_and_uses_the_configured_default() {
        for quality in [None, Some(Value::Null)] {
            let mut h = Harness::new("list");
            h.service.settings.quality = "320k".into();
            let track = h.track("song", true);
            let mut request = json!({"op":"play", "tracks":[track]});
            if let Some(quality) = quality {
                request["quality"] = quality;
            }
            h.service.dispatch(request).unwrap();
            assert_eq!(h.service.requested_quality, "320k");
            h.pump("playing");
        }
    }
    #[test]
    fn removing_the_current_track_preserves_paused_or_stopped_playback() {
        for op in ["pause", "stop"] {
            let mut h = Harness::new("list");
            let first = h.track("first", true);
            let second = h.track("second", true);
            h.service
                .dispatch(json!({"op":"play", "tracks":[first, second.clone()]}))
                .unwrap();
            h.pump("playing");
            h.service.dispatch(json!({"op":op})).unwrap();
            h.service
                .dispatch(json!({"op":"queue_remove", "index":0}))
                .unwrap();
            if op == "pause" {
                assert!(h.service.pause_after_load);
                h.pump("paused");
            } else {
                assert_eq!(h.service.phase, "stopped");
                assert!(h.service.lyric_text.is_empty());
                assert_eq!(h.service.lyric_status, "idle");
            }
            assert_eq!(h.service.current(), Some(&second));
            h.service.dispatch(json!({"op":"queue_clear"})).unwrap();
            assert_eq!(h.service.phase, "stopped");
            assert!(h.service.lyric_text.is_empty());
            assert_eq!(h.service.lyric_status, "idle");
        }
    }
    #[test]
    fn background_stream_retry_and_failure_skip_do_not_resume_paused_audio() {
        let mut h = Harness::new("repeat");
        let first = h.track("first", true);
        let second = h.track("second", true);
        h.service
            .dispatch(json!({"op":"play", "tracks":[first, second]}))
            .unwrap();
        h.pump("playing");
        h.service.dispatch(json!({"op":"pause"})).unwrap();
        for expected_index in [0, 1] {
            h.service.event(Event::StreamFinished(
                h.service.generation,
                Err(Error::coded("network", "connection closed")),
            ));
            assert!(h.service.pause_after_load);
            h.pump("paused");
            assert_eq!(h.service.index, expected_index);
        }
    }
    #[test]
    fn boss_roundtrip_during_a_paused_reload_preserves_pause_intent() {
        for replace_current in [false, true] {
            let mut h = Harness::new("repeat");
            let first = h.track("first", true);
            let second = h.track("second", true);
            h.service
                .dispatch(json!({"op":"play", "tracks":[first, second]}))
                .unwrap();
            h.pump("playing");
            h.service.dispatch(json!({"op":"pause"})).unwrap();
            if replace_current {
                h.service
                    .dispatch(json!({"op":"queue_remove", "index":0}))
                    .unwrap();
            } else {
                h.service.event(Event::StreamFinished(
                    h.service.generation,
                    Err(Error::coded("network", "connection closed")),
                ));
            }
            assert_eq!(h.service.phase, "loading");
            assert!(h.service.pause_after_load);
            h.service
                .dispatch(json!({"op":"boss", "active":true}))
                .unwrap();
            assert_eq!(h.service.phase, "stopped");
            h.service
                .dispatch(json!({"op":"boss", "active":false}))
                .unwrap();
            assert!(h.service.pause_after_load);
            h.pump("paused");
            assert_eq!(h.service.phase, "paused");
        }
    }
    #[test]
    fn explicit_resume_during_a_paused_reload_starts_when_ready() {
        let mut h = Harness::new("list");
        let first = h.track("first", true);
        let second = h.track("second", true);
        h.service
            .dispatch(json!({"op":"play", "tracks":[first, second]}))
            .unwrap();
        h.pump("playing");
        h.service.dispatch(json!({"op":"pause"})).unwrap();
        h.service
            .dispatch(json!({"op":"queue_remove", "index":0}))
            .unwrap();
        assert_eq!(h.service.phase, "loading");
        assert!(h.service.pause_after_load);
        let generation = h.service.generation;
        for _ in 0..2 {
            h.service.dispatch(json!({"op":"resume"})).unwrap();
            assert!(!h.service.pause_after_load);
            assert_eq!(h.service.generation, generation);
        }
        h.pump("playing");
    }
    #[test]
    fn stopping_a_pending_lyric_request_allows_retry_and_ignores_the_old_result() {
        let mut h = Harness::new("list");
        h.service.queue = vec![h.track("song", true)];
        h.service.lyric_status = "loading".into();
        let generation = h.service.generation;
        let serial = h.service.lyric_serial;
        h.service.dispatch(json!({"op":"stop"})).unwrap();
        assert_eq!(h.service.lyric_status, "idle");
        h.service
            .event(Event::Lyrics(generation, serial, Ok("[00:00]stale".into())));
        assert!(h.service.lyric_text.is_empty());
        h.service.dispatch(json!({"op":"lyrics_retry"})).unwrap();
        assert_eq!(h.service.lyric_status, "loading");
        assert!(h.service.lyric_serial > serial);
    }
    #[test]
    fn late_device_reattachment_respects_pause_and_boss_mode() {
        for request in [json!({"op":"pause"}), json!({"op":"boss", "active":true})] {
            let mut h = Harness::new("list");
            let track = h.track("song", true);
            h.service
                .dispatch(json!({"op":"play", "tracks":[track]}))
                .unwrap();
            h.pump("playing");
            h.service.dispatch(json!({"op":"pause"})).unwrap();
            h.service.reattach_player(false).unwrap();
            let completed = h.reattach_event();
            h.service.dispatch(request).unwrap();
            h.service.event(completed);
            assert_eq!(h.service.phase, "paused");
            assert!(!h.service.reattach_pending);
        }
    }
    #[test]
    fn stale_reattach_results_cannot_override_a_newer_seek_or_reattach() {
        let mut h = Harness::new("list");
        let track = h.track("song", true);
        h.service
            .dispatch(json!({"op":"play", "tracks":[track]}))
            .unwrap();
        h.pump("playing");
        h.service.dispatch(json!({"op":"pause"})).unwrap();
        h.service.reattach_player(false).unwrap();
        let stale = h.reattach_event();
        h.service.reattach_player(true).unwrap();
        h.service.event(stale);
        assert_eq!(h.service.phase, "paused");
        assert!(h.service.reattach_pending);
        let ready = h.reattach_event();
        h.service.event(ready);
        assert!(!h.service.reattach_pending);

        h.service.reattach_player(false).unwrap();
        let stale = h.reattach_event();
        let (reply, rx) = mpsc::channel();
        h.service
            .seek_request(json!({"op":"seek_to", "position":12}), reply);
        h.service.event(stale);
        assert_eq!(h.service.phase, "paused");
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if let Ok(result) = rx.try_recv() {
                let status = result.unwrap();
                assert_eq!(status["phase"], "paused");
                assert_eq!(status["position"], 12.);
                break;
            }
            assert!(Instant::now() < deadline);
            if let Ok(event) = h.rx.recv_timeout(Duration::from_millis(20)) {
                h.service.event(event);
            }
        }
    }
    #[test]
    fn boss_mode_pauses_and_restores_playback() {
        let mut h = Harness::new("list");
        let t1 = h.track("one", true);
        h.service.queue = vec![t1];
        h.service.dispatch(json!({"op":"toggle"})).unwrap();
        h.pump("playing");
        assert_eq!(h.service.phase, "playing");
        assert!(!h.service.boss);

        // Enter boss mode: pauses and marks boss = true
        let st = h.service.dispatch(json!({"op":"boss"})).unwrap();
        assert_eq!(st["boss"], true);
        assert_eq!(h.service.phase, "paused");

        // Exit boss mode: unpauses and resumes playing
        let st = h.service.dispatch(json!({"op":"boss"})).unwrap();
        assert_eq!(st["boss"], false);
        assert_eq!(h.service.phase, "playing");
    }

    #[test]
    fn boss_on_is_idempotent_and_media_keys_cannot_restart_audio() {
        let mut h = Harness::new("list");
        h.service.queue = vec![h.track("one", true)];
        h.service.dispatch(json!({"op":"play"})).unwrap();
        h.pump("playing");
        for _ in 0..2 {
            h.service
                .dispatch(json!({"op":"boss","active":true}))
                .unwrap();
        }
        for op in ["resume", "toggle"] {
            let state = h.service.dispatch(json!({"op":op})).unwrap();
            assert_eq!(state["boss"], true);
            assert_eq!(state["phase"], "paused");
        }
        h.service
            .dispatch(json!({"op":"boss","active":false}))
            .unwrap();
        assert_eq!(h.service.phase, "playing");
        h.service.dispatch(json!({"op":"pause"})).unwrap();
        h.service.dispatch(json!({"op":"boss"})).unwrap();
        h.service.dispatch(json!({"op":"boss"})).unwrap();
        assert_eq!(h.service.phase, "paused");
    }
    #[test]
    fn boss_restores_loading_but_respects_explicit_stop_and_pause() {
        let mut h = Harness::new("list");
        h.service.queue = vec![h.track("one", true)];
        h.service.dispatch(json!({"op":"play"})).unwrap();
        assert_eq!(h.service.phase, "loading");
        h.service
            .dispatch(json!({"op":"boss","active":true}))
            .unwrap();
        assert_eq!(h.service.phase, "stopped");
        h.service
            .dispatch(json!({"op":"boss","active":true}))
            .unwrap();
        h.service
            .dispatch(json!({"op":"boss","active":false}))
            .unwrap();
        h.pump("playing");
        for op in ["stop", "pause", "queue_clear"] {
            h.service.dispatch(json!({"op":"play"})).unwrap();
            h.pump("playing");
            h.service
                .dispatch(json!({"op":"boss","active":true}))
                .unwrap();
            h.service.dispatch(json!({"op":op})).unwrap();
            h.service
                .dispatch(json!({"op":"boss","active":false}))
                .unwrap();
            assert_ne!(h.service.phase, "playing", "{op}");
        }
        assert!(h
            .service
            .dispatch(json!({"op":"boss","active":"false"}))
            .is_err());
        assert!(!h.service.boss);
    }
    #[test]
    fn retry_cannot_start_sound_while_boss_is_active() {
        let mut h = Harness::new("list");
        h.service.queue = vec![h.track("one", true)];
        h.service.dispatch(json!({"op":"play"})).unwrap();
        h.pump("playing");
        h.service
            .dispatch(json!({"op":"boss","active":true}))
            .unwrap();
        h.service.dispatch(json!({"op":"retry"})).unwrap();
        h.pump("paused");
        assert!(h.service.boss);
        h.service
            .dispatch(json!({"op":"boss","active":false}))
            .unwrap();
        assert_eq!(h.service.phase, "playing");
    }

    #[test]
    fn audio_device_change_and_reattach_flow() {
        if std::env::var("CI").is_ok()
            || std::env::var("GITHUB_ACTIONS").is_ok()
            || crate::player::current_default_device_name().is_none()
        {
            return;
        }
        let mut h = Harness::new("list");
        h.service.queue = vec![h.track("one", true)];
        h.service.dispatch(json!({"op":"play"})).unwrap();
        h.pump("playing");
        assert_eq!(h.service.phase, "playing");

        h.service.player.null = false;
        h.service
            .player
            .set_stream_device_name_for_test(Some("Mock Bluetooth".into()));
        assert_eq!(
            h.service.player.stream_device_name(),
            Some("Mock Bluetooth")
        );

        h.service.last_device_check = Instant::now() - Duration::from_secs(1);
        h.service.tick();

        assert_eq!(h.service.phase, "paused");
        assert!(h.service.player.stream_is_none());
        assert_eq!(h.service.player.stream_device_name(), None);

        h.service.player.null = true;
        h.service.reattach_player(false).unwrap();
        h.pump("playing");
        assert_eq!(h.service.phase, "playing");
    }
}
