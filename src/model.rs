use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, path::Path};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Error {
    pub message: String,
    pub code: String,
    #[serde(default)]
    pub details: Vec<String>,
}
impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: "error".into(),
            details: vec![],
        }
    }
    pub fn coded(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            ..Self::new(message)
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::new(error.to_string())
    }
}
impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(error.to_string())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub formats: String,
    pub path: String,
}
impl Track {
    pub fn local(path: &Path) -> Result<Self> {
        let path = path.canonicalize()?;
        if !path.is_file() {
            return Err(Error::new("找不到本地音频文件"));
        }
        let value = path.to_string_lossy().to_string();
        Ok(Self {
            id: format!(
                "local:{:.20}",
                format!("{:x}", Sha256::digest(value.as_bytes()))
            ),
            title: clean(&path.file_stem().unwrap_or_default().to_string_lossy()),
            artist: "本地音乐".into(),
            path: value,
            ..Self::default()
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub quality: String,
    pub volume: u8,
    pub mode: String,
    pub direct: bool,
    pub lyric_offset: i64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: "320k".into(),
            volume: 60,
            mode: "list".into(),
            direct: false,
            lyric_offset: 0,
        }
    }
}
pub const MODES: [&str; 4] = ["list", "repeat", "single", "shuffle"];
pub const QUALITIES: [&str; 4] = ["best", "flac", "320k", "128k"];
pub fn mode_label(mode: &str) -> &'static str {
    match mode {
        "repeat" => "列表循环",
        "single" => "单曲循环",
        "shuffle" => "随机播放",
        _ => "顺序播放",
    }
}
pub fn clock(seconds: f64) -> String {
    let s = if seconds.is_finite() {
        seconds.max(0.) as u64
    } else {
        0
    };
    format!("{:02}:{:02}", s / 60, s % 60)
}
pub fn normalize_id(value: &str) -> Result<String> {
    let raw = value
        .trim()
        .strip_prefix("kuwo:")
        .or_else(|| value.trim().strip_prefix("MUSIC_"))
        .unwrap_or(value.trim());
    if raw.is_empty() || !raw.bytes().all(|c| c.is_ascii_digit()) {
        return Err(Error::new("歌曲 ID 应为数字"));
    }
    let value: u64 = raw.parse().map_err(|_| Error::new("歌曲 ID 无效"))?;
    Ok(format!("kuwo:{value}"))
}
pub fn clean(value: &str) -> String {
    let mut normalized = value.to_string();
    for _ in 0..3 {
        normalized = normalized.replace("\\\\u", "\\u");
        for (from, to) in [
            ("\\u0026", "&"),
            ("\\u003c", "<"),
            ("\\u003e", ">"),
            ("\\u0027", "'"),
            ("\\u0022", "\""),
        ] {
            normalized = normalized.replace(from, to);
        }
    }
    let value = normalized.as_str();
    let mut output = String::new();
    let mut tag = false;
    let mut escape = false;
    for c in value.chars() {
        if c == '\u{1b}' {
            escape = true;
            continue;
        }
        if escape {
            if c.is_ascii_alphabetic() {
                escape = false;
            }
            continue;
        }
        if c == '<' {
            tag = true;
            continue;
        }
        if c == '>' && tag {
            tag = false;
            continue;
        }
        if !tag {
            output.push(if c.is_control() { ' ' } else { c });
        }
    }
    for (from, to) in [
        ("&amp;", "&"),
        ("&nbsp;", " "),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&lt;", "<"),
        ("&gt;", ">"),
    ] {
        output = output.replace(from, to);
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}
