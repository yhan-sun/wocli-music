use super::*;
use std::{io::Write, path::Path};
/// Write beside the destination and atomically replace only when explicitly allowed.
pub fn write(list: &Playlist, format: &str, path: &Path, force: bool) -> Result<()> {
    list.validate()?;
    let bytes = match format {
        "json" => serde_json::to_vec_pretty(
            &serde_json::json!({"format":"wocli-playlist","version":1,"playlists":[list]}),
        )?,
        "csv" | "tsv" => {
            let mut writer = csv::WriterBuilder::new()
                .delimiter(if format == "tsv" { b'\t' } else { b',' })
                .from_writer(Vec::new());
            let err = |e: csv::Error| Error::new(e.to_string());
            writer
                .write_record([
                    "title", "artist", "album", "duration", "platform", "trackId", "isrc", "url",
                    "path",
                ])
                .map_err(err)?;
            for e in &list.entries {
                writer
                    .write_record([
                        &e.title,
                        &e.artist,
                        &e.album,
                        &e.duration.to_string(),
                        &e.source,
                        &e.id,
                        &e.isrc,
                        &e.url,
                        &e.path,
                    ])
                    .map_err(err)?;
            }
            let raw = writer.into_inner().map_err(|e| Error::new(e.to_string()))?;
            // BOM allows Windows Excel to recognise Chinese without choosing an encoding.
            [b"\xef\xbb\xbf".as_slice(), &raw].concat()
        }
        "txt" => list
            .entries
            .iter()
            .map(|e| {
                if e.artist.is_empty() {
                    e.title.clone()
                } else {
                    format!("{} - {}", e.artist, e.title)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes(),
        "m3u" | "m3u8" => {
            let mut text = String::from("#EXTM3U\n");
            for e in &list.entries {
                let path = e
                    .resolved
                    .as_ref()
                    .map(|t| t.path.as_str())
                    .filter(|p| !p.is_empty())
                    .unwrap_or(&e.path);
                if path.is_empty() {
                    return Err(Error::new(
                        "M3U8 导出需要每首歌都有本地文件路径；在线歌曲元数据请用 JSON/CSV 导出",
                    ));
                }
                if path.contains(['\n', '\r'])
                    || e.title.contains(['\n', '\r'])
                    || e.artist.contains(['\n', '\r'])
                {
                    return Err(Error::new("M3U8 路径与标题不能包含换行"));
                }
                text.push_str(&format!(
                    "#EXTINF:{},{} - {}\n{}\n",
                    e.duration as i64, e.artist, e.title, path
                ));
            }
            text.into_bytes()
        }
        "jspf" => serde_json::to_vec_pretty(
            &serde_json::json!({"playlist":{"title":list.name,"track":list.entries.iter().map(|e|serde_json::json!({"title":e.title,"creator":e.artist,"album":e.album,"duration":(e.duration*1000.0) as u64,"location":if e.url.is_empty(){vec![]}else{vec![e.url.clone()]}})).collect::<Vec<_>>()}}),
        )?,
        _ => {
            return Err(Error::new(
                "--format 支持 json / csv / tsv / txt / m3u8 / jspf",
            ))
        }
    };
    if bytes.len() > MAX_BYTES {
        return Err(Error::new("导出文件超过 16 MiB，请拆分歌单"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    if force {
        temp.persist(path)
            .map_err(|e| Error::new(e.error.to_string()))?;
    } else {
        temp.persist_noclobber(path).map_err(|e| {
            Error::new(format!("无法保存（覆盖已有文件请加 --force）：{}", e.error))
        })?;
    }
    Ok(())
}
