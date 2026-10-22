//! Persisted source order shared by the CLI and the already-running service.
use crate::{
    model::{clean, Error, Result, Track, QUALITIES},
    provider::{lx, Stream},
    store::Paths,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceKind {
    Native,
    Http {
        url: String,
        #[serde(default)]
        response_path: Option<String>,
    },
    Lx {
        sha256: String,
        #[serde(default)]
        origin: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub builtin: bool,
    pub qualities: Vec<String>,
    #[serde(flatten)]
    pub kind: SourceKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sources {
    pub sources: Vec<Source>,
}
impl Default for Sources {
    fn default() -> Self {
        Self {
            sources: vec![
                Source {
                    id: "kuwo".into(),
                    name: "酷我内置（P2P / 公开接口）".into(),
                    enabled: true,
                    builtin: true,
                    qualities: vec!["flac".into(), "320k".into(), "128k".into()],
                    kind: SourceKind::Native,
                },
                // Domestic platform endpoints; no GitHub or third-party scripts at playback.
                Source {
                    id: "kuwo-mobile".into(),
                    name: "酷我移动接口备用".into(),
                    enabled: true,
                    builtin: true,
                    qualities: vec!["flac".into(), "320k".into(), "128k".into()],
                    kind: SourceKind::Native,
                },
            ],
        }
    }
}
impl Sources {
    pub fn load(home: &Path) -> Result<Self> {
        let file = home.join("sources.json");
        if !file.exists() {
            return Ok(Self::default());
        }
        let data = read_limited(&file, 256 * 1024)?;
        let sources: Self = serde_json::from_slice(&data)?;
        sources.validate()?;
        Ok(sources)
    }
    fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        if self.sources.len() > 32 {
            return Err(Error::new("最多配置 32 个音源"));
        }
        for s in &self.sources {
            if s.id.is_empty()
                || s.id.len() > 64
                || !s
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                || !ids.insert(&s.id)
            {
                return Err(Error::new("音源 ID 无效或重复"));
            }
            if s.name.trim().is_empty()
                || s.name.len() > 300
                || s.qualities.is_empty()
                || s.qualities.len() > 4
                || s.qualities
                    .iter()
                    .any(|q| !QUALITIES.contains(&q.as_str()) || q == "best")
            {
                return Err(Error::new("音源名称或音质列表无效"));
            }
            match &s.kind {
                SourceKind::Native if !["kuwo", "kuwo-mobile"].contains(&s.id.as_str()) => {
                    return Err(Error::new("未知内置音源"))
                }
                SourceKind::Http { url, response_path } => {
                    validate_url(url)?;
                    if response_path
                        .as_ref()
                        .is_some_and(|p| !p.is_empty() && !p.starts_with('/'))
                    {
                        return Err(Error::new(
                            "response_path 应为 JSON Pointer，例如 /data/url",
                        ));
                    }
                }
                SourceKind::Lx { sha256, .. }
                    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) =>
                {
                    return Err(Error::new("LX 脚本摘要无效"))
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn save(&self, home: &Path) -> Result<()> {
        self.validate()?;
        let mut file = tempfile::NamedTempFile::new_in(home)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.as_file().sync_all()?;
        file.persist(home.join("sources.json"))
            .map_err(|e| Error::new(e.error.to_string()))?;
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<&Source> {
        self.sources
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| Error::new(format!("找不到音源 {id}，运行 wocli source list")))
    }
}

pub fn mutate(home: &Path, action: impl FnOnce(&mut Sources) -> Result<()>) -> Result<Sources> {
    fs::create_dir_all(home)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(home.join("sources.lock"))?;
    lock.lock_exclusive()?;
    let mut sources = Sources::load(home)?;
    action(&mut sources)?;
    sources.save(home)?;
    Ok(sources)
}
pub fn enable(home: &Path, id: &str, enabled: bool) -> Result<Sources> {
    mutate(home, |s| {
        s.get(id)?;
        s.sources.iter_mut().find(|s| s.id == id).unwrap().enabled = enabled;
        Ok(())
    })
}
pub fn prefer(home: &Path, id: &str) -> Result<Sources> {
    mutate(home, |s| {
        s.get(id)?;
        let index = s.sources.iter().position(|s| s.id == id).unwrap();
        let mut source = s.sources.remove(index);
        source.enabled = true;
        s.sources.insert(0, source);
        Ok(())
    })
}
pub fn remove(home: &Path, id: &str) -> Result<Sources> {
    mutate(home, |s| {
        if s.get(id)?.builtin {
            return Err(Error::new("内置源请使用 source disable 停用"));
        }
        s.sources.retain(|s| s.id != id);
        Ok(())
    })
}

pub fn add(paths: &Paths, input: &str, name: Option<&str>, agent: &ureq::Agent) -> Result<Source> {
    let bytes = if input.starts_with("https://") || input.starts_with("http://") {
        validate_url(input)?;
        let mut data = Vec::new();
        agent
            .get(input)
            .timeout(Duration::from_secs(15))
            .call()
            .map_err(|_| Error::coded("network", "音源下载失败，也可下载后从本地文件导入"))?
            .into_reader()
            .take(lx::MAX_SCRIPT + 1)
            .read_to_end(&mut data)?;
        data
    } else {
        read_limited(&crate::cli::expand_path(input), lx::MAX_SCRIPT)?
    };
    if bytes.len() as u64 > lx::MAX_SCRIPT {
        return Err(Error::new("音源文件超过 2 MiB"));
    }
    let script = String::from_utf8(bytes).map_err(|_| Error::new("音源文件必须使用 UTF-8"))?;
    let script = script.trim_start_matches('\u{feff}');
    let sha = format!("{:x}", Sha256::digest(script.as_bytes()));
    let (title, qualities, kind) = if script.trim_start().starts_with('{') {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Template {
            name: String,
            url: String,
            #[serde(default)]
            response_path: Option<String>,
            #[serde(default = "default_qualities")]
            qualities: Vec<String>,
        }
        let template: Template = serde_json::from_str(script)?;
        (
            template.name,
            template.qualities,
            SourceKind::Http {
                url: template.url,
                response_path: template.response_path,
            },
        )
    } else {
        let info = lx::inspect(script, agent)?;
        let qualities = info["sources"]["kw"]["qualitys"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|q| ["flac", "320k", "128k"].contains(q))
            .map(str::to_owned)
            .collect();
        (
            lx::metadata(script)["name"]
                .as_str()
                .unwrap_or("自定义 LX 音源")
                .to_string(),
            qualities,
            SourceKind::Lx {
                sha256: sha.clone(),
                origin: if input.starts_with("http") {
                    input.into()
                } else {
                    String::new()
                },
            },
        )
    };
    let source = Source {
        id: format!("user-{}", &sha[..12]),
        name: clean(name.unwrap_or(&title)),
        enabled: true,
        builtin: false,
        qualities,
        kind,
    };
    // Validate before writing anything, and use a content-addressed immutable script file.
    Sources {
        sources: vec![source.clone()],
    }
    .validate()?;
    if matches!(source.kind, SourceKind::Lx { .. }) {
        let directory = paths.home.join("sources/scripts");
        fs::create_dir_all(&directory)?;
        let mut file = tempfile::NamedTempFile::new_in(&directory)?;
        file.write_all(script.as_bytes())?;
        file.persist(directory.join(format!("{sha}.js")))
            .map_err(|e| Error::new(e.error.to_string()))?;
    }
    mutate(&paths.home, |s| {
        if s.sources.iter().any(|s| s.id == source.id) {
            return Err(Error::new(format!("音源已导入：{}", source.id)));
        }
        s.sources.push(source.clone());
        Ok(())
    })?;
    Ok(source)
}
fn default_qualities() -> Vec<String> {
    vec!["flac".into(), "320k".into(), "128k".into()]
}
pub fn quality_order(requested: &str) -> &'static [&'static str] {
    match requested {
        "128k" => &["128k"],
        "320k" => &["320k", "128k"],
        _ => &["flac", "320k", "128k"],
    }
}
pub fn script_path(home: &Path, sha: &str) -> PathBuf {
    home.join("sources/scripts").join(format!("{sha}.js"))
}
pub fn read_script(home: &Path, sha: &str) -> Result<String> {
    let bytes = read_limited(&script_path(home, sha), lx::MAX_SCRIPT)?;
    if format!("{:x}", Sha256::digest(&bytes)) != sha {
        return Err(Error::new("音源脚本校验失败，请重新导入"));
    }
    String::from_utf8(bytes).map_err(|_| Error::new("无效音源编码"))
}
pub fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::new("音源配置或脚本过大"));
    }
    Ok(bytes)
}
pub fn validate_url(url: &str) -> Result<()> {
    if url.len() > 16384
        || !(url.starts_with("http://") || url.starts_with("https://"))
        || url.chars().any(char::is_control)
    {
        return Err(Error::new("音源地址必须是 HTTP/HTTPS"));
    }
    Ok(())
}
pub fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
pub fn resolve(
    source: &Source,
    home: &Path,
    track: &Track,
    quality: &str,
    agent: &ureq::Agent,
    cancelled: Arc<AtomicBool>,
) -> Result<Stream> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::coded("cancelled", "已取消"));
    }
    let url = match &source.kind {
        SourceKind::Native => return Err(Error::new("内置源由原生提供器处理")),
        SourceKind::Lx { sha256, .. } => lx::resolve(
            &read_script(home, sha256)?,
            track,
            quality,
            agent,
            cancelled,
        )?,
        SourceKind::Http { url, response_path } => {
            let replacements = HashMap::from([
                ("id", track.id.trim_start_matches("kuwo:")),
                ("quality", quality),
                ("title", track.title.as_str()),
                ("artist", track.artist.as_str()),
                ("source", "kw"),
            ]);
            let mut address = url.clone();
            for (k, v) in replacements {
                address = address.replace(&format!("{{{k}}}"), &encode(v));
            }
            if let Some(pointer) = response_path {
                let response = agent
                    .get(&address)
                    .timeout(Duration::from_secs(8))
                    .call()
                    .map_err(|_| Error::coded("network", "备用音源解析请求失败"))?;
                let mut bytes = Vec::new();
                response
                    .into_reader()
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > 1024 * 1024 {
                    return Err(Error::new("备用音源响应过大"));
                }
                let value: Value = serde_json::from_slice(&bytes)?;
                value
                    .pointer(pointer)
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::new("备用音源未返回指定音频地址"))?
                    .into()
            } else {
                address
            }
        }
    };
    validate_url(&url)?;
    Ok(Stream {
        url,
        quality: quality.into(),
        size: 0,
        extension: if quality == "flac" { ".flac" } else { ".mp3" }.into(),
    })
}

pub fn describe(sources: &Sources) -> Value {
    json!({"sources":sources.sources,"scope":"kuwo","reload":"next_request"})
}
