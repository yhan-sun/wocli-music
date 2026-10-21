pub mod kuwo;
pub mod local;
pub mod traits;

pub use kuwo::{decode_reply, encode_query, parse_json, KuwoProvider};
pub use local::LocalProvider;
pub use traits::{MusicProvider, Stream};

use crate::model::{Result, Track};
use std::path::Path;
use std::sync::atomic::AtomicBool;

pub struct Provider {
    pub kuwo: KuwoProvider,
    pub local: LocalProvider,
}

impl Provider {
    pub fn new(direct: bool) -> Self {
        Self {
            kuwo: KuwoProvider::new(direct),
            local: LocalProvider,
        }
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
            self.local.lyrics(track).or_else(|_| self.kuwo.lyrics(track))
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
