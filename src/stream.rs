use crate::model::{Error, Result};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

pub const PREBUFFER_SIZE: u64 = 128 * 1024; // 128 KB for instant container probe & playback

pub struct StreamCoord {
    pub written: AtomicU64,
    pub total: AtomicU64,
    pub completed: AtomicBool,
    pub failed: AtomicBool,
    pub cancelled: AtomicBool,
    pub notifier: (Mutex<()>, Condvar),
}

impl StreamCoord {
    pub fn new(total_size: u64) -> Self {
        Self {
            written: AtomicU64::new(0),
            total: AtomicU64::new(total_size),
            completed: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            notifier: (Mutex::new(()), Condvar::new()),
        }
    }

    pub fn notify_progress(&self, bytes_written: u64) {
        self.written.store(bytes_written, Ordering::Release);
        let _guard = self.notifier.0.lock().unwrap();
        self.notifier.1.notify_all();
    }

    pub fn mark_completed(&self, total_bytes: u64) {
        self.written.store(total_bytes, Ordering::Release);
        self.completed.store(true, Ordering::Release);
        let _guard = self.notifier.0.lock().unwrap();
        self.notifier.1.notify_all();
    }

    pub fn mark_failed(&self) {
        self.failed.store(true, Ordering::Release);
        let _guard = self.notifier.0.lock().unwrap();
        self.notifier.1.notify_all();
    }

    pub fn mark_cancelled(&self) {
        self.cancelled.store(true, Ordering::Release);
        let _guard = self.notifier.0.lock().unwrap();
        self.notifier.1.notify_all();
    }
}

pub struct ProgressiveReader {
    file: File,
    pos: u64,
    coord: Arc<StreamCoord>,
}

impl ProgressiveReader {
    pub fn new(file: File, coord: Arc<StreamCoord>) -> Self {
        Self {
            file,
            pos: 0,
            coord,
        }
    }
}

impl Read for ProgressiveReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.coord.cancelled.load(Ordering::Relaxed) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "stream cancelled",
                ));
            }
            if self.coord.failed.load(Ordering::Relaxed) {
                return Err(std::io::Error::other("stream download failed"));
            }

            let written = self.coord.written.load(Ordering::Acquire);
            if self.pos < written {
                let available = (written - self.pos).min(buf.len() as u64) as usize;
                self.file.seek(SeekFrom::Start(self.pos))?;
                let n = self.file.read(&mut buf[..available])?;
                self.pos += n as u64;
                return Ok(n);
            }

            if self.coord.completed.load(Ordering::Acquire) {
                return Ok(0); // Clean EOF
            }

            // Wait for more bytes to be downloaded
            let (lock, cvar) = &self.coord.notifier;
            let guard = lock.lock().unwrap();
            let _ = cvar.wait_timeout(guard, Duration::from_millis(60));
        }
    }
}

impl Seek for ProgressiveReader {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let total = self.coord.total.load(Ordering::Relaxed);
        let new_pos = match from {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => {
                let cur = self.pos as i64 + offset;
                if cur < 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "negative seek position",
                    ));
                }
                cur as u64
            }
            SeekFrom::End(offset) => {
                let base = if total > 0 {
                    total
                } else {
                    self.coord.written.load(Ordering::Relaxed)
                };
                let end = base as i64 + offset;
                if end < 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "seek before stream start",
                    ));
                }
                end as u64
            }
        };

        self.pos = new_pos;
        Ok(self.pos)
    }
}

pub struct StreamHandle {
    pub reader: Option<BufReader<ProgressiveReader>>,
    pub coord: Arc<StreamCoord>,
    pub cache_path: PathBuf,
}

