use crate::{
    ipc, lyrics,
    model::{clock, mode_label, Result, Track, MODES, QUALITIES},
    provider::Provider,
    service::Status,
    store::{Paths, Store},
};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::{
    prelude::*,
    widgets::{
        Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Row, Table, TableState,
        Wrap,
    },
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;

const BG: Color = Color::Rgb(17, 25, 20);
const FG: Color = Color::Rgb(218, 226, 209);
const DIM: Color = Color::Rgb(135, 156, 128);
const ACCENT: Color = Color::Rgb(192, 218, 153);
const PANEL: Color = Color::Rgb(26, 39, 29);
const SELECTED: Color = Color::Rgb(48, 68, 43);
const BAD: Color = Color::Rgb(225, 161, 135);
#[derive(Clone, Debug, PartialEq)]
enum View {
    Likes,
    Queue,
    History,
    Folder(String),
    Search,
}
#[derive(Clone)]
enum Action {
    View(View),
    Search,
    SearchGo,
    Page(i32),
    Lyrics,
    Jobs,
    Help,
    Play(usize),
    Select(usize),
    Control(Value),
    Toggle,
    Mode,
    Quality(bool),
    Collect(Track),
    Collected(String, Track),
    Song(Track),
    Download(Track),
    DownloadGo,
    DownloadField(u8),
    NewFolder(Option<Track>),
    Rename(String),
    DeleteFolder(String),
    FolderMenu(String),
    Remove(Track),
    Volume(i16),
    Close,
    Seek,
    ConfirmDelete(String),
    SubmitInput,
    JobSelect(usize),
}
#[derive(Clone)]
struct Menu {
    title: String,
    caption: String,
    items: Vec<(String, Action)>,
    selected: usize,
}
#[derive(Clone)]
enum InputKind {
    Folder(Option<Track>),
    Rename(String),
}
#[derive(Clone)]
enum Popup {
    Menu(Menu),
    Input {
        title: String,
        value: String,
        kind: InputKind,
    },
    Download {
        track: Track,
        directory: String,
        quality: usize,
        lyrics: bool,
        focus: u8,
    },
    Jobs(usize),
    Help,
}
#[derive(Clone)]
struct Hit {
    rect: Rect,
    action: Action,
}
enum Message {
    State(Box<Status>),
    Offline,
    Error(String),
    Search(u64, std::result::Result<(usize, Vec<Track>), String>),
}
struct Request {
    value: Value,
    start: bool,
}
struct App {
    paths: Paths,
    store: Store,
    state: Status,
    view: View,
    before_search: View,
    rows: Vec<Track>,
    selected: usize,
    table: TableState,
    nav: ListState,
    folders: Vec<(String, usize)>,
    liked: HashSet<String>,
    nav_focus: bool,
    query: String,
    editing: bool,
    search_rows: Vec<Track>,
    total: usize,
    page: usize,
    serial: u64,
    searching: bool,
    search_error: String,
    popup: Option<Popup>,
    full_lyrics: bool,
    hits: Vec<Hit>,
    timeline: Rect,
    seek: Option<(String, f64, f64)>,
    last_click: Option<(usize, Instant)>,
    toast: Option<(String, Instant)>,
    tx: Sender<Request>,
    events: Sender<Message>,
}
impl App {
    fn new(paths: Paths, tx: Sender<Request>, events: Sender<Message>) -> Result<Self> {
        let store = Store::open(&paths)?;
        let settings = paths.settings()?;
        let mut app = Self {
            paths,
            store,
            state: Status {
                volume: settings.volume,
                mode: settings.mode,
                requested_quality: settings.quality.clone(),
                default_quality: settings.quality,
                ..Status::default()
            },
            view: View::Likes,
            before_search: View::Likes,
            rows: vec![],
            selected: 0,
            table: TableState::default(),
            nav: ListState::default(),
            folders: vec![],
            liked: HashSet::new(),
            nav_focus: false,
            query: String::new(),
            editing: false,
            search_rows: vec![],
            total: 0,
            page: 1,
            serial: 0,
            searching: false,
            search_error: String::new(),
            popup: None,
            full_lyrics: false,
            hits: vec![],
            timeline: Rect::default(),
            seek: None,
            last_click: None,
            toast: None,
            tx,
            events,
        };
        if let Some(name) = app.store.load::<String>("rust.ui.folder")? {
            if app.store.folder(&name).is_ok() {
                app.view = View::Folder(name);
            }
        }
        if let Some(queue) = app.store.load::<Value>("queue")? {
            app.state.queue = serde_json::from_value(queue["tracks"].clone()).unwrap_or_default();
        }
        app.reload()?;
        Ok(app)
    }
    fn reload(&mut self) -> Result<()> {
        let selected_id = self.rows.get(self.selected).map(|t| t.id.clone());
        self.folders = self.store.folders()?;
        let likes = self.store.favorites()?;
        self.liked = likes.iter().map(|t| t.id.clone()).collect();
        self.rows = match &self.view {
            View::Likes => likes,
            View::Queue => self.state.queue.clone(),
            View::History => self.store.history()?,
            View::Folder(name) => self.store.folder(name).unwrap_or_default(),
            View::Search => self.search_rows.clone(),
        };
        if let Some(index) = selected_id.and_then(|id| self.rows.iter().position(|t| t.id == id)) {
            self.selected = index;
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.table
            .select((!self.rows.is_empty()).then_some(self.selected));
        Ok(())
    }
    fn notice(&mut self, value: impl Into<String>) {
        self.toast = Some((value.into(), Instant::now()));
    }
    fn send(&self, value: Value) {
        let _ = self.tx.send(Request { value, start: true });
    }
    fn current(&self) -> Option<Track> {
        if self.full_lyrics {
            self.state.track.clone()
        } else {
            self.rows.get(self.selected).cloned()
        }
    }
    fn navigate(&mut self, view: View) -> Result<()> {
        if let View::Folder(name) = &view {
            self.store.save("rust.ui.folder", name)?;
        }
        self.view = view;
        self.selected = 0;
        self.rows.clear();
        self.full_lyrics = false;
        self.editing = false;
        self.nav_focus = false;
        self.table = TableState::default();
        self.reload()
    }
    fn search(&mut self) {
        if self.query.trim().is_empty() {
            return;
        }
        self.serial += 1;
        self.searching = true;
        self.search_error.clear();
        self.editing = false;
        let (serial, query, page, tx, direct) = (
            self.serial,
            self.query.clone(),
            self.page,
            self.events.clone(),
            self.paths.settings().map(|s| s.direct).unwrap_or(false),
        );
        thread::spawn(move || {
            let result = Provider::new(direct)
                .search(&query, page - 1, 20)
                .map_err(|e| e.to_string());
            let _ = tx.send(Message::Search(serial, result));
        });
    }
    fn menu(&mut self, title: &str, caption: String, items: Vec<(String, Action)>) {
        self.popup = Some(Popup::Menu(Menu {
            title: title.into(),
            caption,
            items,
            selected: 0,
        }));
    }
    fn collect(&mut self, track: Track) -> Result<()> {
        let mut items = vec![];
        for (name, count) in &self.folders {
            let present = self.store.folder(name)?.iter().any(|t| t.id == track.id);
            items.push((
                format!("{name}  ·  {count}{}", if present { "  ✓" } else { "" }),
                Action::Collected(name.clone(), track.clone()),
            ));
        }
        items.push((
            "＋ 新建收藏夹…".into(),
            Action::NewFolder(Some(track.clone())),
        ));
        self.menu("收藏到", track.title, items);
        Ok(())
    }
    fn act(&mut self, action: Action) -> Result<()> {
        match action {
            Action::View(view) => {
                self.popup = None;
                self.navigate(view)?;
            }
            Action::Search => {
                if self.view != View::Search {
                    self.before_search = self.view.clone();
                }
                self.view = View::Search;
                self.full_lyrics = false;
                self.editing = true;
                self.nav_focus = false;
                self.reload()?;
            }
            Action::SearchGo => {
                self.page = 1;
                self.search();
            }
            Action::Page(delta) => {
                let next = (self.page as i32 + delta).max(1) as usize;
                if next <= self.total.div_ceil(20).max(1) && next != self.page {
                    self.page = next;
                    self.search();
                }
            }
            Action::Lyrics => {
                self.full_lyrics = !self.full_lyrics;
                self.nav_focus = false;
            }
            Action::Jobs => self.popup = Some(Popup::Jobs(0)),
            Action::Help => self.popup = Some(Popup::Help),
            Action::Play(index) => {
                if index < self.rows.len() {
                    self.send(json!({"op":"play","tracks":self.rows,"index":index}));
                }
            }
            Action::Select(index) => {
                self.selected = index.min(self.rows.len().saturating_sub(1));
                self.table.select(Some(self.selected));
                self.nav_focus = false;
                if self
                    .last_click
                    .is_some_and(|(i, at)| i == index && at.elapsed() < Duration::from_millis(350))
                {
                    self.act(Action::Play(index))?;
                    self.last_click = None;
                } else {
                    self.last_click = Some((index, Instant::now()));
                }
            }
            Action::Control(value) => {
                self.popup = None;
                self.send(value);
            }
            Action::Toggle => {
                if ["stopped", "error", ""].contains(&self.state.phase.as_str()) {
                    if self.state.track.is_some() {
                        self.send(json!({"op":"retry"}));
                    } else {
                        self.act(Action::Play(self.selected))?;
                    }
                } else {
                    self.send(json!({"op":"toggle"}));
                }
            }
            Action::Mode => self.menu(
                "播放方式",
                String::new(),
                MODES
                    .iter()
                    .map(|m| {
                        (
                            format!(
                                "{}{}",
                                mode_label(m),
                                if self.state.mode == *m { "  ✓" } else { "" }
                            ),
                            Action::Control(json!({"op":"mode","value":m})),
                        )
                    })
                    .collect(),
            ),
            Action::Quality(retry) => self.menu(
                if retry {
                    "切换音质并重试"
                } else {
                    "默认音质"
                },
                String::new(),
                QUALITIES
                    .iter()
                    .map(|q| {
                        (
                            format!(
                                "{}{}",
                                q,
                                if (if retry {
                                    &self.state.requested_quality
                                } else {
                                    &self.state.default_quality
                                }) == *q
                                {
                                    "  ✓"
                                } else {
                                    ""
                                }
                            ),
                            Action::Control(if retry {
                                json!({"op":"retry","quality":q})
                            } else {
                                json!({"op":"config","key":"quality","value":q})
                            }),
                        )
                    })
                    .collect(),
            ),
            Action::Collect(track) => self.collect(track)?,
            Action::Collected(name, track) => {
                self.store.collect(&name, &track)?;
                self.store.save("rust.ui.last_collection", &name)?;
                self.popup = None;
                self.reload()?;
                self.notice(format!("已收藏到 {name}"));
            }
            Action::NewFolder(track) => {
                self.popup = Some(Popup::Input {
                    title: "新建收藏夹".into(),
                    value: String::new(),
                    kind: InputKind::Folder(track),
                })
            }
            Action::Rename(name) => {
                self.popup = Some(Popup::Input {
                    title: "重命名收藏夹".into(),
                    value: name.clone(),
                    kind: InputKind::Rename(name),
                })
            }
            Action::FolderMenu(name) => self.menu(
                "收藏夹",
                name.clone(),
                vec![
                    ("重命名…".into(), Action::Rename(name.clone())),
                    ("删除收藏夹…".into(), Action::ConfirmDelete(name)),
                ],
            ),
            Action::ConfirmDelete(name) => self.menu(
                "删除收藏夹？",
                name.clone(),
                vec![
                    ("取消".into(), Action::Close),
                    ("删除收藏夹".into(), Action::DeleteFolder(name)),
                ],
            ),
            Action::DeleteFolder(name) => {
                self.store.delete(&name)?;
                self.popup = None;
                self.navigate(View::Likes)?;
                self.notice("收藏夹已删除");
            }
            Action::Remove(track) => {
                match &self.view {
                    View::Folder(name) => self.store.remove(name, &track.id)?,
                    View::Likes => {
                        if self.store.is_favorite(&track.id) {
                            self.store.favorite(&track)?;
                        }
                    }
                    View::Queue => self.send(
                        json!({"op":"queue_remove","index":self.selected,"track_id":track.id}),
                    ),
                    _ => {}
                }
                self.popup = None;
                self.reload()?;
            }
            Action::Song(track) => {
                let mut items = vec![
                    ("收藏到…".into(), Action::Collect(track.clone())),
                    (
                        if self.liked.contains(&track.id) {
                            "取消喜欢"
                        } else {
                            "喜欢"
                        }
                        .into(),
                        Action::Control(json!({"op":"like","track":track})),
                    ),
                    (
                        "下一首播放".into(),
                        Action::Control(json!({"op":"queue_add","tracks":[track],"next":true})),
                    ),
                    (
                        "加入播放队列".into(),
                        Action::Control(json!({"op":"queue_add","tracks":[track]})),
                    ),
                    ("下载…".into(), Action::Download(track.clone())),
                ];
                if !self.full_lyrics
                    && matches!(self.view, View::Folder(_) | View::Likes | View::Queue)
                {
                    items.push(("从列表移除".into(), Action::Remove(track.clone())));
                }
                self.menu(
                    "歌曲操作",
                    format!("{} · {}", track.title, track.artist),
                    items,
                );
            }
            Action::Download(track) => {
                let directory = std::env::var("WOCLI_DOWNLOAD_DIR")
                    .ok()
                    .or_else(|| {
                        self.store
                            .load::<String>("rust.ui.download_directory")
                            .ok()
                            .flatten()
                    })
                    .unwrap_or_else(|| "~/Music/wocli".into());
                let quality = QUALITIES
                    .iter()
                    .position(|q| *q == self.state.default_quality)
                    .unwrap_or(0);
                self.popup = Some(Popup::Download {
                    track,
                    directory,
                    quality,
                    lyrics: true,
                    focus: 0,
                });
            }
            Action::DownloadField(field) => {
                if let Some(Popup::Download {
                    quality,
                    lyrics,
                    focus,
                    ..
                }) = &mut self.popup
                {
                    *focus = field;
                    if field == 1 {
                        *quality = (*quality + 1) % QUALITIES.len();
                    }
                    if field == 2 {
                        *lyrics = !*lyrics;
                    }
                }
            }
            Action::DownloadGo => {
                if let Some(Popup::Download {
                    track,
                    directory,
                    quality,
                    lyrics,
                    ..
                }) = self.popup.clone()
                {
                    if directory.trim().is_empty() {
                        self.notice("下载目录不能为空");
                        return Ok(());
                    }
                    let path = crate::cli::expand_path(&directory);
                    self.store.save("rust.ui.download_directory", &directory)?;
                    self.send(json!({"op":"download","track":track,"directory":path,"quality":QUALITIES[quality],"lyrics":lyrics}));
                    self.popup = Some(Popup::Jobs(0));
                }
            }
            Action::Volume(delta) => self.send(
                json!({"op":"volume","value":(i16::from(self.state.volume)+delta).clamp(0,100)}),
            ),
            Action::Close => self.popup = None,
            Action::SubmitInput => self.submit_input()?,
            Action::JobSelect(index) => self.popup = Some(Popup::Jobs(index)),
            Action::Seek => {}
        }
        Ok(())
    }
    fn submit_input(&mut self) -> Result<()> {
        if let Some(Popup::Input { value, kind, .. }) = self.popup.clone() {
            let name = value.trim();
            match kind {
                InputKind::Folder(track) => {
                    if self.folders.iter().any(|(v, _)| v == name) {
                        return Err(crate::model::Error::new("已存在同名收藏夹"));
                    }
                    self.store.create_folder(name)?;
                    if let Some(track) = track {
                        self.store.collect(name, &track)?;
                        self.notice(format!("已收藏到 {name}"));
                    } else {
                        self.navigate(View::Folder(name.into()))?;
                    }
                }
                InputKind::Rename(old) => {
                    self.store.rename(&old, name)?;
                    self.navigate(View::Folder(name.into()))?;
                }
            }
            self.popup = None;
            self.reload()?;
        }
        Ok(())
    }
    fn key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('q' | 'c'))
        {
            return Ok(true);
        }
        if let Some(popup) = &mut self.popup {
            if key.code == KeyCode::Esc {
                self.popup = None;
                return Ok(false);
            }
            match popup {
                Popup::Menu(menu) => match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        menu.selected = menu.selected.saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        menu.selected = (menu.selected + 1).min(menu.items.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        let action = menu.items.get(menu.selected).map(|v| v.1.clone());
                        if let Some(action) = action {
                            self.act(action)?;
                        }
                    }
                    _ => {}
                },
                Popup::Input { value, .. } => match key.code {
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        value.push(c)
                    }
                    KeyCode::Backspace => {
                        value.pop();
                    }
                    KeyCode::Enter => self.submit_input()?,
                    _ => {}
                },
                Popup::Download {
                    directory,
                    quality,
                    lyrics,
                    focus,
                    ..
                } => match key.code {
                    KeyCode::Tab | KeyCode::Down => *focus = (*focus + 1) % 4,
                    KeyCode::BackTab | KeyCode::Up => *focus = (*focus + 3) % 4,
                    KeyCode::Char(c)
                        if *focus == 0 && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        directory.push(c)
                    }
                    KeyCode::Backspace if *focus == 0 => {
                        directory.pop();
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if *focus == 1 => {
                        *quality = (*quality + 1) % QUALITIES.len()
                    }
                    KeyCode::Char(' ') if *focus == 2 => *lyrics = !*lyrics,
                    KeyCode::Enter => {
                        if *focus == 1 {
                            *quality = (*quality + 1) % QUALITIES.len();
                        } else if *focus == 2 {
                            *lyrics = !*lyrics;
                        } else {
                            self.act(Action::DownloadGo)?;
                        }
                    }
                    _ => {}
                },
                Popup::Jobs(index) => match key.code {
                    KeyCode::Up | KeyCode::Char('k') => *index = index.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        *index = (*index + 1).min(self.state.downloads.len().saturating_sub(1))
                    }
                    KeyCode::Home | KeyCode::Char('g') => *index = 0,
                    KeyCode::End | KeyCode::Char('G') => {
                        *index = self.state.downloads.len().saturating_sub(1)
                    }
                    KeyCode::Char('r') | KeyCode::Char('c') => {
                        if let Some(job) = self.state.downloads.get(*index) {
                            self.send(json!({"op":if key.code==KeyCode::Char('r'){"download_retry"}else{"download_cancel"},"id":job.id}));
                        }
                    }
                    _ => {}
                },
                Popup::Help => {
                    if key.code == KeyCode::Char('?') {
                        self.popup = None;
                    }
                }
            }
            return Ok(false);
        }
        if self.editing {
            match key.code {
                KeyCode::Esc => {
                    self.editing = false;
                    if self.query.is_empty() {
                        self.navigate(self.before_search.clone())?;
                    }
                }
                KeyCode::Enter => self.act(Action::SearchGo)?,
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.push(c)
                }
                _ => {}
            }
            return Ok(false);
        }
        let action = match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('/') => Some(Action::Search),
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::Search)
            }
            KeyCode::Char('?') => Some(Action::Help),
            KeyCode::Char('v') => Some(Action::Lyrics),
            KeyCode::Char('b') => {
                let items = self
                    .navigation()
                    .into_iter()
                    .map(|(label, view)| (label, Action::View(view)))
                    .collect();
                self.menu("音乐库", String::new(), items);
                None
            }
            KeyCode::Char('m') => Some(Action::Mode),
            KeyCode::Char('n') => Some(Action::NewFolder(None)),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::Control(json!({"op":"retry"})))
            }
            KeyCode::Char('r') => {
                if let View::Folder(name) = &self.view {
                    Some(Action::Rename(name.clone()))
                } else {
                    None
                }
            }
            KeyCode::Char('l') => Some(Action::Control(json!({"op":"lyrics_retry"}))),
            KeyCode::Char('c') => self.current().map(Action::Collect),
            KeyCode::Char('x') => self.current().map(Action::Song),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::Page(1))
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::Page(-1))
            }
            KeyCode::Char('d') => self.current().map(Action::Download),
            KeyCode::Char('f') => self
                .current()
                .map(|t| Action::Control(json!({"op":"like","track":t}))),
            KeyCode::Char('a') => self
                .current()
                .map(|t| Action::Control(json!({"op":"queue_add","tracks":[t]}))),
            KeyCode::Char(' ') => Some(Action::Toggle),
            KeyCode::Char('+') | KeyCode::Char('=') => Some(Action::Volume(5)),
            KeyCode::Char('-') => Some(Action::Volume(-5)),
            KeyCode::Char('[' | '<') => Some(Action::Control(json!({"op":"prev"}))),
            KeyCode::Char(']' | '>') => Some(Action::Control(json!({"op":"next"}))),
            KeyCode::Left | KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::Control(
                    json!({"op":if key.code==KeyCode::Left {"prev"}else{"next"}}),
                ))
            }
            KeyCode::Left | KeyCode::Right => Some(Action::Control(
                json!({"op":"seek","seconds":if key.code==KeyCode::Left {-5}else{5}}),
            )),
            KeyCode::PageUp => Some(Action::Page(-1)),
            KeyCode::PageDown => Some(Action::Page(1)),
            KeyCode::Home | KeyCode::Char('g') => {
                if self.nav_focus {
                    self.nav.select(Some(0));
                } else {
                    self.selected = 0;
                    self.table.select((!self.rows.is_empty()).then_some(0));
                }
                None
            }
            KeyCode::End | KeyCode::Char('G') => {
                if self.nav_focus {
                    let len = self.navigation().len();
                    self.nav.select(Some(len.saturating_sub(1)));
                } else {
                    self.selected = self.rows.len().saturating_sub(1);
                    self.table
                        .select((!self.rows.is_empty()).then_some(self.selected));
                }
                None
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.nav_focus = !self.nav_focus;
                if self.nav_focus {
                    self.nav.select(Some(0));
                }
                None
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k') => {
                let up = matches!(key.code, KeyCode::Up | KeyCode::Char('k'));
                if self.nav_focus {
                    let len = self.navigation().len();
                    let i = self.nav.selected().unwrap_or(0);
                    self.nav.select(Some(if up {
                        i.saturating_sub(1)
                    } else {
                        (i + 1).min(len.saturating_sub(1))
                    }));
                } else {
                    self.selected = if up {
                        self.selected.saturating_sub(1)
                    } else {
                        (self.selected + 1).min(self.rows.len().saturating_sub(1))
                    };
                    self.table
                        .select((!self.rows.is_empty()).then_some(self.selected));
                }
                None
            }
            KeyCode::Enter if self.nav_focus => self
                .navigation()
                .get(self.nav.selected().unwrap_or(0))
                .map(|(_, v)| Action::View(v.clone())),
            KeyCode::Enter => Some(Action::Play(self.selected)),
            KeyCode::Delete | KeyCode::Backspace => self.current().map(Action::Remove),
            KeyCode::Esc => {
                if self.full_lyrics {
                    self.full_lyrics = false;
                } else if self.view == View::Search {
                    self.navigate(self.before_search.clone())?;
                }
                None
            }
            _ => None,
        };
        if let Some(action) = action {
            self.act(action)?;
        }
        Ok(false)
    }
    fn navigation(&self) -> Vec<(String, View)> {
        let mut items = vec![
            (format!("♥ 我喜欢  {}", self.liked.len()), View::Likes),
            (
                format!("≡ 播放队列  {}", self.state.queue.len()),
                View::Queue,
            ),
            ("◷ 最近播放".into(), View::History),
        ];
        items.extend(
            self.folders
                .iter()
                .map(|(n, c)| (format!("{n}  {c}"), View::Folder(n.clone()))),
        );
        items
    }
    fn message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::State(state) => {
                self.state = *state;
                self.reload()?;
            }
            Message::Offline => {
                self.state.phase = "stopped".into();
                self.state.track = None;
            }
            Message::Error(error) => self.notice(error),
            Message::Search(serial, result) if serial == self.serial => {
                self.searching = false;
                match result {
                    Ok((total, tracks)) => {
                        self.store.remember(&tracks)?;
                        self.search_rows = tracks;
                        self.total = total;
                        self.selected = 0;
                        self.rows.clear();
                        self.reload()?;
                    }
                    Err(error) => self.search_error = error,
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn hit(&mut self, rect: Rect, action: Action) {
        if rect.width > 0 && rect.height > 0 {
            self.hits.push(Hit { rect, action });
        }
    }
    fn button(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        label: &str,
        action: Action,
        primary: bool,
    ) {
        if area.width == 0 {
            return;
        }
        frame.render_widget(
            Paragraph::new(label).alignment(Alignment::Center).style(
                Style::default()
                    .fg(if primary { BG } else { FG })
                    .bg(if primary { ACCENT } else { PANEL }),
            ),
            area,
        );
        self.hit(area, action);
    }
    fn mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) -> Result<()> {
        match kind {
            MouseEventKind::Down(event::MouseButton::Left) => {
                if self.popup.is_none()
                    && self.timeline.contains(Position::new(x, y))
                    && ["playing", "paused"].contains(&self.state.phase.as_str())
                {
                    if let Some(track) = &self.state.track {
                        let duration = self.state.duration.max(track.duration);
                        self.seek = Some((
                            track.id.clone(),
                            duration,
                            seek_position(self.timeline, x, duration),
                        ));
                    }
                } else if let Some(hit) = self
                    .hits
                    .iter()
                    .rev()
                    .find(|h| h.rect.contains(Position::new(x, y)))
                    .cloned()
                {
                    self.act(hit.action)?;
                }
            }
            MouseEventKind::Drag(event::MouseButton::Left) => {
                if let Some((_, duration, value)) = &mut self.seek {
                    *value = seek_position(self.timeline, x, *duration);
                }
            }
            MouseEventKind::Up(event::MouseButton::Left) => {
                if let Some((id, _, value)) = self.seek.take() {
                    self.send(json!({"op":"seek_to","track_id":id,"position":value}));
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let code = if kind == MouseEventKind::ScrollUp {
                    KeyCode::Up
                } else {
                    KeyCode::Down
                };
                self.key(KeyEvent::new(code, KeyModifiers::NONE))?;
            }
            _ => {}
        }
        Ok(())
    }
    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.hits.clear();
        self.timeline = Rect::default();
        frame.render_widget(Block::default().style(Style::default().bg(BG).fg(FG)), area);
        if area.width < 30 || area.height < 16 {
            return;
        }
        let inner = area.inner(Margin::new(2, 1));
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(inner);
        let header = rows[0];
        frame.render_widget(
            Paragraph::new("wocli / music").style(Style::default().fg(ACCENT).bold()),
            header,
        );
        let mut right = header.right();
        for (label, action) in [
            (" ? ", Action::Help),
            (" 下载 ", Action::Jobs),
            (" 歌词 ", Action::Lyrics),
            (" 搜索 / ", Action::Search),
        ] {
            let width = label.width() as u16;
            right = right.saturating_sub(width);
            self.button(
                frame,
                Rect::new(right, header.y, width, 1),
                label,
                action,
                false,
            );
            right = right.saturating_sub(1);
        }
        if header.width >= 95 {
            let width = 12;
            right = right.saturating_sub(width);
            self.button(
                frame,
                Rect::new(right, header.y, width, 1),
                &format!("{} ▾", self.state.default_quality),
                Action::Quality(false),
                false,
            );
        }
        let body = rows[1];
        if self.full_lyrics {
            self.draw_lyrics(frame, body, true);
        } else {
            let sidebar = if body.width >= 70 { 19 } else { 0 };
            let cols =
                Layout::horizontal([Constraint::Length(sidebar), Constraint::Min(10)]).split(body);
            if sidebar > 0 {
                self.draw_nav(frame, cols[0]);
            }
            let music = cols[1].inner(Margin::new(u16::from(sidebar > 0), 0));
            if body.width >= 110 && self.state.track.is_some() {
                let cols =
                    Layout::horizontal([Constraint::Percentage(63), Constraint::Percentage(37)])
                        .split(music);
                self.draw_list(frame, cols[0].inner(Margin::new(0, 0)));
                self.draw_lyrics(frame, cols[1].inner(Margin::new(1, 0)), false);
            } else {
                self.draw_list(frame, music);
            }
        }
        self.draw_transport(frame, rows[3]);
        if let Some((value, at)) = &self.toast {
            if at.elapsed() < Duration::from_secs(5) {
                frame.render_widget(
                    Paragraph::new(value.as_str()).style(Style::default().fg(ACCENT)),
                    rows[2],
                );
            }
        }
        if let Some(popup) = self.popup.clone() {
            self.hits.clear();
            self.draw_popup(frame, popup);
        }
    }
    fn draw_nav(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(
            Paragraph::new("音乐库").style(Style::default().fg(DIM)),
            Rect::new(area.x, area.y, area.width, 1),
        );
        self.button(
            frame,
            Rect::new(area.right() - 3, area.y, 3, 1),
            " + ",
            Action::NewFolder(None),
            false,
        );
        let list = Rect::new(
            area.x,
            area.y + 2,
            area.width.saturating_sub(1),
            area.height.saturating_sub(2),
        );
        let nav = self.navigation();
        let items: Vec<ListItem> = nav
            .iter()
            .map(|(label, view)| {
                ListItem::new(label.clone()).style(
                    Style::default()
                        .fg(if *view == self.view { ACCENT } else { DIM })
                        .bg(if *view == self.view { SELECTED } else { BG }),
                )
            })
            .collect();
        let mut state = self.nav.clone();
        if !self.nav_focus {
            state.select(None);
        }
        frame.render_stateful_widget(
            List::new(items).highlight_style(Style::default().fg(FG).bg(SELECTED)),
            list,
            &mut state,
        );
        let offset = state.offset();
        self.nav = state;
        for (i, (_, view)) in nav
            .iter()
            .enumerate()
            .skip(offset)
            .take(list.height as usize)
        {
            self.hit(
                Rect::new(list.x, list.y + (i - offset) as u16, list.width, 1),
                Action::View(view.clone()),
            );
        }
    }
    fn draw_list(&mut self, frame: &mut Frame, area: Rect) {
        let title = match &self.view {
            View::Likes => "我喜欢",
            View::Queue => "播放队列",
            View::History => "最近播放",
            View::Folder(n) => n,
            View::Search => "搜索",
        }
        .to_string();
        let mut top = area.y;
        if self.view == View::Search {
            let entry = Rect::new(area.x, top, area.width.saturating_sub(8), 1);
            frame.render_widget(
                Paragraph::new(format!(
                    "/ {}{}",
                    self.query,
                    if self.editing { "▏" } else { "" }
                ))
                .style(
                    Style::default()
                        .fg(if self.editing { ACCENT } else { FG })
                        .bg(PANEL),
                ),
                entry,
            );
            self.hit(entry, Action::Search);
            self.button(
                frame,
                Rect::new(area.right().saturating_sub(7), top, 7, 1),
                "搜索",
                Action::SearchGo,
                true,
            );
            top += 2;
        }
        let count = if self.view == View::Search {
            self.total
        } else {
            self.rows.len()
        };
        frame.render_widget(
            Paragraph::new(format!("{title}  {count}")).style(Style::default().fg(ACCENT).bold()),
            Rect::new(area.x, top, area.width, 1),
        );
        top += 2;
        let mut x = area.x;
        self.button(
            frame,
            Rect::new(x, top, 10.min(area.width), 1),
            "▶ 播放全部",
            Action::Play(0),
            false,
        );
        x += 11;
        if let Some(track) = self.rows.get(self.selected).cloned() {
            if x + 9 <= area.right() {
                self.button(
                    frame,
                    Rect::new(x, top, 9, 1),
                    "收藏到…",
                    Action::Collect(track.clone()),
                    false,
                );
                x += 10;
            }
            if x + 6 <= area.right() {
                self.button(
                    frame,
                    Rect::new(x, top, 6, 1),
                    "操作",
                    Action::Song(track),
                    false,
                );
                x += 7;
            }
        }
        if let View::Folder(name) = &self.view {
            if x + 3 <= area.right() {
                self.button(
                    frame,
                    Rect::new(x, top, 3, 1),
                    " ⋯ ",
                    Action::FolderMenu(name.clone()),
                    false,
                );
            }
        }
        top += 2;
        let tablearea = Rect::new(
            area.x,
            top,
            area.width,
            area.bottom()
                .saturating_sub(top + u16::from(self.view == View::Search) * 2),
        );
        let current = self.state.track.as_ref().map(|t| t.id.as_str());
        let data: Vec<Row> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let marker = if Some(t.id.as_str()) == current {
                    "▶".into()
                } else {
                    format!(
                        "{}",
                        i + 1
                            + (if self.view == View::Search {
                                (self.page - 1) * 20
                            } else {
                                0
                            })
                    )
                };
                Row::new(vec![
                    marker,
                    format!(
                        "{}{}",
                        if self.liked.contains(&t.id) {
                            "♥ "
                        } else {
                            ""
                        },
                        t.title
                    ),
                    t.artist.clone(),
                    clock(t.duration),
                ])
                .style(Style::default().fg(if Some(t.id.as_str()) == current {
                    ACCENT
                } else {
                    FG
                }))
            })
            .collect();
        let table = Table::new(
            data,
            [
                Constraint::Length(3),
                Constraint::Percentage(62),
                Constraint::Percentage(38),
                Constraint::Length(5),
            ],
        )
        .column_spacing(1)
        .header(Row::new(["", "歌曲", "歌手", "时长"]).style(Style::default().fg(DIM).bg(PANEL)))
        .row_highlight_style(Style::default().bg(SELECTED).fg(ACCENT));
        frame.render_stateful_widget(table, tablearea, &mut self.table);
        let offset = self.table.offset();
        for i in offset
            ..self
                .rows
                .len()
                .min(offset + tablearea.height.saturating_sub(1) as usize)
        {
            self.hit(
                Rect::new(
                    tablearea.x,
                    tablearea.y + 1 + (i - offset) as u16,
                    tablearea.width,
                    1,
                ),
                Action::Select(i),
            );
        }
        if self.view == View::Search {
            let y = area.bottom().saturating_sub(1);
            if !self.search_error.is_empty() {
                frame.render_widget(
                    Paragraph::new(self.search_error.clone()).style(Style::default().fg(BAD)),
                    Rect::new(area.x, y, area.width, 1),
                );
            } else if self.searching {
                frame.render_widget(
                    Paragraph::new("搜索中…").style(Style::default().fg(DIM)),
                    Rect::new(area.x, y, area.width, 1),
                );
            } else {
                self.button(
                    frame,
                    Rect::new(area.x, y, 6, 1),
                    "←",
                    Action::Page(-1),
                    false,
                );
                frame.render_widget(
                    Paragraph::new(format!(
                        "{} / {}",
                        self.page,
                        self.total.div_ceil(20).max(1)
                    ))
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(DIM)),
                    Rect::new(area.x + 7, y, 12, 1),
                );
                if area.width >= 26 {
                    self.button(
                        frame,
                        Rect::new(area.x + 20, y, 6, 1),
                        "→",
                        Action::Page(1),
                        false,
                    );
                }
            }
        }
    }
    fn draw_lyrics(&mut self, frame: &mut Frame, area: Rect, full: bool) {
        frame.render_widget(Block::default().style(Style::default().bg(PANEL)), area);
        let inner = area.inner(Margin::new(2, 1));
        if inner.height < 5 {
            return;
        }
        frame.render_widget(
            Paragraph::new("歌词").style(Style::default().fg(DIM)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        if full {
            self.button(
                frame,
                Rect::new(inner.right().saturating_sub(6), inner.y, 6, 1),
                "返回",
                Action::Lyrics,
                false,
            );
        }
        if let Some(track) = self.state.track.clone() {
            frame.render_widget(
                Paragraph::new(track.title.clone()).style(Style::default().fg(ACCENT).bold()),
                Rect::new(inner.x, inner.y + 2, inner.width, 1),
            );
            frame.render_widget(
                Paragraph::new(track.artist.clone()).style(Style::default().fg(DIM)),
                Rect::new(inner.x, inner.y + 3, inner.width, 1),
            );
            let content = Rect::new(
                inner.x,
                inner.y + 5,
                inner.width,
                inner.height.saturating_sub(7),
            );
            if self.state.phase == "error" {
                frame.render_widget(
                    Paragraph::new(self.state.error.clone())
                        .wrap(Wrap { trim: true })
                        .style(Style::default().fg(BAD)),
                    Rect::new(content.x, content.y, content.width, 3.min(content.height)),
                );
                if content.height >= 5 {
                    self.button(
                        frame,
                        Rect::new(content.x, content.y + 4, 6, 1),
                        "重试",
                        Action::Control(json!({"op":"retry"})),
                        true,
                    );
                    self.button(
                        frame,
                        Rect::new(
                            content.x + 7,
                            content.y + 4,
                            10.min(content.width.saturating_sub(7)),
                            1,
                        ),
                        "切换音质",
                        Action::Quality(true),
                        false,
                    );
                }
            } else {
                let lines = lyrics::parse(&self.state.lyrics);
                if lines.is_empty() {
                    let label = match self.state.lyrics_status.as_str() {
                        "loading" => "歌词加载中…",
                        "unavailable" | "error" => {
                            if self.state.lyrics_error.is_empty() {
                                "未取得歌词"
                            } else {
                                &self.state.lyrics_error
                            }
                        }
                        _ => "",
                    };
                    frame.render_widget(
                        Paragraph::new(label)
                            .wrap(Wrap { trim: true })
                            .style(Style::default().fg(DIM)),
                        Rect::new(content.x, content.y, content.width, 3.min(content.height)),
                    );
                    if self.state.lyrics_status != "loading" && content.height >= 5 {
                        self.button(
                            frame,
                            Rect::new(content.x, content.y + 4, 10.min(content.width), 1),
                            "重试歌词",
                            Action::Control(json!({"op":"lyrics_retry"})),
                            false,
                        );
                    }
                } else {
                    let position = self
                        .seek
                        .as_ref()
                        .map(|v| v.2)
                        .unwrap_or(self.state.position)
                        + self.state.lyric_offset as f64 / 1000.;
                    let index = lyrics::current(&lines, position, 0);
                    let visible = (content.height as usize / 2).max(1);
                    let start = index.unwrap_or(0).saturating_sub(visible / 2);
                    for (i, line) in lines.iter().enumerate().skip(start).take(visible) {
                        frame.render_widget(
                            Paragraph::new(line.text.clone())
                                .alignment(Alignment::Center)
                                .style(
                                    Style::default()
                                        .fg(if Some(i) == index { ACCENT } else { DIM })
                                        .add_modifier(if Some(i) == index {
                                            Modifier::BOLD
                                        } else {
                                            Modifier::empty()
                                        }),
                                ),
                            Rect::new(
                                content.x,
                                content.y + ((i - start) * 2) as u16,
                                content.width,
                                1,
                            ),
                        );
                    }
                }
            }
            let y = inner.bottom().saturating_sub(1);
            self.button(
                frame,
                Rect::new(inner.x, y, 9.min(inner.width), 1),
                "收藏到…",
                Action::Collect(track.clone()),
                false,
            );
            if inner.width >= 17 {
                self.button(
                    frame,
                    Rect::new(inner.x + 10, y, 7, 1),
                    "操作",
                    Action::Song(track),
                    false,
                );
            }
            if inner.width >= 28 {
                self.button(
                    frame,
                    Rect::new(inner.x + 18, y, 10, 1),
                    "刷新歌词",
                    Action::Control(json!({"op":"lyrics_retry"})),
                    false,
                );
            }
        }
    }
    fn draw_transport(&mut self, frame: &mut Frame, area: Rect) {
        let y = area.y + 1;
        let title = self
            .state
            .track
            .as_ref()
            .map(|t| format!("{} · {}", t.title, t.artist))
            .unwrap_or_default();
        let controls = if area.width >= 95 {
            64
        } else {
            area.width.min(64)
        };
        let titlewidth = area.width.saturating_sub(controls + 2);
        if titlewidth > 0 {
            frame.render_widget(
                Paragraph::new(title).style(Style::default().fg(ACCENT)),
                Rect::new(area.x, y, titlewidth, 1),
            );
        }
        let mut x = area.right().saturating_sub(controls);
        for (label, width, action, primary) in [
            (
                "⏮".to_string(),
                5,
                Action::Control(json!({"op":"prev"})),
                false,
            ),
            (
                (match self.state.phase.as_str() {
                    "playing" => "暂停",
                    "loading" => "取消",
                    "error" => "重试",
                    _ => "播放",
                })
                .into(),
                6,
                Action::Toggle,
                true,
            ),
            ("⏭".into(), 5, Action::Control(json!({"op":"next"})), false),
            (
                if self.state.liked { "♥" } else { "♡" }.into(),
                4,
                Action::Control(json!({"op":"like"})),
                false,
            ),
            (
                format!("{} ▾", mode_label(&self.state.mode)),
                14,
                Action::Mode,
                false,
            ),
            ("−".into(), 3, Action::Volume(-5), false),
        ] {
            if x + width <= area.right() {
                self.button(frame, Rect::new(x, y, width, 1), &label, action, primary);
                x += width + 1;
            }
        }
        if x + 5 <= area.right() {
            frame.render_widget(
                Paragraph::new(format!("{}%", self.state.volume))
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(DIM)),
                Rect::new(x, y, 5, 1),
            );
            x += 5;
        }
        if x + 3 <= area.right() {
            self.button(frame, Rect::new(x, y, 3, 1), "+", Action::Volume(5), false);
        }
        let position = self
            .seek
            .as_ref()
            .map(|v| v.2)
            .unwrap_or(self.state.position);
        let duration = self
            .state
            .duration
            .max(self.state.track.as_ref().map(|t| t.duration).unwrap_or(0.));
        let y = area.y + 2;
        frame.render_widget(
            Paragraph::new(clock(position)).style(Style::default().fg(DIM)),
            Rect::new(area.x, y, 5, 1),
        );
        let suffix = if self.state.phase == "loading" {
            if self.state.total > 0 {
                format!("缓冲 {}%", self.state.received * 100 / self.state.total)
            } else {
                "解析中".into()
            }
        } else if self.state.phase == "error" {
            match self.state.error_code.as_str() {
                "incomplete_audio" => "仅试听",
                "source_unavailable" => "无音源",
                _ => "播放失败",
            }
            .into()
        } else {
            self.state.quality.clone()
        };
        let tail = if area.width >= 60 {
            format!("{}  {suffix}", clock(duration))
        } else {
            clock(duration)
        };
        let width = tail.width() as u16;
        frame.render_widget(
            Paragraph::new(tail).style(Style::default().fg(if self.state.phase == "error" {
                BAD
            } else {
                DIM
            })),
            Rect::new(
                area.right().saturating_sub(width),
                y,
                width.min(area.width),
                1,
            ),
        );
        self.timeline = Rect::new(area.x + 7, y, area.width.saturating_sub(width + 9), 1);
        frame.render_widget(
            Gauge::default()
                .ratio(if duration > 0. {
                    (position / duration).clamp(0., 1.)
                } else {
                    0.
                })
                .gauge_style(Style::default().fg(ACCENT).bg(PANEL))
                .label(""),
            self.timeline,
        );
        self.hit(self.timeline, Action::Seek);
    }
    fn draw_popup(&mut self, frame: &mut Frame, popup: Popup) {
        let (title, height) = match &popup {
            Popup::Menu(m) => (m.title.as_str(), m.items.len() as u16 + 6),
            Popup::Input { title, .. } => (title.as_str(), 7),
            Popup::Download { .. } => ("下载歌曲", 13),
            Popup::Jobs(_) => ("下载任务", 18),
            Popup::Help => ("快捷键", 22),
        };
        let outer = frame.area();
        let width = outer.width.saturating_sub(4).min(66);
        let height = height.min(outer.height.saturating_sub(2));
        let rect = Rect::new(
            outer.x + (outer.width - width) / 2,
            outer.y + (outer.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Block::default()
                .title(format!(" {title} "))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM))
                .style(Style::default().bg(PANEL).fg(FG)),
            rect,
        );
        let body = rect.inner(Margin::new(2, 1));
        self.button(
            frame,
            Rect::new(rect.right().saturating_sub(4), rect.y, 3, 1),
            " × ",
            Action::Close,
            false,
        );
        match popup {
            Popup::Menu(menu)=>{
                frame.render_widget(Paragraph::new(menu.caption).style(Style::default().fg(DIM)),Rect::new(body.x,body.y,body.width,1));
                let list=Rect::new(body.x,body.y+2,body.width,body.height.saturating_sub(2));
                let items:Vec<ListItem>=menu.items.iter().map(|(label,_)|ListItem::new(format!(" {label}"))).collect();let mut state=ListState::default().with_selected(Some(menu.selected));
                frame.render_stateful_widget(List::new(items).highlight_style(Style::default().fg(ACCENT).bg(SELECTED)),list,&mut state);
                for (i,(_,action)) in menu.items.iter().enumerate().skip(state.offset()).take(list.height as usize){self.hit(Rect::new(list.x,list.y+(i-state.offset()) as u16,list.width,1),action.clone());}
            }
            Popup::Input{value,..}=>{
                frame.render_widget(Paragraph::new(format!("{value}▏")).style(Style::default().fg(ACCENT).bg(BG)),Rect::new(body.x,body.y+1,body.width,1));
                // Mouse and keyboard use the same validation path.
                self.button(frame,Rect::new(body.x,body.bottom().saturating_sub(1),8,1),"保存 ↵",Action::Close,true);
                if let Some(hit)=self.hits.last_mut(){hit.action=Action::SubmitInput;}
            }
            Popup::Download{track,directory,quality,lyrics,focus}=>{
                frame.render_widget(Paragraph::new(format!("{} · {}",track.title,track.artist)).style(Style::default().fg(ACCENT)),Rect::new(body.x,body.y,body.width,1));
                frame.render_widget(Paragraph::new("目录").style(Style::default().fg(DIM)),Rect::new(body.x,body.y+2,body.width,1));
                frame.render_widget(Paragraph::new(format!("{directory}{}",if focus==0{"▏"}else{""})).style(Style::default().fg(FG).bg(if focus==0{SELECTED}else{BG})),Rect::new(body.x,body.y+3,body.width,1));self.hit(Rect::new(body.x,body.y+3,body.width,1),Action::DownloadField(0));
                self.button(frame,Rect::new(body.x,body.y+5,20.min(body.width),1),&format!("音质  {} ▾",QUALITIES[quality]),Action::DownloadField(1),focus==1);
                self.button(frame,Rect::new(body.x,body.y+7,20.min(body.width),1),if lyrics{"[✓] 保存 LRC 歌词"}else{"[ ] 保存 LRC 歌词"},Action::DownloadField(2),focus==2);
                self.button(frame,Rect::new(body.x,body.bottom().saturating_sub(1),12,1),"开始下载",Action::DownloadGo,true);
            }
            Popup::Jobs(index)=>{
                let jobs=self.state.downloads.clone();
                let list=Rect::new(body.x,body.y,body.width,body.height.saturating_sub(4));
                let items:Vec<ListItem>=jobs.iter().map(|j|ListItem::new(format!("{}  {}{}",job_phase(&j.phase),j.track.title,if j.total>0{format!("  {}%",j.received*100/j.total)}else{String::new()}))).collect();
                let mut state=ListState::default().with_selected(Some(index));frame.render_stateful_widget(List::new(items).highlight_style(Style::default().fg(ACCENT).bg(SELECTED)),list,&mut state);
                for i in state.offset()..jobs.len().min(state.offset()+list.height as usize){self.hit(Rect::new(list.x,list.y+(i-state.offset()) as u16,list.width,1),Action::JobSelect(i));}
                if let Some(job)=jobs.get(index){
                    let detail=if !job.error.is_empty(){&job.error}else if !job.warning.is_empty(){&job.warning}else{&job.path};
                    frame.render_widget(Paragraph::new(detail.clone()).style(Style::default().fg(if job.phase=="error"{BAD}else{DIM})),Rect::new(body.x,body.bottom().saturating_sub(3),body.width,1));
                    if ["error","cancelled"].contains(&job.phase.as_str()){self.button(frame,Rect::new(body.x,body.bottom().saturating_sub(1),10,1),"重试 r",Action::Control(json!({"op":"download_retry","id":job.id})),true);}
                    else if !["done","cancelling"].contains(&job.phase.as_str()){self.button(frame,Rect::new(body.x,body.bottom().saturating_sub(1),10,1),"取消 c",Action::Control(json!({"op":"download_cancel","id":job.id})),false);}
                }
            }
            Popup::Help=>frame.render_widget(Paragraph::new("/  搜索\nEnter  播放所选歌曲\nSpace  暂停 / 继续\nj / k  上下移动\ng / G  列表顶部 / 底部\n[ / ]  上一首 / 下一首\nCtrl+U / D  翻页\n← →  跳转 5 秒\n+ −  音量调节\nc  收藏到…      f  喜欢\nx  歌曲操作     d  下载\na  加入队列     m  播放方式\nv  展开 / 收起歌词\nl  刷新歌词     Ctrl+R  重试歌曲\nTab  切换侧栏   b  音乐库\nEsc  返回       q  退出界面").style(Style::default().fg(FG)),body),
        }
    }
}
fn job_phase(phase: &str) -> &str {
    match phase {
        "queued" => "排队",
        "resolving" => "解析",
        "downloading" => "下载",
        "lyrics" => "歌词",
        "done" => "完成",
        "error" => "失败",
        "cancelled" => "取消",
        "cancelling" => "取消中",
        _ => phase,
    }
}
fn seek_position(rect: Rect, x: u16, duration: f64) -> f64 {
    if rect.width <= 1 {
        0.
    } else {
        f64::from(x.saturating_sub(rect.x).min(rect.width - 1)) / f64::from(rect.width - 1)
            * duration
    }
}
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            std::io::stdout(),
            event::DisableMouseCapture,
            event::DisableBracketedPaste
        );
        ratatui::restore();
    }
}
fn request_message(paths: &Paths, request: Request) -> Message {
    match ipc::request(paths, request.value, request.start) {
        Ok(Some(value)) => match serde_json::from_value(value) {
            Ok(state) => Message::State(Box::new(state)),
            Err(error) => Message::Error(error.to_string()),
        },
        Ok(None) => Message::Offline,
        Err(error) => Message::Error(error.to_string()),
    }
}
pub fn run(paths: Paths) -> Result<()> {
    let (tx, rx) = mpsc::channel::<Request>();
    let (events, receiver): (Sender<Message>, Receiver<Message>) = mpsc::channel();
    let worker_paths = paths.clone();
    let worker_events = events.clone();
    thread::spawn(move || {
        // A seek can wait for future download data. Keep status, stop and next
        // requests flowing while it waits, with bounded concurrency for seeks.
        let seeks = crate::pool::WorkerPool::new(2);
        for request in rx {
            if matches!(request.value["op"].as_str(), Some("seek" | "seek_to")) {
                let paths = worker_paths.clone();
                let events = worker_events.clone();
                seeks.execute(move || {
                    // State is refreshed by the normal poll. A delayed seek
                    // acknowledgement must not overwrite a newer stop/next response.
                    if let Message::Error(error) = request_message(&paths, request) {
                        let _ = events.send(Message::Error(error));
                    }
                });
            } else {
                let _ = worker_events.send(request_message(&worker_paths, request));
            }
        }
    });
    let mut app = App::new(paths, tx, events)?;
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    crossterm::execute!(
        std::io::stdout(),
        event::EnableMouseCapture,
        event::EnableBracketedPaste
    )?;
    let mut polled = Instant::now() - Duration::from_secs(1);
    let mut quit = false;
    while !quit {
        if polled.elapsed() >= Duration::from_millis(350) {
            let _ = app.tx.send(Request {
                value: json!({"op":"status"}),
                start: false,
            });
            polled = Instant::now();
        }
        while let Ok(message) = receiver.try_recv() {
            if let Err(e) = app.message(message) {
                app.notice(e.to_string());
            }
        }
        terminal.draw(|f| app.draw(f))?;
        if event::poll(Duration::from_millis(40))? {
            let result = match event::read()? {
                Event::Key(key) if key.kind != event::KeyEventKind::Release => {
                    app.key(key).map(|v| quit = v)
                }
                Event::Mouse(mouse) => app.mouse(mouse.kind, mouse.column, mouse.row),
                Event::Paste(text) => {
                    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
                    match &mut app.popup {
                        Some(Popup::Input { value, .. }) => value.push_str(&clean),
                        Some(Popup::Download {
                            directory,
                            focus: 0,
                            ..
                        }) => directory.push_str(&clean),
                        None if app.editing => app.query.push_str(&clean),
                        _ => {}
                    }
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(e) = result {
                app.notice(e.to_string());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};
    #[test]
    fn progress_clamps_to_track() {
        let r = Rect::new(10, 0, 101, 1);
        assert_eq!(seek_position(r, 60, 200.), 100.);
        assert_eq!(seek_position(r, 999, 200.), 200.);
        assert_eq!(seek_position(r, 0, 200.), 0.);
    }
    #[test]
    fn layouts_keep_collection_entry_and_render_small_sizes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: temp.path().into(),
            runtime: temp.path().join("runtime"),
        };
        let (tx, _) = mpsc::channel();
        let (events, _) = mpsc::channel();
        let mut app = App::new(paths, tx, events).unwrap();
        app.store
            .favorite(&Track {
                id: "1".into(),
                title: "晴天".into(),
                artist: "周杰伦".into(),
                duration: 269.,
                ..Track::default()
            })
            .unwrap();
        app.reload().unwrap();
        for (w, h) in [(123, 41), (80, 26), (60, 24), (30, 10)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| app.draw(f)).unwrap();
            if w >= 60 {
                assert!(app
                    .hits
                    .iter()
                    .any(|hit| matches!(hit.action, Action::Collect(_))));
            }
        }
    }
}

#[cfg(test)]
mod screenshots {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};
    fn xml(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
    fn color(value: Color) -> String {
        match value {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            _ => "#dce3d2".into(),
        }
    }
    #[test]
    #[ignore = "run through scripts/screenshots_native.py"]
    fn export() {
        let output = std::env::var("WOCLI_SCREENSHOTS").unwrap();
        let root = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: root.path().into(),
            runtime: root.path().join("run"),
        };
        let (tx, _) = mpsc::channel();
        let (events, _) = mpsc::channel();
        let mut app = App::new(paths, tx, events).unwrap();
        let tracks: Vec<Track> = [
            ("夜航", "回声计划", 254.),
            ("雨后的街道", "林间", 218.),
            ("山海之间", "远山", 286.),
            ("Sunday Morning", "Paper Moon", 231.),
            ("散步到月亮升起", "回声计划", 265.),
            ("光の向こう", "青い部屋", 243.),
            ("末班车", "林间", 207.),
            ("A Quiet Place", "Paper Moon", 312.),
        ]
        .iter()
        .enumerate()
        .map(|(i, (title, artist, duration))| Track {
            id: format!("demo:{i}"),
            title: (*title).into(),
            artist: (*artist).into(),
            duration: *duration,
            ..Track::default()
        })
        .collect();
        for t in &tracks {
            app.store.collect("深夜电台", t).unwrap();
        }
        for t in &tracks[1..4] {
            app.store.collect("通勤路上", t).unwrap();
        }
        app.store.create_folder("慢慢听").unwrap();
        app.store.favorite(&tracks[0]).unwrap();
        app.store.favorite(&tracks[2]).unwrap();
        app.navigate(View::Folder("深夜电台".into())).unwrap();
        app.state=Status{track:Some(tracks[0].clone()),queue:tracks.clone(),phase:"playing".into(),position:93.,duration:254.,quality:"320k".into(),requested_quality:"best".into(),volume:60,mode:"repeat".into(),liked:true,lyrics_status:"ready".into(),lyrics:"[00:00]城市的灯，一盏盏亮起\n[00:15]晚风翻过没读完的书\n[00:30]穿过街角，走过长桥\n[00:45]远处的列车驶向星空\n[01:00]让脚步慢一些\n[01:15]让旋律再停留一会\n[01:30]把一整天的喧闹，留在身后\n[01:45]今晚的路还有很长\n[02:00]沿着灯火，慢慢回家\n[02:15]一首歌，陪你走到天亮".into(),..Status::default()};
        app.reload().unwrap();
        std::fs::create_dir_all(&output).unwrap();
        for (name, width, height, collection) in [
            ("library", 123, 41, false),
            ("collections", 123, 41, true),
            ("compact", 80, 26, false),
        ] {
            app.popup = None;
            if collection {
                app.collect(tracks[0].clone()).unwrap();
            }
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| app.draw(f)).unwrap();
            let buffer = terminal.backend().buffer();
            let mut svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}"><rect width="100%" height="100%" fill="#111914"/><g font-family="Menlo, monospace, PingFang SC" font-size="14">"##,
                width * 9 + 24,
                height * 19 + 24,
                width * 9 + 24,
                height * 19 + 24
            );
            for y in 0..height {
                let mut skip = false;
                for x in 0..width {
                    if skip {
                        skip = false;
                        continue;
                    }
                    let cell = &buffer[(x, y)];
                    let columns = cell.symbol().width().max(1);
                    skip = columns == 2;
                    if cell.bg != Color::Reset {
                        svg.push_str(&format!(
                            r#"<rect x="{}" y="{}" width="{}" height="19" fill="{}"/>"#,
                            x * 9 + 12,
                            y * 19 + 12,
                            columns * 9,
                            color(cell.bg)
                        ));
                    }
                }
            }
            for y in 0..height {
                for x in 0..width {
                    let cell = &buffer[(x, y)];
                    if !cell.symbol().trim().is_empty() {
                        svg.push_str(&format!(
                            r#"<text x="{}" y="{}" fill="{}"{}>{}</text>"#,
                            x * 9 + 12,
                            y * 19 + 27,
                            color(cell.fg),
                            if cell.modifier.contains(Modifier::BOLD) {
                                " font-weight=\"bold\""
                            } else {
                                ""
                            },
                            xml(cell.symbol())
                        ));
                    }
                }
            }
            svg.push_str("</g></svg>");
            std::fs::write(
                std::path::Path::new(&output).join(format!("{name}.svg")),
                svg,
            )
            .unwrap();
        }
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, App, Receiver<Request>) {
        let root = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let (events, _) = mpsc::channel();
        let app = App::new(
            Paths {
                home: root.path().into(),
                runtime: root.path().join("run"),
            },
            tx,
            events,
        )
        .unwrap();
        (root, app, rx)
    }
    #[test]
    fn collection_context_is_selected_song_or_current_lyrics_song() {
        let (_root, mut app, _rx) = setup();
        let selected = Track {
            id: "1".into(),
            title: "选中".into(),
            ..Track::default()
        };
        let playing = Track {
            id: "2".into(),
            title: "播放中".into(),
            ..Track::default()
        };
        app.store.favorite(&selected).unwrap();
        app.store.create_folder("日常").unwrap();
        app.reload().unwrap();
        app.state.track = Some(playing.clone());
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(&app.popup,Some(Popup::Menu(m)) if m.caption==selected.title));
        app.act(Action::Collected("日常".into(), selected)).unwrap();
        assert_eq!(app.store.folder("日常").unwrap()[0].id, "1");
        app.full_lyrics = true;
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(&app.popup,Some(Popup::Menu(m)) if m.caption==playing.title));
    }
    #[test]
    fn drag_keeps_original_track_identity_when_song_changes() {
        let (_root, mut app, rx) = setup();
        app.state.phase = "paused".into();
        app.state.duration = 200.;
        app.state.track = Some(Track {
            id: "old".into(),
            ..Default::default()
        });
        app.timeline = Rect::new(10, 20, 101, 1);
        app.mouse(MouseEventKind::Down(event::MouseButton::Left), 60, 20)
            .unwrap();
        app.state.track = Some(Track {
            id: "new".into(),
            ..Default::default()
        });
        app.mouse(MouseEventKind::Up(event::MouseButton::Left), 60, 20)
            .unwrap();
        let command = rx.try_recv().unwrap();
        assert_eq!(command.value["track_id"], "old");
        assert_eq!(command.value["position"], 100.);
    }
    #[test]
    fn text_input_accepts_space_and_q_without_quitting() {
        let (_root, mut app, _rx) = setup();
        app.act(Action::Search).unwrap();
        for c in ['C', 'r', 'y', ' ', 'q'] {
            assert!(!app
                .key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
                .unwrap());
        }
        assert_eq!(app.query, "Cry q");
        app.act(Action::NewFolder(None)).unwrap();
        if let Some(Popup::Input { value, .. }) = &mut app.popup {
            *value = "旅途".into();
        }
        app.act(Action::SubmitInput).unwrap();
        assert!(app
            .store
            .folders()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "旅途"));
    }
    #[test]
    fn vim_navigation_and_media_shortcuts() {
        let (_root, mut app, rx) = setup();
        for i in 1..=5 {
            let t = Track {
                id: format!("id-{i}"),
                title: format!("Title {i}"),
                ..Track::default()
            };
            app.store.favorite(&t).unwrap();
        }
        app.reload().unwrap();
        assert_eq!(app.rows.len(), 5);

        // j down, k up
        assert_eq!(app.selected, 0);
        app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.selected, 1);
        app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.selected, 0);

        // G to bottom, g to top
        app.key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.selected, 4);
        app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.selected, 0);

        // [ prev, ] next
        app.key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE))
            .unwrap();
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd.value["op"], "prev");

        app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE))
            .unwrap();
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd.value["op"], "next");
    }
}
