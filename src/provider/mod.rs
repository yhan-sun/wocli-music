pub mod kuwo;
pub mod local;
pub mod lx;
pub mod traits;

pub use kuwo::{decode_reply, encode_query, parse_json, KuwoProvider};
pub use local::LocalProvider;
pub use traits::{MusicProvider, Stream};

use crate::{
    model::{Error, Result, Track},
    sources::{SourceKind, Sources},
};
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub struct Provider {
    pub kuwo: KuwoProvider,
    pub local: LocalProvider,
    home: Option<PathBuf>,
}

impl Provider {
    pub fn new(direct: bool) -> Self {
        Self {
            kuwo: KuwoProvider::new(direct),
            local: LocalProvider,
            home: None,
        }
    }

    pub fn with_sources(direct: bool, home: PathBuf) -> Self {
        Self {
            home: Some(home),
            ..Self::new(direct)
        }
    }

    pub fn native_stream(
        &self,
        id: &str,
        track: &Track,
        quality: &str,
        cancelled: &AtomicBool,
    ) -> Result<Stream> {
        match id {
            "kuwo" => self.kuwo.stream(track, quality, cancelled),
            "kuwo-mobile" => self.kuwo.mobile_stream(track, quality, cancelled),
            _ => Err(Error::new("未知内置音源")),
        }
    }

    /// A source is excluded after its resolved audio fails, so retries never loop on a bad URL.
    pub fn resolve(
        &self,
        track: &Track,
        quality: &str,
        cancelled: Arc<AtomicBool>,
        excluded: &HashSet<String>,
    ) -> Result<(String, Stream)> {
        if !track.path.is_empty() {
            return Ok((
                "local".into(),
                self.local.stream(track, quality, &cancelled)?,
            ));
        }
        let Some(home) = &self.home else {
            return Ok(("kuwo".into(), self.kuwo.stream(track, quality, &cancelled)?));
        };
        let sources = Sources::load(home)?;
        let mut errors = Vec::new();
        let started = std::time::Instant::now();
        for source in sources
            .sources
            .iter()
            .filter(|s| s.enabled && !excluded.contains(&s.id))
        {
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(Error::coded("cancelled", "已取消"));
            }
            if started.elapsed() > std::time::Duration::from_secs(75) {
                errors.push("本次换源已达到 75 秒限制".into());
                break;
            }
            if matches!(source.kind, SourceKind::Native) {
                match self.native_stream(&source.id, track, quality, &cancelled) {
                    Ok(stream) => return Ok((source.id.clone(), stream)),
                    Err(e) => {
                        errors.push(format!("{}: {}", source.name, e));
                        errors.extend(e.details.iter().map(|d| format!("{}: {d}", source.name)));
                    }
                }
                continue;
            }
            for q in crate::sources::quality_order(quality)
                .iter()
                .filter(|q| source.qualities.iter().any(|s| s == **q))
            {
                if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(Error::coded("cancelled", "已取消"));
                }
                if started.elapsed() > std::time::Duration::from_secs(75) {
                    errors.push("本次换源已达到 75 秒限制".into());
                    break;
                }
                let result = crate::sources::resolve(
                    source,
                    home,
                    track,
                    q,
                    self.agent(),
                    cancelled.clone(),
                )
                .and_then(|mut stream| {
                    stream.size = self.kuwo.probe(&stream.url, 0)?;
                    Ok(stream)
                });
                match result {
                    Ok(stream) => return Ok((source.id.clone(), stream)),
                    Err(e) if e.code == "cancelled" => return Err(e),
                    Err(e) => errors.push(format!("{} {q}: {}", source.name, e)),
                }
            }
        }
        Err(Error {
            code: "source_unavailable".into(),
            message: "所有已启用音源均不可用；可运行 wocli source list / test 检查".into(),
            details: errors,
        })
    }

    pub fn agent(&self) -> &ureq::Agent {
        self.kuwo.agent()
    }

    pub fn search(&self, keyword: &str, page: usize, limit: usize) -> Result<(usize, Vec<Track>)> {
        self.kuwo.search(keyword, page, limit)
    }

    pub fn details(&self, id: &str) -> Result<Track> {
        if id.starts_with('/') || id.starts_with('\\') || id.contains(":\\") || id.contains(":/") {
            self.local.details(id)
        } else {
            self.kuwo.details(id)
        }
    }

    pub fn stream(
        &self,
        track: &Track,
        requested_quality: &str,
        cancelled: &AtomicBool,
    ) -> Result<Stream> {
        if !track.path.is_empty() {
            self.local.stream(track, requested_quality, cancelled)
        } else {
            self.kuwo.stream(track, requested_quality, cancelled)
        }
    }

    pub fn lyrics(&self, track: &Track) -> Result<String> {
        if !track.path.is_empty() {
            self.local
                .lyrics(track)
                .or_else(|_| self.kuwo.lyrics(track))
        } else {
            self.kuwo.lyrics(track)
        }
    }

    pub fn download(
        &self,
        stream: &Stream,
        target: &Path,
        allow_existing: bool,
        cancelled: &AtomicBool,
        mut progress: impl FnMut(u64, u64) + Send,
    ) -> Result<u64> {
        if stream.url.starts_with("file://") {
            self.local
                .download(stream, target, allow_existing, cancelled, &mut progress)
        } else {
            self.kuwo
                .download(stream, target, allow_existing, cancelled, progress)
        }
    }
}
