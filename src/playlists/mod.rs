//! Portable playlist metadata. Foreign IDs never enter the playback queue directly.
mod export;
mod parse;
mod resolve;
mod storage;

pub use export::write;
pub use parse::read;
pub use resolve::{resolve, ResolveOptions};
pub use storage::{backups, restore, save, stored, Conflict, SaveReport};

use crate::model::{Error, Result, Track};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use unicode_normalization::UnicodeNormalization;

pub const MAX_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 10_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchState {
    Matched,
    #[default]
    Pending,
    Ambiguous,
    Invalid,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub source: String,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Seconds, regardless of the source format's unit.
    pub duration: f64,
    pub isrc: String,
    pub url: String,
    pub path: String,
    pub status: MatchState,
    pub reason: String,
    pub candidates: Vec<Track>,
    pub resolved: Option<Track>,
    pub duplicate: bool,
    pub manual: bool,
}
impl Entry {
    pub fn from_track(track: Track) -> Self {
        Self {
            source: if track.path.is_empty() { "kw" } else { "local" }.into(),
            id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration: track.duration,
            path: track.path.clone(),
            status: MatchState::Matched,
            resolved: Some(track),
            ..Self::default()
        }
    }
    pub fn key(&self) -> String {
        if let Some(track) = &self.resolved {
            return format!("target:{}", track.id);
        }
        if !self.path.is_empty() {
            return format!("path:{}", self.path);
        }
        if !self.source.is_empty() && !self.id.is_empty() {
            return format!("origin:{}:{}", self.source, self.id);
        }
        // Keep case/accents and punctuation for deduplication: fuzzy matching is not identity.
        format!(
            "meta:{}\0{}\0{}\0{}",
            self.title.trim(),
            self.artist.trim(),
            self.album.trim(),
            self.duration
        )
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Playlist {
    pub name: String,
    pub format: String,
    pub entries: Vec<Entry>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Summary {
    pub total: usize,
    pub matched: usize,
    pub pending: usize,
    pub ambiguous: usize,
    pub invalid: usize,
    pub duplicates: usize,
}
impl Playlist {
    pub fn summary(&self) -> Summary {
        let mut out = Summary {
            total: self.entries.len(),
            ..Summary::default()
        };
        for entry in &self.entries {
            if entry.duplicate {
                out.duplicates += 1;
                continue;
            }
            match entry.status {
                MatchState::Matched => out.matched += 1,
                MatchState::Pending => out.pending += 1,
                MatchState::Ambiguous => out.ambiguous += 1,
                MatchState::Invalid => out.invalid += 1,
            }
        }
        out
    }
    pub fn deduplicate(&mut self) {
        let mut seen = HashSet::new();
        for entry in &mut self.entries {
            entry.duplicate = !seen.insert(entry.key());
        }
    }
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        if self.entries.len() > MAX_ENTRIES {
            return Err(Error::new("单张歌单最多导入 10000 条"));
        }
        for entry in &self.entries {
            if !entry.duration.is_finite() || entry.duration < 0.0 {
                return Err(Error::new("歌单时长无效"));
            }
        }
        Ok(())
    }
}
pub fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(Error::new("歌单名称应为 1–100 字，不能包含控制字符"));
    }
    Ok(())
}
pub fn select(
    mut lists: Vec<Playlist>,
    selector: Option<&str>,
    name: Option<&str>,
) -> Result<Playlist> {
    let index = if let Some(selector) = selector {
        lists
            .iter()
            .position(|p| p.name == selector)
            .or_else(|| {
                selector
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .filter(|&i| i < lists.len())
            })
            .ok_or_else(|| Error::new("找不到指定歌单；用 playlist scan 文件 查看名称和序号"))?
    } else if lists.len() == 1 {
        0
    } else {
        let mut error =
            Error::new("文件包含多张歌单，请使用 --list 名称或序号选择（playlist scan 查看）");
        error.details = lists
            .iter()
            .enumerate()
            .map(|(i, p)| format!("{}: {} ({} 条)", i + 1, p.name, p.entries.len()))
            .collect();
        return Err(error);
    };
    let mut list = lists.remove(index);
    if let Some(name) = name.filter(|v| !v.trim().is_empty()) {
        list.name = name.trim().into();
    }
    list.validate()?;
    Ok(list)
}
pub(super) fn normalized(text: &str) -> String {
    text.nfkc()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}
