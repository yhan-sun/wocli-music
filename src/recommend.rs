//! A small, local-first daily mix. History records starts, not listening completion.
use crate::{
    model::{Result, Track},
    provider::Provider,
    store::Store,
};
use rand::{rngs::StdRng, seq::SliceRandom, SeedableRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const CACHE: &str = "daily_recommendations_v1";
const BLOCKED: &str = "daily_blocked_v1";
const LIMIT: usize = 20;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub track: Track,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Daily {
    pub date: String,
    pub items: Vec<Item>,
    pub warnings: Vec<String>,
    pub offline: bool,
}

fn key(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn fingerprint(t: &Track) -> String {
    format!("{}\0{}", key(&t.title), key(&t.artist))
}
fn usable(t: &Track) -> bool {
    !t.id.is_empty()
        && !t.title.trim().is_empty()
        && (t.path.is_empty() || std::path::Path::new(&t.path).is_file())
}
fn seed(date: &str, refresh: bool) -> u64 {
    if refresh {
        return rand::random();
    }
    let hash = Sha256::digest(date.as_bytes());
    u64::from_le_bytes(hash[..8].try_into().unwrap())
}

pub fn blocked(store: &Store) -> Result<HashSet<String>> {
    Ok(store.load(BLOCKED)?.unwrap_or_default())
}
pub fn set_blocked(store: &Store, id: &str, value: bool) -> Result<()> {
    let mut ids = blocked(store)?;
    if value {
        ids.insert(id.to_owned());
    } else {
        ids.remove(id);
    }
    store.save(BLOCKED, &ids)
}

pub fn snapshot(store: &Store) -> Result<Option<Daily>> {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let Some(mut saved) = store.load::<Daily>(CACHE)? else {
        return Ok(None);
    };
    if saved.date != date {
        return Ok(None);
    }
    let excluded = blocked(store)?;
    saved
        .items
        .retain(|i| !excluded.contains(&i.track.id) && usable(&i.track));
    Ok(Some(saved))
}

pub fn daily(store: &Store, provider: &Provider, offline: bool, refresh: bool) -> Result<Daily> {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    if !refresh {
        if let Some(saved) = snapshot(store)? {
            return Ok(saved);
        }
    }
    let favorites = store.favorites()?;
    let history = store.history()?;
    let mut library = Vec::new();
    for t in &favorites {
        library.push((t.clone(), "来自我喜欢".to_string()));
    }
    for (name, _) in store.folders()? {
        for t in store.folder(&name)? {
            library.push((t, format!("来自歌单：{name}")));
        }
    }
    for t in &history {
        library.push((t.clone(), "来自最近播放".to_string()));
    }
    let excluded = blocked(store)?;
    let known: HashSet<_> = library.iter().map(|(t, _)| t.id.clone()).collect();
    let known_fingerprints: HashSet<_> = library.iter().map(|(t, _)| fingerprint(t)).collect();
    let mut artist_weights = HashMap::<String, usize>::new();
    for t in &favorites {
        *artist_weights
            .entry(t.artist.trim().to_string())
            .or_default() += 3;
    }
    for (t, _) in &library {
        *artist_weights
            .entry(t.artist.trim().to_string())
            .or_default() += 1;
    }
    artist_weights.retain(|artist, _| {
        !artist.is_empty()
            && artist != "本地音乐"
            && !artist.contains(['&', '/', '、', ',', '，', ';', '；'])
    });
    let mut artists: Vec<_> = artist_weights.into_iter().collect();
    artists.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut rng = StdRng::seed_from_u64(seed(&date, refresh));
    let mut fresh = Vec::new();
    let mut warnings = Vec::new();
    if !offline {
        for (artist, _) in artists.iter().take(3) {
            match provider.search(artist, 0, 30) {
                Ok((_, tracks)) => {
                    for track in tracks {
                        if key(&track.artist) == key(artist)
                            && !known.contains(&track.id)
                            && !known_fingerprints.contains(&fingerprint(&track))
                        {
                            fresh.push((track, format!("因为你常听 {artist}")));
                        }
                    }
                }
                Err(_) => warnings.push(format!("搜索 {artist} 失败，已使用可用的本地候选")),
            }
        }
    }
    fresh.shuffle(&mut rng);
    library.shuffle(&mut rng);
    // Alternate discovery and familiar tracks, with a per-artist cap.
    let mut seen = HashSet::new();
    let mut ids = HashSet::new();
    let mut counts = HashMap::<String, usize>::new();
    let mut items = Vec::new();
    let mut a = fresh.into_iter();
    let mut b = library.into_iter();
    loop {
        let mut progressed = false;
        for next in [a.next(), b.next()].into_iter().flatten() {
            progressed = true;
            let (track, reason) = next;
            let artist = key(&track.artist);
            let limit_artist = !artist.is_empty() && artist != "本地音乐";
            if !usable(&track)
                || excluded.contains(&track.id)
                || ids.contains(&track.id)
                || seen.contains(&fingerprint(&track))
                || (limit_artist && counts.get(&artist).copied().unwrap_or(0) >= 3)
            {
                continue;
            }
            ids.insert(track.id.clone());
            seen.insert(fingerprint(&track));
            *counts.entry(artist).or_default() += 1;
            items.push(Item { track, reason });
            if items.len() == LIMIT {
                break;
            }
        }
        if !progressed || items.len() == LIMIT {
            break;
        }
    }
    let daily = Daily {
        date,
        items,
        warnings,
        offline,
    };
    store.remember(
        &daily
            .items
            .iter()
            .map(|item| item.track.clone())
            .collect::<Vec<_>>(),
    )?;
    store.save(CACHE, &daily)?;
    Ok(daily)
}
