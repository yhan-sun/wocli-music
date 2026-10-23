use super::*;
use flate2::read::GzDecoder;
use serde_json::Value;
use std::{fs::File, io::Read, path::Path};

fn bounded(reader: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::new("歌单文件或解压内容超过 16 MiB"));
    }
    Ok(bytes)
}
fn string(value: &Value) -> String {
    match value {
        Value::String(v) => v.clone(),
        Value::Number(v) => v.to_string(),
        _ => String::new(),
    }
}
fn get(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .map(|k| string(&value[*k]))
        .find(|s| !s.is_empty())
        .unwrap_or_default()
}
fn duration(value: &str) -> f64 {
    if value.contains(':') {
        let parts: Option<Vec<f64>> = value.split(':').map(|p| p.trim().parse().ok()).collect();
        parts
            .map(|p| p.iter().fold(0.0, |sum, v| sum * 60.0 + v))
            .unwrap_or(0.0)
    } else {
        value.parse::<f64>().unwrap_or(0.0).max(0.0)
    }
}
fn source(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "kuwo" | "酷我" | "酷我音乐" | "kw" => "kw".into(),
        "netease" | "网易" | "网易云" | "网易云音乐" | "wy" => "wy".into(),
        "qq" | "qq音乐" | "tx" => "tx".into(),
        _ => value.trim().into(),
    }
}
fn record(value: &Value, lx: bool) -> Entry {
    let meta = &value["meta"];
    Entry {
        source: source(&get(value, &["source", "platform"])),
        id: if lx {
            get(meta, &["songId"])
        } else {
            get(value, &["id", "trackId"])
        },
        title: get(value, &["title", "name"]),
        artist: get(value, &["artist", "singer"]),
        album: if lx {
            get(meta, &["albumName"])
        } else {
            get(value, &["album"])
        },
        duration: duration(&get(value, &["duration", "interval"])),
        path: get(meta, &["filePath"]),
        url: get(value, &["url"]),
        ..Entry::default()
    }
}
fn list(value: &Value, name: &str, key: &str, lx: bool, format: &str) -> Result<Playlist> {
    let entries = value[key]
        .as_array()
        .ok_or_else(|| Error::new(format!("{format} 缺少 {key} 数组")))?;
    check_count(entries.len())?;
    Ok(Playlist {
        name: get(value, &["name", "title"]).trim().to_string(),
        format: format.into(),
        entries: entries.iter().map(|v| record(v, lx)).collect(),
    }
    .with_name(name))
}
impl Playlist {
    fn with_name(mut self, fallback: &str) -> Self {
        if self.name.is_empty() {
            self.name = fallback.chars().take(100).collect();
        }
        self
    }
}
fn check_count(count: usize) -> Result<()> {
    if count > MAX_ENTRIES {
        return Err(Error::new("一份文件最多 10000 条歌曲"));
    }
    Ok(())
}
fn check_json_lists(lists: &[Value], key: &str) -> Result<()> {
    if lists.len() > 1000 {
        return Err(Error::new("一份文件最多 1000 张歌单"));
    }
    let count = lists
        .iter()
        .map(|v| v[key].as_array().map_or(0, Vec::len))
        .sum();
    check_count(count)
}
fn json(raw: &str, name: &str) -> Result<Vec<Playlist>> {
    let mut value: Value = serde_json::from_str(raw)?;
    if let Value::String(inner) = &value {
        value = serde_json::from_str(inner)?;
    }
    if value["format"] == "wocli-playlist" {
        if value["version"] != 1 {
            return Err(Error::new("不支持的 wocli 歌单版本"));
        }
        let raw_lists = value["playlists"]
            .as_array()
            .ok_or_else(|| Error::new("缺少 playlists 数组"))?;
        check_json_lists(raw_lists, "entries")?;
        for list in raw_lists {
            for entry in list["entries"].as_array().into_iter().flatten() {
                if entry["candidates"].as_array().is_some_and(|v| v.len() > 20) {
                    return Err(Error::new("单条歌曲最多保留 20 个匹配候选"));
                }
            }
        }
        let lists: Vec<Playlist> = serde_json::from_value(value["playlists"].clone())?;
        return Ok(lists);
    }
    if let Some(kind) = value["type"].as_str() {
        return match kind {
            "playListPart_v2" => Ok(vec![list(&value["data"], name, "list", true, "lx-v2")?]),
            _ => Err(Error::new(format!(
                "暂不支持 LX 备份类型 {kind}；请从 LX 导出单张 v2 歌单"
            ))),
        };
    }
    if let Some(sheets) = value["musicSheets"].as_array() {
        check_json_lists(sheets, "musicList")?;
        return sheets
            .iter()
            .map(|v| list(v, name, "musicList", false, "musicfree"))
            .collect();
    }
    if let Some(playlist) = value["playlist"].as_object() {
        let tracks = playlist
            .get("track")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new("JSPF 缺少 track 数组"))?;
        check_count(tracks.len())?;
        return Ok(vec![Playlist {
            name: string(&value["playlist"]["title"]),
            format: "jspf".into(),
            entries: tracks
                .iter()
                .map(|t| Entry {
                    title: get(t, &["title"]),
                    artist: get(t, &["creator"]),
                    album: get(t, &["album"]),
                    duration: duration(&get(t, &["duration"])) / 1000.0,
                    url: t["location"]
                        .as_array()
                        .and_then(|v| v.first())
                        .map(string)
                        .unwrap_or_default(),
                    ..Entry::default()
                })
                .collect(),
        }
        .with_name(name)]);
    }
    Err(Error::new(
        "无法识别 JSON 歌单；支持 wocli v1、LX 单歌单 v2、MusicFree 备份和 JSPF",
    ))
}
fn csv(raw: &str, name: &str, delimiter: u8) -> Result<Vec<Playlist>> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .trim(csv::Trim::All)
        .from_reader(raw.as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| Error::new(format!("CSV 表头错误：{e}")))?
        .iter()
        .map(normalized)
        .collect::<Vec<_>>();
    let col = |keys: &[&str]| headers.iter().position(|h| keys.iter().any(|k| h == k));
    let title = col(&[
        "title",
        "trackname",
        "songname",
        "name",
        "歌名",
        "歌曲名",
        "歌曲名称",
        "标题",
    ])
    .ok_or_else(|| Error::new("CSV 缺少歌名表头（title / 歌曲名）"))?;
    let artist = col(&[
        "artist",
        "artists",
        "artistname",
        "歌手",
        "歌手名",
        "艺术家",
    ]);
    let album = col(&["album", "albumname", "专辑", "专辑名"]);
    let src = col(&["platform", "source", "平台", "来源"]);
    let id = col(&["trackid", "songid", "id", "歌曲id"]);
    let time = col(&["duration", "durationseconds", "时长", "时长秒"]);
    let ms = col(&["durationms", "durationmilliseconds", "时长毫秒"]);
    let isrc = col(&["isrc"]);
    let url = col(&["url", "网址", "链接"]);
    let path = col(&["path", "filepath", "路径"]);
    let mut entries = vec![];
    for row in reader.records() {
        let row = row.map_err(|e| Error::new(format!("CSV 行解析失败：{e}")))?;
        let field = |index: Option<usize>| index.and_then(|i| row.get(i)).unwrap_or("").to_string();
        entries.push(Entry {
            title: field(Some(title)),
            artist: field(artist),
            album: field(album),
            source: source(&field(src)),
            id: field(id),
            duration: if ms.is_some() {
                duration(&field(ms)) / 1000.0
            } else {
                duration(&field(time))
            },
            isrc: field(isrc),
            url: field(url),
            path: field(path),
            ..Entry::default()
        });
        if entries.len() > MAX_ENTRIES {
            return Err(Error::new("单张歌单最多导入 10000 条"));
        }
    }
    Ok(vec![Playlist {
        name: name.into(),
        format: "csv".into(),
        entries,
    }])
}
fn text_entry(line: &str, title_first: bool) -> Entry {
    let pair = line.split_once(" - ");
    let (title, artist) = match pair {
        Some((a, b)) if title_first => (a, b),
        Some((a, b)) => (b, a),
        None => (line, ""),
    };
    Entry {
        title: title.trim().into(),
        artist: artist.trim().into(),
        ..Entry::default()
    }
}
fn m3u(raw: &str, name: &str) -> Result<Vec<Playlist>> {
    if raw.lines().any(|l| l.trim_start().starts_with("#EXT-X-")) {
        return Err(Error::new("这是 HLS 分片清单，不是歌曲歌单"));
    }
    let mut entries = vec![];
    let mut info = Entry::default();
    for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(ext) = line.strip_prefix("#EXTINF:") {
            if let Some((d, title)) = ext.split_once(',') {
                info = text_entry(title, false);
                info.duration = duration(d);
            }
        } else if !line.starts_with('#') {
            info.url = line.into();
            check_count(entries.len() + 1)?;
            entries.push(std::mem::take(&mut info));
        }
    }
    Ok(vec![Playlist {
        name: name.into(),
        format: "m3u8".into(),
        entries,
    }])
}
fn pls(raw: &str, name: &str) -> Result<Vec<Playlist>> {
    let mut entries = std::collections::BTreeMap::<usize, Entry>::new();
    for line in raw.lines() {
        let Some((key, val)) = line.trim().split_once('=') else {
            continue;
        };
        let key = key.to_lowercase();
        for prefix in ["file", "title", "length"] {
            if let Some(n) = key
                .strip_prefix(prefix)
                .and_then(|v| v.parse::<usize>().ok())
            {
                if !entries.contains_key(&n) {
                    check_count(entries.len() + 1)?;
                }
                let entry = entries.entry(n).or_default();
                match prefix {
                    "file" => entry.url = val.into(),
                    "title" => {
                        let parsed = text_entry(val, false);
                        entry.title = parsed.title;
                        entry.artist = parsed.artist;
                    }
                    _ => entry.duration = duration(val),
                }
            }
        }
    }
    Ok(vec![Playlist {
        name: name.into(),
        format: "pls".into(),
        entries: entries.into_values().collect(),
    }])
}
fn local_path(value: &str, base: &Path) -> Option<String> {
    if value.is_empty() || (value.contains("://") && !value.starts_with("file://")) {
        return None;
    }
    let value = if let Some(uri) = value.strip_prefix("file://") {
        // URI decoding is byte based so non-ASCII paths roundtrip too.
        let uri = uri
            .strip_prefix("localhost/")
            .map(|v| format!("/{v}"))
            .unwrap_or_else(|| uri.into());
        if !uri.starts_with('/') {
            return None;
        }
        let mut out = Vec::new();
        let bytes = uri.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                if let (Some(a), Some(b)) = (
                    (bytes[i + 1] as char).to_digit(16),
                    (bytes[i + 2] as char).to_digit(16),
                ) {
                    let n = (a * 16 + b) as u8;
                    out.push(n);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8(out).ok()?
    } else {
        value.into()
    };
    #[cfg(windows)]
    let value = if value.as_bytes().get(2) == Some(&b':') && value.starts_with('/') {
        value[1..].to_string()
    } else {
        value
    };
    let path = Path::new(&value);
    Some(
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        }
        .to_string_lossy()
        .into(),
    )
}
pub fn read(path: &Path, text_order: &str) -> Result<Vec<Playlist>> {
    if !["artist-title", "title-artist"].contains(&text_order) {
        return Err(Error::new("--text-order 应为 artist-title 或 title-artist"));
    }
    let bytes = bounded(File::open(path)?)?;
    let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
        bounded(GzDecoder::new(bytes.as_slice()))?
    } else {
        bytes
    };
    let raw = std::str::from_utf8(&bytes)
        .map_err(|_| Error::new("歌单须为 UTF-8 编码；请另存为 UTF-8 CSV/TXT"))?
        .trim_start_matches('\u{feff}')
        .trim();
    let name = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let mut lists = if ext == "csv" || ext == "tsv" {
        csv(raw, &name, if ext == "tsv" { b'\t' } else { b',' })?
    } else if raw.starts_with("#EXTM3U") || ["m3u", "m3u8"].contains(&ext.as_str()) {
        m3u(raw, &name)?
    } else if raw.starts_with("[playlist]") || ext == "pls" {
        pls(raw, &name)?
    } else if ext == "txt" {
        check_count(raw.lines().filter(|l| !l.trim().is_empty()).count())?;
        vec![Playlist {
            name: name.into(),
            format: "txt".into(),
            entries: raw
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|l| text_entry(l, text_order == "title-artist"))
                .collect(),
        }]
    } else if raw.starts_with('{') || raw.starts_with('[') || raw.starts_with('"') {
        json(raw, &name)?
    } else {
        return Err(Error::new(
            "不支持的歌单格式；请导出 CSV/TXT、LX 单歌单 v2、MusicFree JSON、M3U8/PLS 或 JSPF",
        ));
    };
    if lists.is_empty() {
        return Err(Error::new("文件没有歌单"));
    }
    let total: usize = lists.iter().map(|p| p.entries.len()).sum();
    if total > MAX_ENTRIES || lists.len() > 1000 {
        return Err(Error::new("一份文件最多 10000 条歌曲、1000 张歌单"));
    }
    let base = path
        .canonicalize()?
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    for list in &mut lists {
        list.validate()?;
        for e in &mut list.entries {
            if let Some(path) = local_path(if !e.path.is_empty() { &e.path } else { &e.url }, &base)
            {
                e.path = path;
            }
            if e.title.is_empty() && !e.path.is_empty() {
                e.title = Path::new(&e.path)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into();
            }
            if e.title.trim().is_empty() {
                e.status = MatchState::Invalid;
                e.reason = "缺少歌名".into();
                e.resolved = None;
            }
        }
        list.deduplicate();
    }
    Ok(lists)
}
