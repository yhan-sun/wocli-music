use crate::model::{Result, Track};
use std::path::Path;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug)]
pub struct Stream {
    pub url: String,
    pub quality: String,
    pub size: u64,
    pub extension: String,
}

pub trait MusicProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn search(&self, keyword: &str, page: usize, limit: usize) -> Result<(usize, Vec<Track>)>;
    fn details(&self, id: &str) -> Result<Track>;
    fn stream(&self, track: &Track, requested_quality: &str, cancelled: &AtomicBool) -> Result<Stream>;
    fn lyrics(&self, track: &Track) -> Result<String>;
    fn download(
        &self,
        stream: &Stream,
        target: &Path,
        allow_existing: bool,
        cancelled: &AtomicBool,
        progress: &mut (dyn FnMut(u64, u64) + Send),
    ) -> Result<u64>;
}
