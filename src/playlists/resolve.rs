use super::*;
use crate::{model::normalize_id, provider::Provider, store::Paths};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

fn artists(value: &str) -> Vec<String> {
    let mut values = value
        .split(['&', '/', '、', ';', '；', ','])
        .map(normalized)
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}
fn compatible(entry: &Entry, track: &Track) -> bool {
    // Retain version qualifiers (Live, instrumental, remaster, cover) in title comparison.
    !normalized(&entry.title).is_empty()
        && normalized(&entry.title) == normalized(&track.title)
        && !artists(&entry.artist).is_empty()
        && artists(&entry.artist) == artists(&track.artist)
        && (entry.duration <= 0.0
            || track.duration <= 0.0
            || (entry.duration - track.duration).abs() <= 5.0)
}
fn candidates(entry: &mut Entry, tracks: Vec<Track>) {
    let mut seen = HashSet::new();
    let mut exact = tracks
        .iter()
        .filter(|t| compatible(entry, t) && seen.insert(t.id.clone()))
        .cloned()
        .collect::<Vec<_>>();
    if exact.len() > 1 && !entry.album.is_empty() {
        let album = exact
            .iter()
            .filter(|t| normalized(&t.album) == normalized(&entry.album))
            .cloned()
            .collect::<Vec<_>>();
        if album.len() == 1 {
            exact = album;
        }
    }
    if exact.len() == 1 {
        entry.resolved = exact.pop();
        entry.status = MatchState::Matched;
        entry.reason = "歌名、歌手及版本信息一致".into();
        entry.candidates.clear();
    } else {
        entry.status = if exact.is_empty() {
            MatchState::Pending
        } else {
            MatchState::Ambiguous
        };
        entry.reason = if exact.is_empty() {
            "未找到唯一且信息一致的可播放版本"
        } else {
            "有多个匹配版本，请使用 playlist match 手动选择"
        }
        .into();
        entry.candidates = if exact.is_empty() {
            tracks.into_iter().take(5).collect()
        } else {
            exact.into_iter().take(5).collect()
        };
    }
}
fn local(entry: &mut Entry) {
    match Track::local(std::path::Path::new(&entry.path)) {
        Ok(mut track) => {
            track.title = entry.title.clone();
            if !entry.artist.is_empty() {
                track.artist = entry.artist.clone();
            }
            track.album = entry.album.clone();
            track.duration = entry.duration;
            entry.resolved = Some(track);
            entry.status = MatchState::Matched;
            entry.reason = "本地文件存在".into();
        }
        Err(_) => {
            entry.resolved = None;
            entry.status = MatchState::Pending;
            entry.reason = "本地文件不存在，请修复路径后重新导入".into();
        }
    }
}
fn resolve_one(mut entry: Entry, known: &[Track], provider: Option<&Provider>) -> Entry {
    if entry.status == MatchState::Invalid {
        return entry;
    }
    if !entry.path.is_empty() {
        local(&mut entry);
        return entry;
    }
    // Native backups carry explicit wocli/Kuwo IDs, but reject arbitrary injected provider IDs.
    if let Some(track) = entry.resolved.as_ref() {
        if track.path.is_empty() && normalize_id(&track.id).is_ok() {
            entry.status = MatchState::Matched;
            return entry;
        }
    }
    entry.resolved = None;
    entry.candidates.clear();
    if entry.source == "kw" && normalize_id(&entry.id).is_ok() {
        let direct = known
            .iter()
            .find(|t| t.id == entry.id)
            .cloned()
            .or_else(|| provider.and_then(|p| p.details(&entry.id).ok()));
        if let Some(track) = direct {
            if compatible(&entry, &track) {
                entry.resolved = Some(track);
                entry.status = MatchState::Matched;
                entry.reason = "已核对酷我 ID 与曲目信息".into();
                return entry;
            }
        }
    }
    let cached = known
        .iter()
        .filter(|t| compatible(&entry, t))
        .cloned()
        .collect();
    candidates(&mut entry, cached);
    if entry.status == MatchState::Matched {
        return entry;
    }
    if let Some(provider) = provider {
        if entry.artist.trim().is_empty() {
            entry.reason = "缺少歌手，无法可靠自动匹配；请手动选择歌曲".into();
            return entry;
        }
        match provider.search(&format!("{} {}", entry.title, entry.artist), 0, 20) {
            Ok((_, tracks)) => candidates(&mut entry, tracks),
            Err(error) => {
                entry.status = MatchState::Pending;
                entry.reason = format!("搜索失败，可稍后重试：{error}");
            }
        }
    } else if entry.status == MatchState::Pending {
        entry.reason = "离线导入：未在本地曲库找到唯一匹配；可稍后 playlist resolve".into();
    }
    entry
}
/// All network work finishes before a caller starts its database transaction.
/// At most four requests run at once; cancellation prevents any subsequent commit.
pub fn resolve(
    list: &mut Playlist,
    known: &[Track],
    paths: &Paths,
    offline: bool,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize) + Sync,
) -> Result<()> {
    list.validate()?;
    let settings = paths.settings()?;
    let input = &list.entries;
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results = Mutex::new(vec![None; input.len()]);
    // Resolve identical source entries once, preserving their original positions.
    let mut unique = HashMap::new();
    let mut indices = Vec::with_capacity(input.len());
    let mut jobs = Vec::new();
    for (i, entry) in input.iter().enumerate() {
        let n = *unique.entry(entry.key()).or_insert_with(|| {
            jobs.push(i);
            jobs.len() - 1
        });
        indices.push(n);
    }
    std::thread::scope(|scope| {
        for _ in 0..4.min(jobs.len()) {
            let (jobs, next, done, results, progress) = (&jobs, &next, &done, &results, &progress);
            scope.spawn(move || {
                let provider =
                    (!offline).then(|| Provider::with_sources(settings.direct, paths.home.clone()));
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let n = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&i) = jobs.get(n) else {
                        break;
                    };
                    let entry = resolve_one(input[i].clone(), known, provider.as_ref());
                    results.lock().unwrap()[n] = Some(entry);
                    progress(done.fetch_add(1, Ordering::Relaxed) + 1, jobs.len());
                }
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::coded("cancelled", "导入已取消，未写入歌单"));
    }
    let results = results.into_inner().unwrap();
    list.entries = indices
        .into_iter()
        .map(|n| results[n].clone().unwrap())
        .collect();
    list.deduplicate();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conservative_matching_keeps_platform_ids_and_versions_separate() {
        let entry = Entry {
            source: "wy".into(),
            id: "123".into(),
            title: "歌".into(),
            artist: "甲".into(),
            ..Entry::default()
        };
        let wrong = Track {
            id: "123".into(),
            title: "另一首".into(),
            artist: "乙".into(),
            ..Track::default()
        };
        assert!(resolve_one(entry.clone(), &[wrong], None)
            .resolved
            .is_none());
        let live = Track {
            id: "1".into(),
            title: "歌 (Live)".into(),
            artist: "甲".into(),
            ..Track::default()
        };
        assert!(resolve_one(entry.clone(), &[live], None).resolved.is_none());
        let track = Track {
            id: "2".into(),
            title: "歌".into(),
            artist: "甲".into(),
            album: "A".into(),
            ..Track::default()
        };
        let mut second = track.clone();
        second.id = "3".into();
        second.album = "B".into();
        assert_eq!(
            resolve_one(entry.clone(), &[track.clone(), second.clone()], None).status,
            MatchState::Ambiguous
        );
        let mut exact = entry;
        exact.album = "A".into();
        assert_eq!(
            resolve_one(exact, &[track, second], None)
                .resolved
                .unwrap()
                .id,
            "2"
        );
    }
}
