use crate::model::{Error, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

pub const PREBUFFER_SIZE: u64 = 256 * 1024;
// ureq body reads can stay blocked until the socket deadline after cancellation.
// Bound those outstanding reads independently from the service worker pool.
static STREAM_CONNECTIONS: OnceLock<crate::pool::ConnectionLimiter> = OnceLock::new();
const MAX_STREAM_CONNECTIONS: usize = 8;

// Compressed bytes must cover time, not just a fixed size: 256 KiB can be less
// than two seconds of FLAC. Keep startup reserves bounded even for unusual files.
pub(crate) fn prebuffer_size(total: u64, duration: f64) -> u64 {
    let timed = if duration.is_finite() && duration > 0.0 {
        (total as f64 / duration * 8.0) as u64
    } else {
        0
    };
    PREBUFFER_SIZE
        .max(timed)
        .min(4 * 1024 * 1024)
        .min(if total > 0 { total } else { u64::MAX })
}

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
    cancelled: Arc<AtomicBool>,
    disk_pos: Option<u64>,
}

impl ProgressiveReader {
    pub fn new(file: File, coord: Arc<StreamCoord>) -> Self {
        Self::with_cancel(file, coord, Arc::new(AtomicBool::new(false)))
    }
    pub fn with_cancel(file: File, coord: Arc<StreamCoord>, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            file,
            pos: 0,
            coord,
            cancelled,
            disk_pos: None,
        }
    }
}

impl Read for ProgressiveReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.coord.cancelled.load(Ordering::Relaxed)
                || self.cancelled.load(Ordering::Relaxed)
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "stream cancelled",
                ));
            }
            if self.coord.failed.load(Ordering::Relaxed) {
                return Err(std::io::Error::other("stream download failed"));
            }

            let written = self.coord.written.load(Ordering::Acquire);
            if self.pos < written {
                let available = (written - self.pos).min(buf.len() as u64) as usize;
                if self.disk_pos != Some(self.pos) {
                    self.file.seek(SeekFrom::Start(self.pos))?;
                }
                let n = self.file.read(&mut buf[..available])?;
                self.pos += n as u64;
                self.disk_pos = Some(self.pos);
                return Ok(n);
            }

            if self.coord.completed.load(Ordering::Acquire) {
                // Completion may have published another chunk after our first snapshot.
                if self.pos < self.coord.written.load(Ordering::Acquire) {
                    continue;
                }
                return Ok(0);
            }

            // Wait for more bytes to be downloaded
            let (lock, cvar) = &self.coord.notifier;
            let guard = lock.lock().unwrap();
            // Check again under the notifier lock so a just-published chunk
            // cannot lose its wakeup and add an unnecessary 50ms audio gap.
            if self.pos < self.coord.written.load(Ordering::Acquire)
                || self.coord.completed.load(Ordering::Acquire)
                || self.coord.failed.load(Ordering::Acquire)
                || self.coord.cancelled.load(Ordering::Relaxed)
                || self.cancelled.load(Ordering::Relaxed)
            {
                continue;
            }
            let _ = cvar.wait_timeout(guard, Duration::from_millis(50));
        }
    }
}

impl Seek for ProgressiveReader {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let total = self.coord.total.load(Ordering::Relaxed);
        let new_pos = match from {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => {
                let cur = self.pos.checked_add_signed(offset);
                if cur.is_none() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "negative seek position",
                    ));
                }
                cur.unwrap()
            }
            SeekFrom::End(offset) => {
                let base = if total > 0 {
                    total
                } else {
                    self.coord.written.load(Ordering::Relaxed)
                };
                let end = base.checked_add_signed(offset);
                if end.is_none() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "seek before stream start",
                    ));
                }
                end.unwrap()
            }
        };

        self.pos = new_pos;
        Ok(self.pos)
    }
}

pub struct StreamHandle {
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
    on_complete: impl Fn(Result<u64>) + Send + 'static,
) -> Result<StreamHandle> {
    start_stream_buffered(
        agent,
        url,
        total_size,
        0.0,
        cache_path,
        cancelled,
        (on_buffer, on_complete),
    )
}

