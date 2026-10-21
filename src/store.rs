use crate::model::{Error, Result, Settings, Track, MODES, QUALITIES};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
pub struct Paths {
    pub home: PathBuf,
    pub runtime: PathBuf,
}
impl Paths {
    pub fn discover() -> Result<Self> {
        let home = if let Some(value) = env::var_os("WOCLI_HOME") {
            PathBuf::from(value)
        } else {
            #[cfg(windows)]
            {
                let config = env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .or_else(|| {
                        env::var_os("USERPROFILE").map(|v| PathBuf::from(v).join(".config"))
                    })
                    .ok_or_else(|| Error::new("无法确定配置目录，请设置 WOCLI_HOME"))?;
                config.join("wocli-music")
            }
            #[cfg(not(windows))]
            {
                let config = env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .or_else(|| env::var_os("HOME").map(|v| PathBuf::from(v).join(".config")))
                    .ok_or_else(|| Error::new("无法确定配置目录，请设置 WOCLI_HOME"))?;
                config.join("wocli-music")
            }
        };
        fs::create_dir_all(&home)?;
        let home = fs::canonicalize(&home).unwrap_or(home);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&home, fs::Permissions::from_mode(0o700));
        }
        let tag = format!("{:x}", Sha256::digest(home.to_string_lossy().as_bytes()));
        let runtime = if let Some(value) = env::var_os("WOCLI_RUNTIME_DIR") {
            PathBuf::from(value)
        } else {
            #[cfg(windows)]
            {
                let base = env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(env::temp_dir);
                base.join("wocli-music").join("run").join(&tag[..12])
            }
            #[cfg(not(windows))]
            {
                let uid = unsafe { libc::getuid() };
                if let Some(value) = env::var_os("XDG_RUNTIME_DIR") {
                    PathBuf::from(value).join(format!("wocli-rust-{}", &tag[..12]))
                } else {
                    PathBuf::from(format!("/tmp/wocli-rust-{uid}-{}", &tag[..12]))
                }
            }
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let uid = unsafe { libc::getuid() };
            if let Ok(meta) = fs::symlink_metadata(&runtime) {
                if meta.file_type().is_symlink() || meta.uid() != uid {
                    return Err(Error::new("运行目录不属于当前用户"));
                }
            }
            fs::create_dir_all(&runtime)?;
            let _ = fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700));
        }
        #[cfg(not(unix))]
        {
            fs::create_dir_all(&runtime)?;
        }
        Ok(Self { home, runtime })
    }
    pub fn socket(&self) -> PathBuf {
        #[cfg(windows)]
        {
            self.runtime.join("control.endpoint")
        }
        #[cfg(not(windows))]
        {
            self.runtime.join("control.sock")
        }
    }
    pub fn settings(&self) -> Result<Settings> {
        let path = self.home.join("settings.json");
        let settings: Settings = if path.exists() {
            serde_json::from_slice(&fs::read(path)?)?
        } else {
            Settings::default()
        };
        Self::validate(&settings)?;
        Ok(settings)
    }
    pub fn validate(settings: &Settings) -> Result<()> {
        if settings.volume > 100
            || !MODES.contains(&settings.mode.as_str())
            || !QUALITIES.contains(&settings.quality.as_str())
            || settings.lyric_offset.unsigned_abs() > 60_000
        {
            return Err(Error::new("音量、播放方式、音质或歌词偏移无效"));
        }
        Ok(())
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        Self::validate(settings)?;
        let temporary = self
            .home
            .join(format!(".settings-{}.tmp", std::process::id()));
        fs::write(&temporary, serde_json::to_vec_pretty(settings)?)?;
        fs::rename(temporary, self.home.join("settings.json"))?;
        Ok(())
    }
}
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub struct Store {
    pub db: Connection,
}
impl Store {
    pub fn open(paths: &Paths) -> Result<Self> {
        let db = Connection::open(paths.home.join("library.sqlite3"))?;
        db.busy_timeout(Duration::from_secs(10))?;
        db.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS tracks (id TEXT PRIMARY KEY, data TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS favorites (id TEXT PRIMARY KEY, added REAL NOT NULL);
            CREATE TABLE IF NOT EXISTS history (seq INTEGER PRIMARY KEY, id TEXT, played REAL);
            CREATE TABLE IF NOT EXISTS playlists (name TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS members (name TEXT,id TEXT,position INTEGER,PRIMARY KEY(name,id));
            CREATE TABLE IF NOT EXISTS state (key TEXT PRIMARY KEY,data TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS lyrics (id TEXT PRIMARY KEY,text TEXT NOT NULL);")?;
        Ok(Self { db })
    }
    pub fn remember(&self, tracks: &[Track]) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        for track in tracks {
            tx.execute(
                "INSERT OR REPLACE INTO tracks VALUES (?,?)",
                params![track.id, serde_json::to_string(track)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn track(&self, id: &str) -> Result<Option<Track>> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT data FROM tracks WHERE id=?", [id], |r| r.get(0))
            .optional()?;
        raw.map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }
    pub fn save<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.db.execute(
            "INSERT OR REPLACE INTO state VALUES (?,?)",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }
    pub fn load<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let raw: Option<String> = self
            .db
            .query_row("SELECT data FROM state WHERE key=?", [key], |r| r.get(0))
            .optional()?;
        raw.map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }
    fn tracks(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<Track>> {
        let mut stmt = self.db.prepare(sql)?;
        let rows = stmt.query_map(args, |r| r.get::<_, String>(0))?;
        rows.map(|v| Ok(serde_json::from_str(&v?)?)).collect()
    }
    pub fn favorites(&self) -> Result<Vec<Track>> {
        self.tracks(
            "SELECT t.data FROM favorites f JOIN tracks t ON f.id=t.id ORDER BY f.added DESC",
            &[],
        )
    }
    pub fn history(&self) -> Result<Vec<Track>> {
        self.tracks("SELECT t.data FROM history h JOIN tracks t ON h.id=t.id GROUP BY h.id ORDER BY MAX(h.played) DESC LIMIT 100", &[])
    }
    pub fn is_favorite(&self, id: &str) -> bool {
        self.db
            .query_row("SELECT 1 FROM favorites WHERE id=?", [id], |_| Ok(()))
            .optional()
            .ok()
            .flatten()
            .is_some()
    }
    pub fn favorite(&self, track: &Track) -> Result<bool> {
        self.remember(std::slice::from_ref(track))?;
        let liked = self.is_favorite(&track.id);
        if liked {
            self.db
                .execute("DELETE FROM favorites WHERE id=?", [&track.id])?;
        } else {
            self.db.execute(
                "INSERT OR IGNORE INTO favorites VALUES (?,?)",
                params![track.id, now()],
            )?;
        }
        Ok(!liked)
    }
    pub fn played(&self, track: &Track) -> Result<()> {
        self.remember(std::slice::from_ref(track))?;
        self.db.execute(
            "INSERT INTO history(id,played) VALUES (?,?)",
            params![track.id, now()],
        )?;
        self.db.execute("DELETE FROM history WHERE seq NOT IN (SELECT seq FROM history ORDER BY seq DESC LIMIT 1000)", [])?;
        Ok(())
    }
    pub fn folders(&self) -> Result<Vec<(String, usize)>> {
        let mut stmt = self.db.prepare("SELECT p.name,COUNT(m.id) FROM playlists p LEFT JOIN members m ON p.name=m.name GROUP BY p.name ORDER BY p.name")?;
        let values = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(values)
    }
    pub fn create_folder(&self, name: &str) -> Result<()> {
        if name.trim().is_empty() || name.chars().count() > 100 {
            return Err(Error::new("收藏夹名称应为 1–100 字"));
        }
        self.db
            .execute("INSERT OR IGNORE INTO playlists VALUES (?)", [name])?;
        Ok(())
    }
    pub fn folder(&self, name: &str) -> Result<Vec<Track>> {
        if !self.folders()?.iter().any(|(v, _)| v == name) {
            return Err(Error::new("找不到收藏夹"));
        }
        self.tracks("SELECT t.data FROM members m JOIN tracks t ON m.id=t.id WHERE m.name=? ORDER BY m.position", &[&name])
    }
    pub fn collect(&self, name: &str, track: &Track) -> Result<()> {
        self.create_folder(name)?;
        self.remember(std::slice::from_ref(track))?;
        self.db.execute("INSERT OR IGNORE INTO members VALUES (?,?,(SELECT COALESCE(MAX(position),0)+1 FROM members WHERE name=?))", params![name, track.id, name])?;
        Ok(())
    }
    pub fn remove(&self, name: &str, id: &str) -> Result<()> {
        self.db
            .execute("DELETE FROM members WHERE name=? AND id=?", [name, id])?;
        Ok(())
    }
    pub fn rename(&self, old: &str, new: &str) -> Result<()> {
        self.folder(old)?;
        if old == new {
            return Ok(());
        }
        if self.folders()?.iter().any(|(v, _)| v == new) {
            return Err(Error::new("已存在同名收藏夹"));
        }
        let tx = self.db.unchecked_transaction()?;
        self.create_folder(new)?;
        tx.execute("UPDATE members SET name=? WHERE name=?", [new, old])?;
        tx.execute("DELETE FROM playlists WHERE name=?", [old])?;
        tx.commit()?;
        Ok(())
    }
    pub fn delete(&self, name: &str) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        tx.execute("DELETE FROM members WHERE name=?", [name])?;
        tx.execute("DELETE FROM playlists WHERE name=?", [name])?;
        tx.commit()?;
        Ok(())
    }
    pub fn lyric(&self, id: &str) -> Result<String> {
        Ok(self
            .db
            .query_row("SELECT text FROM lyrics WHERE id=?", [id], |r| r.get(0))
            .optional()?
            .unwrap_or_default())
    }
    pub fn save_lyric(&self, id: &str, text: &str) -> Result<()> {
        self.db
            .execute("INSERT OR REPLACE INTO lyrics VALUES (?,?)", [id, text])?;
        Ok(())
    }
}
