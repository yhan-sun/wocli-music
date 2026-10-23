use super::*;
use crate::playlists::{self, Conflict, Playlist};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone)]
pub(super) enum Stage {
    Input,
    Working(String),
    Choosing(Arc<Vec<Playlist>>),
    Preview(Arc<Playlist>),
    Review(Arc<Playlist>),
}
#[derive(Clone)]
pub(super) struct Dialog {
    pub path: String,
    pub name: String,
    pub list: String,
    pub title_first: bool,
    pub offline: bool,
    pub focus: u8,
    pub selected: usize,
    pub stage: Stage,
    pub error: String,
    pub export: Option<String>,
    pub existing: bool,
}
impl Default for Dialog {
    fn default() -> Self {
        Self {
            path: String::new(),
            name: String::new(),
            list: String::new(),
            title_first: false,
            offline: false,
            focus: 0,
            selected: 0,
            stage: Stage::Input,
            error: String::new(),
            export: None,
            existing: false,
        }
    }
}
pub(super) enum Update {
    Parsed(std::result::Result<Vec<Playlist>, String>),
    Progress(usize, usize),
    Resolved(std::result::Result<Playlist, String>),
}
impl App {
    pub(super) fn close_transfer(&mut self) {
        if let Some(cancel) = self.transfer_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.transfer_serial += 1;
        self.popup = None;
    }
    pub(super) fn open_transfer(&mut self, export: Option<String>) {
        self.close_transfer();
        let mut dialog = Dialog {
            export: export.clone(),
            ..Dialog::default()
        };
        if export.is_some() {
            dialog.path = "playlist.wocli.json".into();
            dialog.name = "json".into();
        }
        self.popup = Some(Popup::Transfer(dialog));
    }
    pub(super) fn transfer_go(&mut self) -> Result<()> {
        let Some(Popup::Transfer(mut dialog)) = self.popup.clone() else {
            return Ok(());
        };
        dialog.error.clear();
        match &dialog.stage {
            Stage::Working(_) => return Ok(()),
            Stage::Choosing(lists) => {
                if let Some(list) = lists.get(dialog.selected) {
                    self.transfer_resolve(dialog.clone(), list.clone())?;
                }
                return Ok(());
            }
            Stage::Review(list) => {
                self.transfer_resolve(dialog.clone(), (**list).clone())?;
                return Ok(());
            }
            Stage::Preview(list) => {
                let mut list = (**list).clone();
                if !dialog.name.trim().is_empty() {
                    list.name = dialog.name.trim().into();
                }
                match playlists::save(
                    &self.store,
                    list,
                    if dialog.existing {
                        Conflict::Replace
                    } else {
                        Conflict::Rename
                    },
                ) {
                    Ok(report) => {
                        self.close_transfer();
                        self.navigate(View::Folder(report.name.clone()))?;
                        let s = report.summary;
                        self.notice(format!("已导入 {}：可播放 {}，未匹配 {}，待确认 {}，重复 {}；playlist inspect 查看详情",report.name,s.matched,s.pending,s.ambiguous,s.duplicates));
                    }
                    Err(error) => {
                        dialog.error = error.to_string();
                        self.popup = Some(Popup::Transfer(dialog));
                    }
                }
                return Ok(());
            }
            Stage::Input => {}
        }
        if let Some(name) = &dialog.export {
            let result = playlists::stored(&self.store, name).and_then(|list| {
                playlists::write(
                    &list,
                    &dialog.name,
                    &crate::cli::expand_path(dialog.path.trim()),
                    false,
                )
            });
            match result {
                Ok(()) => {
                    self.close_transfer();
                    self.notice(format!("已导出到 {}", dialog.path));
                }
                Err(error) => {
                    dialog.error = error.to_string();
                    self.popup = Some(Popup::Transfer(dialog));
                }
            }
            return Ok(());
        }
        let path = crate::cli::expand_path(dialog.path.trim().trim_matches('"').trim_matches('\''));
        if !path.is_file() {
            dialog.error = "请选择本地歌单文件（可粘贴完整路径）".into();
            self.popup = Some(Popup::Transfer(dialog));
            return Ok(());
        }
        self.transfer_serial += 1;
        let serial = self.transfer_serial;
        let events = self.events.clone();
        let order = if dialog.title_first {
            "title-artist"
        } else {
            "artist-title"
        };
        dialog.stage = Stage::Working("正在解析歌单文件…".into());
        self.popup = Some(Popup::Transfer(dialog));
        thread::spawn(move || {
            let _ = events.send(Message::Transfer(
                serial,
                Update::Parsed(playlists::read(&path, order).map_err(|e| e.to_string())),
            ));
        });
        Ok(())
    }
    fn transfer_resolve(&mut self, mut dialog: Dialog, mut list: Playlist) -> Result<()> {
        let known = self.store.known_tracks()?;
        if !dialog.name.trim().is_empty() {
            list.name = dialog.name.trim().into();
        }
        dialog.stage = Stage::Working(format!(
            "正在匹配 {} 条歌曲…（Esc 可取消）",
            list.entries.len()
        ));
        self.transfer_serial += 1;
        let serial = self.transfer_serial;
        let cancel = Arc::new(AtomicBool::new(false));
        self.transfer_cancel = Some(cancel.clone());
        let paths = self.paths.clone();
        let events = self.events.clone();
        let offline = dialog.offline;
        let retry = dialog.existing;
        self.popup = Some(Popup::Transfer(dialog));
        thread::spawn(move || {
            let result = playlists::resolve(
                &mut list,
                &known,
                &paths,
                playlists::ResolveOptions {
                    offline,
                    direct: false,
                    retry,
                },
                &cancel,
                |done, total| {
                    let _ = events.send(Message::Transfer(serial, Update::Progress(done, total)));
                },
            )
            .map(|_| list)
            .map_err(|e| e.to_string());
            let _ = events.send(Message::Transfer(serial, Update::Resolved(result)));
        });
        Ok(())
    }
    pub(super) fn transfer_update(&mut self, serial: u64, update: Update) -> Result<()> {
        if serial != self.transfer_serial {
            return Ok(());
        }
        let Some(Popup::Transfer(mut dialog)) = self.popup.clone() else {
            return Ok(());
        };
        match update {
            Update::Parsed(Ok(lists)) => {
                if lists.len() == 1 || !dialog.list.trim().is_empty() {
                    match playlists::select(
                        lists,
                        (!dialog.list.trim().is_empty()).then_some(dialog.list.trim()),
                        None,
                    ) {
                        Ok(list) => return self.transfer_resolve(dialog, list),
                        Err(error) => {
                            dialog.error = error.to_string();
                            dialog.stage = Stage::Input;
                        }
                    }
                } else {
                    dialog.selected = 0;
                    dialog.stage = Stage::Choosing(Arc::new(lists));
                }
            }
            Update::Resolved(Ok(list)) => {
                self.transfer_cancel = None;
                dialog.stage = Stage::Preview(Arc::new(list));
            }
            Update::Progress(done, total) => {
                dialog.stage = Stage::Working(format!(
                    "已处理 {done}/{total}；Esc 取消，播放器仍可响应老板键"
                ));
            }
            Update::Parsed(Err(error)) | Update::Resolved(Err(error)) => {
                self.transfer_cancel = None;
                dialog.error = error;
                dialog.stage = Stage::Input;
            }
        }
        self.popup = Some(Popup::Transfer(dialog));
        Ok(())
    }
    pub(super) fn transfer_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(Popup::Transfer(dialog)) = &mut self.popup else {
            return Ok(());
        };
        if key.code == KeyCode::Esc {
            self.close_transfer();
            return Ok(());
        }
        match &dialog.stage {
            Stage::Working(_) => {}
            Stage::Preview(_) | Stage::Review(_) => {
                if key.code == KeyCode::Enter {
                    self.transfer_go()?;
                }
            }
            Stage::Choosing(lists) => match key.code {
                KeyCode::Up => dialog.selected = dialog.selected.saturating_sub(1),
                KeyCode::Down => {
                    dialog.selected = (dialog.selected + 1).min(lists.len().saturating_sub(1))
                }
                KeyCode::Enter => self.transfer_go()?,
                _ => {}
            },
            Stage::Input => {
                let fields = if dialog.export.is_some() { 3 } else { 6 };
                match key.code {
                    KeyCode::Tab | KeyCode::Down => dialog.focus = (dialog.focus + 1) % fields,
                    KeyCode::BackTab | KeyCode::Up => {
                        dialog.focus = (dialog.focus + fields - 1) % fields
                    }
                    KeyCode::Char(' ') if dialog.focus == 3 => {
                        dialog.title_first = !dialog.title_first
                    }
                    KeyCode::Char(' ') if dialog.focus == 4 => dialog.offline = !dialog.offline,
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        match dialog.focus {
                            0 => dialog.path.push(c),
                            1 => dialog.name.push(c),
                            2 if dialog.export.is_none() => dialog.list.push(c),
                            _ => {}
                        }
                    }
                    KeyCode::Backspace => {
                        let value = match dialog.focus {
                            0 => Some(&mut dialog.path),
                            1 => Some(&mut dialog.name),
                            2 if dialog.export.is_none() => Some(&mut dialog.list),
                            _ => None,
                        };
                        if let Some(value) = value {
                            value.pop();
                        }
                    }
                    KeyCode::Enter => {
                        if dialog.focus == 3 {
                            dialog.title_first = !dialog.title_first;
                        } else if dialog.focus == 4 {
                            dialog.offline = !dialog.offline;
                        } else {
                            self.transfer_go()?;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
    pub(super) fn draw_transfer(&mut self, frame: &mut Frame, body: Rect, dialog: Dialog) {
        let mut lines = vec![];
        match &dialog.stage {
            Stage::Input => {
                let labels = if dialog.export.is_some() {
                    vec![
                        "保存路径（不覆盖已有文件）",
                        "格式：json / csv / tsv / txt / m3u8 / jspf",
                    ]
                } else {
                    vec![
                        "文件路径（支持直接粘贴）",
                        "新歌单名（留空沿用原名称）",
                        "多歌单选择（留空后选择 / 名称 / 序号）",
                    ]
                };
                for (i, label) in labels.iter().enumerate() {
                    lines.push(Line::styled(*label, Style::default().fg(DIM)));
                    let value = match i {
                        0 => &dialog.path,
                        1 => &dialog.name,
                        _ => &dialog.list,
                    };
                    lines.push(Line::styled(
                        format!("{value}{}", if dialog.focus == i as u8 { "▏" } else { "" }),
                        Style::default().fg(FG).bg(if dialog.focus == i as u8 {
                            SELECTED
                        } else {
                            BG
                        }),
                    ));
                    self.hit(
                        Rect::new(body.x, body.y + i as u16 * 2 + 1, body.width, 1),
                        Action::TransferField(i as u8),
                    );
                }
                if dialog.export.is_none() {
                    lines.push(Line::styled(
                        format!(
                            "{} TXT：{}",
                            if dialog.focus == 3 { "›" } else { " " },
                            if dialog.title_first {
                                "歌名 - 歌手"
                            } else {
                                "歌手 - 歌名"
                            }
                        ),
                        Style::default().fg(ACCENT),
                    ));
                    self.hit(
                        Rect::new(body.x, body.y + 6, body.width, 1),
                        Action::TransferField(3),
                    );
                    lines.push(Line::styled(
                        format!(
                            "{} 离线匹配：{}",
                            if dialog.focus == 4 { "›" } else { " " },
                            if dialog.offline { "开" } else { "关" }
                        ),
                        Style::default().fg(ACCENT),
                    ));
                    self.hit(
                        Rect::new(body.x, body.y + 7, body.width, 1),
                        Action::TransferField(4),
                    );
                    lines.push(Line::styled(
                        "LX v2 / MusicFree / CSV / TXT / JSON / M3U / PLS / JSPF",
                        Style::default().fg(DIM),
                    ));
                }
                lines.push(Line::styled(
                    "Tab 切换 · 空格切换选项 · Enter 继续 · Esc 取消",
                    Style::default().fg(DIM),
                ));
            }
            Stage::Working(text) => {
                lines.push(Line::styled(text.clone(), Style::default().fg(ACCENT)));
                lines.push(Line::raw(
                    "等待当前网络请求结束前即可取消；取消后不会保存半张歌单。",
                ));
            }
            Stage::Choosing(lists) => {
                lines.push(Line::raw("↑/↓ 选择需要导入的歌单，Enter 继续"));
                let start = dialog
                    .selected
                    .saturating_sub(body.height.saturating_sub(5) as usize);
                for (i, list) in lists
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(body.height.saturating_sub(4) as usize)
                {
                    lines.push(Line::styled(
                        format!(
                            "{} {} · {} 条",
                            if i == dialog.selected { "›" } else { " " },
                            list.name,
                            list.entries.len()
                        ),
                        Style::default().fg(if i == dialog.selected { ACCENT } else { FG }),
                    ));
                    self.hit(
                        Rect::new(body.x, body.y + lines.len() as u16 - 1, body.width, 1),
                        Action::TransferChoose(i),
                    );
                }
            }
            Stage::Preview(list) | Stage::Review(list) => {
                let s = list.summary();
                lines.push(Line::styled(
                    format!("{}  ·  {}  ·  {} 条", list.name, list.format, s.total),
                    Style::default().fg(ACCENT),
                ));
                lines.push(Line::raw(format!(
                    "可播放 {}  未匹配 {}  待确认 {}",
                    s.matched, s.pending, s.ambiguous
                )));
                lines.push(Line::raw(format!(
                    "无效 {}  重复 {}（播放时去重）",
                    s.invalid, s.duplicates
                )));
                lines.push(Line::raw(""));
                for (i, e) in list
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.status != playlists::MatchState::Matched && !e.duplicate)
                    .take(5)
                {
                    lines.push(Line::styled(
                        format!("#{} {} · {}", i + 1, e.title, e.artist),
                        Style::default().fg(DIM),
                    ));
                }
                lines.push(Line::raw(
                    "未匹配曲目会保留，可用 playlist inspect / resolve / match 修复。",
                ));
                lines.push(Line::raw(if dialog.existing {
                    "更新前保存快照；确认后才会写入歌单。"
                } else {
                    "同名时自动另存；确认后才会写入歌单。"
                }));
            }
        }
        if !dialog.error.is_empty() {
            lines.push(Line::styled(dialog.error.clone(), Style::default().fg(BAD)));
        }
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().fg(FG)),
            Rect::new(body.x, body.y, body.width, body.height.saturating_sub(2)),
        );
        let label = match dialog.stage {
            Stage::Input if dialog.export.is_some() => "导出 ↵",
            Stage::Input => "解析与匹配 ↵",
            Stage::Preview(_) if dialog.existing => "确认更新 ↵",
            Stage::Preview(_) => "确认导入 ↵",
            Stage::Review(_) => "重试匹配 ↵",
            Stage::Choosing(_) => "选择歌单 ↵",
            Stage::Working(_) => "",
        };
        if !label.is_empty() {
            self.button(
                frame,
                Rect::new(
                    body.x,
                    body.bottom().saturating_sub(1),
                    18.min(body.width),
                    1,
                ),
                label,
                Action::TransferGo,
                true,
            );
        }
    }
}