pub(crate) fn start_stream_buffered(
    agent: &ureq::Agent,
    url: &str,
    total_size: u64,
    duration: f64,
    cache_path: PathBuf,
    cancelled: Arc<AtomicBool>,
    callbacks: (
        impl Fn(u64, u64) + Send + 'static,
        impl Fn(Result<u64>) + Send + 'static,
    ),
) -> Result<StreamHandle> {
    let (on_buffer, on_complete) = callbacks;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use md5::{Digest, Md5};
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::coded("cancelled", "已取消"));
    }
    let connection = STREAM_CONNECTIONS
        .get_or_init(|| crate::pool::ConnectionLimiter::new(MAX_STREAM_CONNECTIONS))
        .acquire()
        .ok_or_else(|| Error::coded("worker_busy", "音频连接繁忙，请稍后重试"))?;
    // Keep failed/slow streaming attempts separate from the full-download fallback.
    let cache_path = cache_path.with_extension(format!(
        "{}.stream",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let response = agent
        .get(url)
        .timeout(Duration::from_secs(120))
        .set(
            "User-Agent",
            "Mozilla/4.0 (compatible; MSIE 6.0; Windows NT 5.0; .NET CLR 1.1.4322)",
        )
        .call()
        .map_err(|e| Error::coded("network", format!("网络连接失败: {e}")))?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::coded("cancelled", "已取消"));
    }
    let status = response.status();
    if status != 200 && status != 206 {
        return Err(Error::coded("network", format!("HTTP 请求失败 ({status})")));
    }
    let content_length = response
        .header("Content-Length")
        .and_then(|h| h.parse::<u64>().ok());
    let range_total = if status == 206 {
        let (start, end, total) =
            crate::provider::kuwo::range(response.header("Content-Range").unwrap_or_default())
                .ok_or_else(|| Error::coded("incomplete_audio", "服务器返回错误分块"))?;
        if start != 0 || end.checked_add(1) != Some(total) {
            return Err(Error::coded("incomplete_audio", "服务器仅返回部分音频"));
        }
        Some(total)
    } else {
        None
    };
    if range_total.is_some_and(|total| content_length.is_some_and(|len| len != total)) {
        return Err(Error::coded("incomplete_audio", "服务器返回错误分块长度"));
    }
    let actual_total = range_total.or(content_length).unwrap_or(total_size);
    if total_size > 0 && actual_total > 0 && (actual_total as f64) < total_size as f64 * 0.8 {
        return Err(Error::coded(
            "incomplete_audio",
            "服务器返回试听或不完整音频",
        ));
    }
    let expected_md5 = response
        .header("X-COS-META-MD5")
        .or_else(|| response.header("Content-MD5"))
        .map(str::to_string);
    let coord = Arc::new(StreamCoord::new(actual_total));
    let coord_worker = coord.clone();
    let cache_worker_path = cache_path.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let cancel_wait = cancelled.clone();
    thread::Builder::new()
        .name("wocli-stream-download".into())
        .spawn(move || {
            let _connection = connection;
            let mut ready = false;
            let result = (|| {
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&cache_worker_path)?;
                let mut reader = response.into_reader();
                let mut buffer = [0u8; 32 * 1024];
                let mut head = Vec::new();
                let mut digest = Md5::new();
                let mut downloaded = 0u64;
                let prebuffer_target = prebuffer_size(actual_total, duration);
                loop {
                    if cancelled.load(Ordering::Relaxed)
                        || coord_worker.cancelled.load(Ordering::Relaxed)
                    {
                        return Err(Error::coded("cancelled", "已取消"));
                    }
                    let n = reader
                        .read(&mut buffer)
                        .map_err(|e| Error::coded("network", format!("数据流读取中断: {e}")))?;
                    if n == 0 {
                        break;
                    }
                    if head.len() < 4 {
                        head.extend_from_slice(&buffer[..n.min(4 - head.len())]);
                        if head.len() == 4 && !crate::provider::kuwo::valid_audio(&head) {
                            return Err(Error::coded("invalid_audio", "返回内容不是有效音频"));
                        }
                    }
                    downloaded += n as u64;
                    if downloaded > 1024 * 1024 * 1024 {
                        return Err(Error::coded("invalid_audio", "音频长度异常"));
                    }
                    file.write_all(&buffer[..n])?;
                    digest.update(&buffer[..n]);
                    coord_worker.notify_progress(downloaded);
                    on_buffer(downloaded, actual_total);
                    if !ready && downloaded >= prebuffer_target {
                        ready = true;
                        let _ = ready_tx.send(Ok(()));
                    }
                }
                if cancelled.load(Ordering::Relaxed)
                    || coord_worker.cancelled.load(Ordering::Relaxed)
                {
                    return Err(Error::coded("cancelled", "已取消"));
                }
                if head.len() < 4 || !crate::provider::kuwo::valid_audio(&head) {
                    return Err(Error::coded("invalid_audio", "返回内容不是有效音频"));
                }
                if (actual_total > 0 && downloaded != actual_total)
                    || (total_size > 0 && (downloaded as f64) < total_size as f64 * 0.8)
                {
                    return Err(Error::coded("incomplete_audio", "音频传输长度不完整"));
                }
                if let Some(expected) = expected_md5 {
                    let digest = digest.finalize();
                    if expected.trim_matches('"').to_lowercase() != format!("{digest:x}")
                        && expected.trim_matches('"') != STANDARD.encode(digest)
                    {
                        return Err(Error::coded("incomplete_audio", "音频 MD5 校验失败"));
                    }
                }
                file.flush()?;
                coord_worker.total.store(downloaded, Ordering::Release);
                coord_worker.mark_completed(downloaded);
                on_buffer(downloaded, downloaded);
                Ok(downloaded)
            })();
            match result {
                Ok(n) => {
                    if !ready {
                        let _ = ready_tx.send(Ok(()));
                    }
                    on_complete(Ok(n));
                }
                Err(error) => {
                    if error.code == "cancelled" {
                        coord_worker.mark_cancelled();
                    } else {
                        coord_worker.mark_failed();
                    }
                    let _ = std::fs::remove_file(cache_worker_path);
                    if ready {
                        on_complete(Err(error));
                    } else {
                        let _ = ready_tx.send(Err(error));
                    }
                }
            }
        })
        .map_err(|e| Error::new(format!("无法启动音频下载线程：{e}")))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    loop {
        if cancel_wait.load(Ordering::Relaxed) {
            coord.mark_cancelled();
            let _ = std::fs::remove_file(&cache_path);
            return Err(Error::coded("cancelled", "已取消"));
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            coord.mark_cancelled();
            let _ = std::fs::remove_file(&cache_path);
            return Err(Error::coded("network", "预缓冲超时，网络连接缓慢"));
        }
        match ready_rx.recv_timeout(remaining.min(Duration::from_millis(50))) {
            Ok(Ok(())) if !cancel_wait.load(Ordering::Relaxed) => {
                return Ok(StreamHandle { coord, cache_path });
            }
            Ok(Ok(())) => continue,
            Ok(Err(error)) => return Err(error),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                coord.mark_failed();
                let _ = std::fs::remove_file(&cache_path);
                return Err(Error::coded("network", "音频下载线程意外结束"));
            }
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
            let mut f = OpenOptions::new()
                .append(true)
                .open(&file_path_clone)
                .unwrap();
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

    #[test]
    fn test_decoder_progressive_prebuffer() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("prebuffer.wav");

        // Generate a 2-second 8000Hz 16-bit mono WAV (16044 bytes total)
        let samples: u32 = 8000 * 2;
        let mut wav = vec![];
        wav.extend(b"RIFF");
        wav.extend((36 + samples * 2).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes()); // PCM
        wav.extend(1u16.to_le_bytes()); // mono
        wav.extend(8000u32.to_le_bytes()); // rate
        wav.extend(16000u32.to_le_bytes()); // byte rate
        wav.extend(2u16.to_le_bytes()); // block align
        wav.extend(16u16.to_le_bytes()); // bits per sample
        wav.extend(b"data");
        wav.extend((samples * 2).to_le_bytes());
        wav.resize(44 + samples as usize * 2, 0);

        let total = wav.len() as u64;
        let prebuffer_len = 4000; // write initial prebuffer portion

        let mut write_file = File::create(&file_path).unwrap();
        write_file.write_all(&wav[..prebuffer_len]).unwrap();
        write_file.flush().unwrap();

        let coord = Arc::new(StreamCoord::new(total));
        coord.notify_progress(prebuffer_len as u64);

        let read_file = File::open(&file_path).unwrap();
        let reader = std::io::BufReader::new(ProgressiveReader::new(read_file, coord.clone()));

        // Writer thread simulates streaming remainder
        let coord_worker = coord.clone();
        let file_path_worker = file_path.clone();
        let wav_worker = wav.clone();
        thread::spawn(move || {
            let mut curr = prebuffer_len;
            while curr < wav_worker.len() {
                thread::sleep(Duration::from_millis(20));
                let next = (curr + 4000).min(wav_worker.len());
                let mut f = OpenOptions::new()
                    .append(true)
                    .open(&file_path_worker)
                    .unwrap();
                f.write_all(&wav_worker[curr..next]).unwrap();
                f.flush().unwrap();
                curr = next;
                coord_worker.notify_progress(curr as u64);
            }
            coord_worker.mark_completed(wav_worker.len() as u64);
        });

        let decoder = rodio::Decoder::try_from(reader).unwrap();
        let mut samples_read = 0;
        for _s in decoder {
            samples_read += 1;
        }
        assert_eq!(samples_read, samples);
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use crate::player::test_wav;
    use std::{
        net::{TcpListener, TcpStream},
        sync::mpsc,
        time::Instant,
    };

    fn serve(handler: impl FnOnce(TcpStream) + Send + 'static) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buf = [0; 4096];
            assert!(socket.read(&mut buf).unwrap() > 0);
            handler(socket);
        });
        (url, worker)
    }
    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new().try_proxy_from_env(false).build()
    }

    #[test]
    fn sequential_tail_waits_for_real_eof_and_cancellation_is_not_retryable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tail.wav");
        let wav = test_wav(10);
        let written = wav.len() - 8192;
        fs_write(&path, &wav[..written]);
        let coord = Arc::new(StreamCoord::new(wav.len() as u64));
        coord.notify_progress(written as u64);
        let worker_coord = coord.clone();
        let worker_path = path.clone();
        let worker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            let mut file = OpenOptions::new().append(true).open(worker_path).unwrap();
            file.write_all(&wav[written..]).unwrap();
            worker_coord.mark_completed(wav.len() as u64);
        });
        let decoder = rodio::Decoder::try_from(std::io::BufReader::new(ProgressiveReader::new(
            File::open(&path).unwrap(),
            coord.clone(),
        )))
        .unwrap();
        assert_eq!(decoder.count(), 80000);
        worker.join().unwrap();
        coord.mark_cancelled();
        let mut reader = ProgressiveReader::new(File::open(path).unwrap(), coord);
        assert_eq!(
            reader.read(&mut [0; 4]).unwrap_err().kind(),
            std::io::ErrorKind::ConnectionAborted
        );
    }
    fn fs_write(path: &std::path::Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn inherited_short_request_deadline_does_not_cut_off_streaming() {
        let dir = tempfile::tempdir().unwrap();
        let body = test_wav(30);
        let total = body.len();
        let (url, server) = serve(move |mut s| {
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            s.write_all(&body[..300000]).unwrap();
            thread::sleep(Duration::from_millis(150));
            s.write_all(&body[300000..]).unwrap();
        });
        let (tx, rx) = mpsc::channel();
        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .timeout(Duration::from_millis(80))
            .build();
        let handle = start_stream(
            &agent,
            &url,
            total as u64,
            dir.path().join("cache"),
            Arc::new(AtomicBool::new(false)),
            |_, _| {},
            move |r| {
                let _ = tx.send(r);
            },
        )
        .unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap(),
            total as u64
        );
        assert!(handle.coord.completed.load(Ordering::Acquire));
        server.join().unwrap();
    }
    #[test]
    fn cancelling_prebuffer_releases_the_caller_and_cleans_the_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let body = test_wav(30);
        let (sent_tx, sent_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (url, server) = serve(move |mut s| {
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            s.write_all(&body[..40000]).unwrap();
            sent_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
        });
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_worker = cancelled.clone();
        let path = dir.path().join("cancel-prebuffer");
        let (tx, rx) = mpsc::channel();
        let caller = thread::spawn(move || {
            let result = start_stream(&agent(), &url, 0, path, cancel_worker, |_, _| {}, |_| {});
            let _ = tx.send(result.err().map(|error| error.code));
        });
        sent_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while std::fs::read_dir(dir.path()).unwrap().count() == 0 {
            assert!(
                Instant::now() < deadline,
                "download did not create its partial file"
            );
            thread::sleep(Duration::from_millis(2));
        }
        cancelled.store(true, Ordering::Relaxed);
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(500)).unwrap(),
            Some("cancelled".into())
        );
        // The HTTP reader is still blocked; waking it must finish cleanup and
        // release its cancellation token, without reporting a successful stream.
        release_tx.send(()).unwrap();
        server.join().unwrap();
        caller.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while Arc::strong_count(&cancelled) != 1 {
            assert!(
                Instant::now() < deadline,
                "stream worker retained its cancellation token"
            );
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[test]
    fn previews_and_partial_http_responses_are_rejected() {
        for partial in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let body = test_wav(2);
            let (url, server) = serve(move |mut s| {
                let status = if partial {
                    "206 Partial Content"
                } else {
                    "200 OK"
                };
                let range = if partial {
                    "Content-Range: bytes 0-32043/1000000\r\n"
                } else {
                    ""
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 {status}\r\n{range}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(&body);
            });
            let result = start_stream(
                &agent(),
                &url,
                1_000_000,
                dir.path().join("cache"),
                Arc::new(AtomicBool::new(false)),
                |_, _| {},
                |_| {},
            );
            assert_eq!(result.err().unwrap().code, "incomplete_audio");
            server.join().unwrap();
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
    #[test]
    fn prebuffer_failure_does_not_block_full_download_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("cache");
        let (url, server) = serve(|mut s| {
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nwrong")
                .unwrap();
        });
        assert!(start_stream(
            &agent(),
            &url,
            0,
            target.clone(),
            Arc::new(AtomicBool::new(false)),
            |_, _| {},
            |_| {}
        )
        .is_err());
        server.join().unwrap();
        assert!(!target.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let body = test_wav(2);
        let total = body.len();
        let (url, server) = serve(move |mut s| {
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            s.write_all(&body).unwrap();
        });
        let stream = crate::provider::Stream {
            url,
            size: total as u64,
            quality: "320k".into(),
            extension: "wav".into(),
        };
        assert_eq!(
            crate::provider::KuwoProvider::new(true)
                .download(&stream, &target, false, &AtomicBool::new(false), |_, _| {})
                .unwrap(),
            total as u64
        );
        server.join().unwrap();
        assert_eq!(std::fs::metadata(target).unwrap().len(), total as u64);
    }
    #[test]
    fn errors_after_prebuffer_are_reported_and_not_marked_complete() {
        for checksum in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let body = test_wav(30);
            let (url, server) = serve(move |mut s| {
                let extra = if checksum {
                    "Content-MD5: wrong\r\n"
                } else {
                    ""
                };
                let len = body.len() + if checksum { 0 } else { 1000 };
                write!(
                    s,
                    "HTTP/1.1 200 OK\r\n{extra}Content-Length: {len}\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                s.write_all(&body[..300000]).unwrap();
                thread::sleep(Duration::from_millis(100));
                if checksum {
                    s.write_all(&body[300000..]).unwrap();
                }
            });
            let (tx, rx) = mpsc::channel();
            let handle = start_stream(
                &agent(),
                &url,
                0,
                dir.path().join("cache"),
                Arc::new(AtomicBool::new(false)),
                |_, _| {},
                move |r| {
                    let _ = tx.send(r);
                },
            )
            .unwrap();
            assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap().is_err());
            assert!(handle.coord.failed.load(Ordering::Acquire));
            assert!(!handle.coord.completed.load(Ordering::Acquire));
            server.join().unwrap();
        }
    }
    #[test]
    fn stopping_a_starved_decoder_releases_its_worker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel.wav");
        let bytes = test_wav(30);
        fs_write(&path, &bytes[..40000]);
        let coord = Arc::new(StreamCoord::new(bytes.len() as u64));
        coord.notify_progress(40000);
        let cancelled = Arc::new(AtomicBool::new(false));
        let reader =
            ProgressiveReader::with_cancel(File::open(path).unwrap(), coord, cancelled.clone());
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let decoder = rodio::Decoder::try_from(std::io::BufReader::new(reader)).unwrap();
            let _ = tx.send(decoder.count());
        });
        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        let start = Instant::now();
        cancelled.store(true, Ordering::Relaxed);
        assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap() < 240000);
        assert!(start.elapsed() < Duration::from_secs(1));
        worker.join().unwrap();
    }
}

#[cfg(test)]
mod buffer_sizing_tests {
    use super::*;
    #[test]
    fn reserves_time_for_high_bitrate_audio_with_a_fixed_memory_bound() {
        assert_eq!(prebuffer_size(30 * 1024 * 1024, 240.0), 1024 * 1024);
        assert_eq!(prebuffer_size(4 * 1024 * 1024, 240.0), PREBUFFER_SIZE);
        assert_eq!(prebuffer_size(10 * 1024 * 1024, 1.0), 4 * 1024 * 1024);
        assert_eq!(prebuffer_size(10000, 5.0), 10000);
        assert_eq!(prebuffer_size(0, 0.0), PREBUFFER_SIZE);
        assert_eq!(prebuffer_size(0, f64::NAN), PREBUFFER_SIZE);
    }
}
