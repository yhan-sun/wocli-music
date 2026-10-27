//! Only this worker may decode/read files. The device callback consumes a bounded,
//! wait-free PCM ring and never waits for a network request or condition variable.
use crate::model::{Error, Result};
use rodio::Source;
use rtrb::{Consumer, RingBuffer};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct BufferState {
    pub samples: Arc<AtomicU64>,
    pub produced: AtomicU64,
    pub buffering: AtomicBool,
    pub underruns: AtomicU64,
    pub done: AtomicBool,
    pub format_error: AtomicBool,
}
pub(super) struct BufferedSource {
    consumer: Consumer<f32>,
    state: Arc<BufferState>,
    cancelled: Arc<AtomicBool>,
    channels: u16,
    rate: u32,
    duration: Option<Duration>,
    frame_remaining: usize,
    silent_remaining: usize,
    rebuffer_target: usize,
    starved: bool,
}
impl BufferedSource {
    pub fn prepare(
        mut source: Box<dyn Source<Item = f32> + Send>,
        samples: Arc<AtomicU64>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(Self, Arc<BufferState>)> {
        let channels = source.channels();
        let rate = source.sample_rate();
        let duration = source.total_duration();
        if channels == 0 || rate == 0 {
            return Err(Error::coded("decode_error", "音频采样率或声道数无效"));
        }
        let per_second = rate as usize * channels as usize;
        let capacity = (per_second.saturating_mul(4)).clamp(4096, 2 * 1024 * 1024);
        let capacity = capacity - capacity % channels as usize;
        let target = per_second.min(capacity / 2).max(channels as usize);
        let (mut producer, consumer) = RingBuffer::new(capacity);
        let state = Arc::new(BufferState {
            samples,
            ..BufferState::default()
        });
        let worker = state.clone();
        let cancel = cancelled.clone();
        thread::Builder::new()
            .name("wocli-pcm-decoder".into())
            .spawn(move || {
                'decode: loop {
                    if cancel.load(Ordering::Relaxed) || producer.is_abandoned() {
                        break;
                    }
                    let slots = producer.slots();
                    if slots == 0 {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    let mut produced = 0;
                    for _ in 0..slots.min(2048) {
                        if cancel.load(Ordering::Relaxed) {
                            break 'decode;
                        }
                        let Some(sample) = source.next() else {
                            worker.produced.fetch_add(produced, Ordering::Release);
                            break 'decode;
                        };
                        if source.channels() != channels || source.sample_rate() != rate {
                            worker.format_error.store(true, Ordering::Release);
                            worker.produced.fetch_add(produced, Ordering::Release);
                            break 'decode;
                        }
                        // One producer: the consumer can only increase the free space.
                        if producer.push(sample).is_err() {
                            break 'decode;
                        }
                        produced += 1;
                    }
                    worker.produced.fetch_add(produced, Ordering::Release);
                }
                worker.done.store(true, Ordering::Release);
            })
            .map_err(|e| Error::new(format!("无法启动音频解码线程：{e}")))?;
        let deadline = Instant::now() + Duration::from_secs(6);
        while consumer.slots() < target
            && !state.done.load(Ordering::Acquire)
            && Instant::now() < deadline
        {
            if cancelled.load(Ordering::Relaxed) {
                return Err(Error::coded("cancelled", "已取消"));
            }
            thread::sleep(Duration::from_millis(2));
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err(Error::coded("cancelled", "已取消"));
        }
        Ok((
            Self {
                consumer,
                state: state.clone(),
                cancelled,
                channels,
                rate,
                duration,
                frame_remaining: 0,
                silent_remaining: 0,
                rebuffer_target: target,
                starved: false,
            },
            state,
        ))
    }
    fn silent_frame(&mut self) -> Option<f32> {
        self.silent_remaining = self.channels as usize - 1;
        Some(0.0)
    }
}
impl Iterator for BufferedSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.cancelled.load(Ordering::Relaxed) {
            return None;
        }
        if self.silent_remaining > 0 {
            self.silent_remaining -= 1;
            return Some(0.0);
        }
        if self.frame_remaining == 0 {
            let available = self.consumer.slots();
            let done = self.state.done.load(Ordering::Acquire);
            if self.starved && available < self.rebuffer_target && !done {
                return self.silent_frame();
            }
            if available < self.channels as usize {
                // Completion can be published after the first available-data snapshot.
                if done && self.consumer.slots() < self.channels as usize {
                    self.state.buffering.store(false, Ordering::Release);
                    return None;
                }
                if !self.starved {
                    self.starved = true;
                    self.state.buffering.store(true, Ordering::Release);
                    self.state.underruns.fetch_add(1, Ordering::Relaxed);
                }
                return self.silent_frame();
            }
            self.starved = false;
            self.state.buffering.store(false, Ordering::Release);
            self.frame_remaining = self.channels as usize;
        }
        // A complete frame was checked above; the producer never removes samples.
        let sample = self.consumer.pop().ok()?;
        self.frame_remaining -= 1;
        self.state.samples.fetch_add(1, Ordering::Relaxed);
        Some(sample)
    }
}
impl Source for BufferedSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }
}
impl Drop for BufferedSource {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Gated {
        position: usize,
        limit: usize,
        pause_at: usize,
        gate: Arc<AtomicBool>,
        blocked: Arc<AtomicBool>,
        cancel: Arc<AtomicBool>,
    }
    impl Iterator for Gated {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            assert_eq!(thread::current().name(), Some("wocli-pcm-decoder"));
            if self.position == self.pause_at {
                self.blocked.store(true, Ordering::Release);
                while !self.gate.load(Ordering::Acquire) && !self.cancel.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(2));
                }
            }
            if self.cancel.load(Ordering::Relaxed) || self.position == self.limit {
                return None;
            }
            self.position += 1;
            Some(self.position as f32)
        }
    }
    impl Source for Gated {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> u16 {
            2
        }
        fn sample_rate(&self) -> u32 {
            1000
        }
        fn total_duration(&self) -> Option<Duration> {
            Some(Duration::from_secs_f64(self.limit as f64 / 2000.0))
        }
    }
    fn wait(condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "PCM worker did not reach expected state"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn a_stalled_decoder_never_blocks_output_and_resumes_without_losing_stereo_alignment() {
        let gate = Arc::new(AtomicBool::new(false));
        let blocked = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(AtomicU64::new(0));
        let source = Gated {
            position: 0,
            limit: 20000,
            pause_at: 4001,
            gate: gate.clone(),
            blocked: blocked.clone(),
            cancel: cancel.clone(),
        };
        let (mut source, state) =
            BufferedSource::prepare(Box::new(source), samples.clone(), cancel.clone()).unwrap();
        wait(|| blocked.load(Ordering::Acquire));
        for n in 1..=4000 {
            assert_eq!(source.next(), Some(n as f32));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let callback = thread::spawn(move || {
            for _ in 0..4000 {
                assert_eq!(source.next(), Some(0.0));
            }
            let _ = tx.send(source);
        });
        let result = rx.recv_timeout(Duration::from_millis(300));
        if result.is_err() {
            cancel.store(true, Ordering::Relaxed);
            gate.store(true, Ordering::Release);
        }
        let mut source = result.expect("device callback waited for the stalled decoder");
        callback.join().unwrap();
        assert_eq!(samples.load(Ordering::Relaxed), 4000);
        assert!(state.buffering.load(Ordering::Acquire));
        assert_eq!(state.underruns.load(Ordering::Relaxed), 1);
        gate.store(true, Ordering::Release);
        wait(|| source.consumer.slots() >= source.rebuffer_target);
        assert_eq!(source.next(), Some(4001.0));
        assert_eq!(source.next(), Some(4002.0));
        assert!(!state.buffering.load(Ordering::Acquire));
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(source.next(), None);
        wait(|| state.done.load(Ordering::Acquire));
    }
    #[test]
    fn decode_ahead_is_bounded_and_dropping_output_cancels_a_blocked_worker() {
        let gate = Arc::new(AtomicBool::new(false));
        let blocked = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let source = Gated {
            position: 0,
            limit: 100000,
            pause_at: 4000,
            gate,
            blocked: blocked.clone(),
            cancel: cancel.clone(),
        };
        let (source, state) = BufferedSource::prepare(
            Box::new(source),
            Arc::new(AtomicU64::new(0)),
            cancel.clone(),
        )
        .unwrap();
        wait(|| blocked.load(Ordering::Acquire));
        drop(source);
        assert!(cancel.load(Ordering::Relaxed));
        wait(|| state.done.load(Ordering::Acquire));
        let cancel = Arc::new(AtomicBool::new(false));
        let source = Gated {
            position: 0,
            limit: 100000,
            pause_at: usize::MAX,
            gate: Arc::new(AtomicBool::new(true)),
            blocked: Arc::new(AtomicBool::new(false)),
            cancel: cancel.clone(),
        };
        let (source, state) = BufferedSource::prepare(
            Box::new(source),
            Arc::new(AtomicU64::new(0)),
            cancel.clone(),
        )
        .unwrap();
        wait(|| source.consumer.slots() == 8000 && state.produced.load(Ordering::Acquire) == 8000);
        assert_eq!(state.produced.load(Ordering::Acquire), 8000);
        thread::sleep(Duration::from_millis(20));
        assert_eq!(state.produced.load(Ordering::Acquire), 8000);
        drop(source);
        wait(|| state.done.load(Ordering::Acquire));
    }
    #[test]
    fn short_files_end_without_padding_or_startup_wait() {
        let cancel = Arc::new(AtomicBool::new(false));
        let source = Gated {
            position: 0,
            limit: 16,
            pause_at: usize::MAX,
            gate: Arc::new(AtomicBool::new(true)),
            blocked: Arc::new(AtomicBool::new(false)),
            cancel: cancel.clone(),
        };
        let start = Instant::now();
        let (mut source, state) =
            BufferedSource::prepare(Box::new(source), Arc::new(AtomicU64::new(0)), cancel).unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(
            source.by_ref().collect::<Vec<_>>(),
            (1..=16).map(|n| n as f32).collect::<Vec<_>>()
        );
        assert_eq!(state.underruns.load(Ordering::Relaxed), 0);
    }
}