pub fn start_stream(
    agent: &ureq::Agent,
    url: &str,
    total_size: u64,
    cache_path: PathBuf,
    cancelled: Arc<AtomicBool>,
    on_buffer: impl Fn(u64, u64) + Send + 'static,
) -> Result<StreamHandle> {
    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let coord = Arc::new(StreamCoord::new(total_size));
    let coord_worker = coord.clone();
    let cancelled_worker = cancelled.clone();

    // Make initial GET request
    let response = agent
        .get(url)
        .set("User-Agent", "Mozilla/4.0 (compatible; MSIE 6.0; Windows NT 5.0; .NET CLR 1.1.4322)")
        .call()
        .map_err(|e| Error::coded("network", format!("网络连接失败: {e}")))?;

    let status = response.status();
    if status != 200 && status != 206 {
        return Err(Error::coded("network", format!("HTTP 请求失败 ({status})")));
    }

    let actual_total = response
        .header("Content-Length")
        .and_then(|h| h.parse::<u64>().ok())
        .unwrap_or(total_size);

    if actual_total > 0 {
        coord.total.store(actual_total, Ordering::Relaxed);
    }

    let cache_worker_path = cache_path.clone();

    // Spawn download thread
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let ready_flag = Arc::new(AtomicBool::new(false));
    let ready_flag_worker = ready_flag.clone();

    thread::spawn(move || {
        let mut file = match OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&cache_worker_path)
        {
            Ok(f) => f,
            Err(e) => {
                coord_worker.mark_failed();
                let _ = ready_tx.send(Err(Error::new(format!("无法创建缓存文件: {e}"))));
                return;
            }
        };

        let mut reader = response.into_reader();
        let mut buffer = [0u8; 32 * 1024];
        let mut downloaded: u64 = 0;
        let prebuffer_target = if actual_total > 0 {
            PREBUFFER_SIZE.min(actual_total)
        } else {
            PREBUFFER_SIZE
        };

        loop {
            if cancelled_worker.load(Ordering::Relaxed) || coord_worker.cancelled.load(Ordering::Relaxed) {
                coord_worker.mark_cancelled();
                let _ = std::fs::remove_file(&cache_worker_path);
                return;
            }

            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = file.flush();
                    coord_worker.mark_completed(downloaded);
                    if !ready_flag_worker.load(Ordering::Relaxed) {
                        ready_flag_worker.store(true, Ordering::Relaxed);
                        let _ = ready_tx.send(Ok(()));
                    }
                    on_buffer(downloaded, downloaded);
                    break;
                }
                Ok(n) => {
                    if file.write_all(&buffer[..n]).is_err() {
                        coord_worker.mark_failed();
                        if !ready_flag_worker.load(Ordering::Relaxed) {
                            let _ = ready_tx.send(Err(Error::new("写入缓存失败")));
                        }
                        return;
                    }
                    downloaded += n as u64;
                    coord_worker.notify_progress(downloaded);
                    on_buffer(downloaded, actual_total);

                    // Once prebuffer threshold is met, notify the player to start immediately!
                    if !ready_flag_worker.load(Ordering::Relaxed) && downloaded >= prebuffer_target {
                        ready_flag_worker.store(true, Ordering::Relaxed);
                        let _ = file.flush();
                        let _ = ready_tx.send(Ok(()));
                    }
                }
                Err(e) => {
                    coord_worker.mark_failed();
                    if !ready_flag_worker.load(Ordering::Relaxed) {
                        let _ = ready_tx.send(Err(Error::coded("network", format!("数据流读取中断: {e}"))));
                    }
                    return;
                }
            }
        }
    });

    // Wait until pre-buffer threshold is achieved (typically ~100-250ms)
    match ready_rx.recv_timeout(Duration::from_secs(12)) {
        Ok(Ok(())) => {
            let read_file = File::open(&cache_path)?;
            let progressive = ProgressiveReader::new(read_file, coord.clone());
            Ok(StreamHandle {
                reader: Some(BufReader::new(progressive)),
                coord,
                cache_path,
            })
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            coord.mark_cancelled();
            Err(Error::coded("network", "预缓冲超时，网络连接缓慢"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_progressive_reader_reads_chunks_and_waits_for_completion() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_stream.bin");

        let coord = Arc::new(StreamCoord::new(100));
        let mut write_file = File::create(&file_path).unwrap();

        // Write first 40 bytes
        let chunk1 = vec![1u8; 40];
        write_file.write_all(&chunk1).unwrap();
        write_file.flush().unwrap();
        coord.notify_progress(40);

        let read_file = File::open(&file_path).unwrap();
        let mut reader = ProgressiveReader::new(read_file, coord.clone());

        let mut buf = [0u8; 30];
        let n1 = reader.read(&mut buf).unwrap();
        assert_eq!(n1, 30);
        assert_eq!(&buf[..n1], &chunk1[..30]);

        // Write remaining 60 bytes asynchronously
        let coord_clone = coord.clone();
        let file_path_clone = file_path.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            let mut f = OpenOptions::new().append(true).open(&file_path_clone).unwrap();
            let chunk2 = vec![2u8; 60];
            f.write_all(&chunk2).unwrap();
            f.flush().unwrap();
            coord_clone.mark_completed(100);
        });

        // Next read takes the remaining 10 bytes of chunk 1
        let mut buf2 = [0u8; 20];
        let n2 = reader.read(&mut buf2).unwrap();
        assert_eq!(n2, 10);
        assert_eq!(&buf2[..n2], &chunk1[30..40]);

        // Next read waits and receives chunk 2
        let mut buf3 = [0u8; 70];
        let n3 = reader.read(&mut buf3).unwrap();
        assert_eq!(n3, 60);
        assert_eq!(&buf3[..n3], &vec![2u8; 60][..]);

        // Next read returns EOF (0)
        let n4 = reader.read(&mut buf3).unwrap();
        assert_eq!(n4, 0);
    }

    #[test]
    fn test_progressive_reader_seek() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_seek.bin");
        let coord = Arc::new(StreamCoord::new(1000));
        let file = File::create(&file_path).unwrap();
        let mut reader = ProgressiveReader::new(file, coord.clone());

        assert_eq!(reader.seek(SeekFrom::Start(50)).unwrap(), 50);
        assert_eq!(reader.seek(SeekFrom::Current(20)).unwrap(), 70);
        assert_eq!(reader.seek(SeekFrom::Current(-10)).unwrap(), 60);
        assert_eq!(reader.seek(SeekFrom::End(-50)).unwrap(), 950);
    }
}
