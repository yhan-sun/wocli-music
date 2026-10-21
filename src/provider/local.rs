use crate::model::{Error, Result, Track};
use crate::provider::traits::{MusicProvider, Stream};
use std::path::Path;
use std::sync::atomic::AtomicBool;

pub struct LocalProvider;

impl Default for LocalProvider {
    fn default() -> Self {
        Self
    }
}

impl MusicProvider for LocalProvider {
    fn name(&self) -> &'static str {
        "local"
    }

    fn search(&self, _keyword: &str, _page: usize, _limit: usize) -> Result<(usize, Vec<Track>)> {
        Ok((0, vec![]))
    }

    fn details(&self, id: &str) -> Result<Track> {
        let p = Path::new(id);
        Track::local(p)
    }

    fn stream(
        &self,
        track: &Track,
        _requested_quality: &str,
        _cancelled: &AtomicBool,
    ) -> Result<Stream> {
        if track.path.is_empty() {
            return Err(Error::new("非本地音频文件"));
        }
        let p = Path::new(&track.path);
        if !p.exists() {
            return Err(Error::new("本地音频文件不存在"));
        }
        let metadata = std::fs::metadata(p)?;
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("flac")
            .to_string();
        Ok(Stream {
            url: format!("file://{}", track.path),
            quality: "flac".to_string(),
            size: metadata.len(),
            extension: ext,
        })
    }

    fn lyrics(&self, track: &Track) -> Result<String> {
        if track.path.is_empty() {
            return Err(Error::new("非本地音频文件"));
        }
        let p = Path::new(&track.path);
        let lrc_path = p.with_extension("lrc");
        if lrc_path.exists() {
            Ok(std::fs::read_to_string(lrc_path)?)
        } else {
            Err(Error::new("本地无同名 LRC 歌词"))
        }
    }

    fn download(
        &self,
        stream: &Stream,
        target: &Path,
        _allow_existing: bool,
        _cancelled: &AtomicBool,
        _progress: &mut (dyn FnMut(u64, u64) + Send),
    ) -> Result<u64> {
        let source_path = stream.url.strip_prefix("file://").unwrap_or(&stream.url);
        let bytes = std::fs::copy(source_path, target)?;
        Ok(bytes)
    }
}
