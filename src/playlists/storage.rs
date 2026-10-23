use super::*;
use crate::store::{now, Store};
use rusqlite::{params, OptionalExtension};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Conflict {
    #[default]
    Rename,
    Error,
    Merge,
    Replace,
}
impl Conflict {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "rename" => Ok(Self::Rename),
            "error" => Ok(Self::Error),
            "merge" => Ok(Self::Merge),
            "replace" => Ok(Self::Replace),
            _ => Err(Error::new(
                "--on-conflict 应为 rename / error / merge / replace",
            )),
        }
    }
}
#[derive(Debug, Serialize)]
pub struct SaveReport {
    pub name: String,
    pub summary: Summary,
    pub backup: Option<i64>,
}
/// Reconcile the metadata snapshot with ordinary add/remove operations on the playable view.
pub fn stored(store: &Store, name: &str) -> Result<Playlist> {
    let tracks = store.folder(name)?;
    let raw: Option<String> = store
        .db
        .query_row(
            "SELECT data FROM playlist_imports WHERE name=?",
            [name],
            |r| r.get(0),
        )
        .optional()?;
    let mut list = raw
        .map(|s| serde_json::from_str::<Playlist>(&s))
        .transpose()?
        .unwrap_or_else(|| Playlist {
            name: name.into(),
            format: "wocli".into(),
            entries: vec![],
        });
    list.name = name.into();
    let ids = tracks.iter().map(|t| t.id.as_str()).collect::<HashSet<_>>();
    list.entries.retain(|e| {
        e.resolved
            .as_ref()
            .is_none_or(|t| ids.contains(t.id.as_str()))
    });
    let present = list
        .entries
        .iter()
        .filter_map(|e| e.resolved.as_ref().map(|t| t.id.clone()))
        .collect::<HashSet<_>>();
    list.entries.extend(
        tracks
            .into_iter()
            .filter(|t| !present.contains(&t.id))
            .map(Entry::from_track),
    );
    list.deduplicate();
    Ok(list)
}
pub fn save(store: &Store, mut list: Playlist, conflict: Conflict) -> Result<SaveReport> {
    list.validate()?;
    let tx = store.db.unchecked_transaction()?;
    let exists = store
        .db
        .query_row("SELECT 1 FROM playlists WHERE name=?", [&list.name], |_| {
            Ok(())
        })
        .optional()?
        .is_some();
    let mut backup = None;
    if exists {
        match conflict {
            Conflict::Error => {
                return Err(Error::new(
                    "已存在同名歌单，请指定 --on-conflict rename / merge / replace",
                ))
            }
            Conflict::Rename => {
                let original = list.name.clone();
                let mut n = 2;
                loop {
                    let suffix = format!(" (导入 {n})");
                    list.name = format!(
                        "{}{}",
                        original
                            .chars()
                            .take(100 - suffix.chars().count())
                            .collect::<String>(),
                        suffix
                    );
                    if store
                        .db
                        .query_row("SELECT 1 FROM playlists WHERE name=?", [&list.name], |_| {
                            Ok(())
                        })
                        .optional()?
                        .is_none()
                    {
                        break;
                    }
                    n += 1;
                }
            }
            Conflict::Merge | Conflict::Replace => {
                let old = stored(store, &list.name)?;
                tx.execute(
                    "INSERT INTO playlist_backups(name,data,created) VALUES (?,?,?)",
                    params![list.name, serde_json::to_string(&old)?, now()],
                )?;
                backup = Some(tx.last_insert_rowid());
                if conflict == Conflict::Merge {
                    let mut entries = old.entries;
                    let mut seen = entries.iter().map(Entry::key).collect::<HashSet<_>>();
                    entries.extend(list.entries.into_iter().filter(|e| seen.insert(e.key())));
                    list.entries = entries;
                    list.validate()?;
                }
            }
        }
    }
    list.deduplicate();
    tx.execute("INSERT OR IGNORE INTO playlists VALUES (?)", [&list.name])?;
    tx.execute("DELETE FROM members WHERE name=?", [&list.name])?;
    for (i, entry) in list.entries.iter().enumerate() {
        if let Some(track) = entry
            .resolved
            .as_ref()
            .filter(|_| entry.status == MatchState::Matched && !entry.duplicate)
        {
            tx.execute(
                "INSERT OR REPLACE INTO tracks VALUES (?,?)",
                params![track.id, serde_json::to_string(track)?],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO members VALUES (?,?,?)",
                params![list.name, track.id, i],
            )?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO playlist_imports VALUES (?,?)",
        params![list.name, serde_json::to_string(&list)?],
    )?;
    tx.commit()?;
    Ok(SaveReport {
        name: list.name.clone(),
        summary: list.summary(),
        backup,
    })
}
pub fn backups(store: &Store, name: &str) -> Result<Vec<serde_json::Value>> {
    let mut stmt = store
        .db
        .prepare("SELECT id,created FROM playlist_backups WHERE name=? ORDER BY id DESC")?;
    let rows = stmt.query_map([name], |r| {
        Ok(serde_json::json!({"id":r.get::<_,i64>(0)?,"created":r.get::<_,f64>(1)?}))
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}
pub fn restore(store: &Store, name: &str, id: i64) -> Result<SaveReport> {
    let raw: String = store
        .db
        .query_row(
            "SELECT data FROM playlist_backups WHERE name=? AND id=?",
            params![name, id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::new("找不到该歌单快照"))?;
    let mut list: Playlist = serde_json::from_str(&raw)?;
    list.name = name.into();
    save(store, list, Conflict::Replace)
}
