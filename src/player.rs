use crate::{
    model::{Error, Result, Track},
    stream::{ProgressiveReader, StreamCoord},
};
use rodio::{OutputStream, Sink, Source};
use std::{
    fs::File,
    io::{BufReader, Read, Seek},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct AudioInput {
    pub path: PathBuf,
    pub hint: String,
    pub stream: Option<Arc<StreamCoord>>,
}
trait AudioReader: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> AudioReader for T {}

/// Decoding and seeking can wait for network data. Prepare them on a worker,
/// before handing the source to the audio output or the service event loop.
pub struct PreparedAudio {
    source: Box<dyn Source<Item = f32> + Send>,
    duration: f64,
    position: f64,
    cancelled: Arc<AtomicBool>,
    samples: Arc<AtomicU64>,
    samples_per_second: f64,
}
struct PrimedSource {
    source: Box<dyn Source<Item = f32> + Send>,
    first: Option<f32>,
    samples: Arc<AtomicU64>,
}
impl Iterator for PrimedSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sample = self.first.take().or_else(|| self.source.next());
        if sample.is_some() {
            self.samples.fetch_add(1, Ordering::Relaxed);
        }
        sample
    }
}
impl Source for PrimedSource {
    fn current_span_len(&self) -> Option<usize> {
        self.source.current_span_len()
    }
    fn channels(&self) -> u16 {
        self.source.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.source.total_duration()
    }
}
impl AudioInput {
    pub fn prepare(
        &self,
        track: &Track,
        position: f64,
        cancelled: Arc<AtomicBool>,
        null: bool,
    ) -> Result<PreparedAudio> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(Error::coded("cancelled", "已取消"));
        }
        let file = File::open(&self.path)?;
        let complete = self
            .stream
            .as_ref()
            .is_none_or(|c| c.completed.load(Ordering::Acquire));
        let byte_len = if complete {
            Some(file.metadata()?.len())
        } else {
            None
        };
        let reader: Box<dyn AudioReader> = if let Some(coord) = &self.stream {
            Box::new(ProgressiveReader::with_cancel(
                file,
                coord.clone(),
                cancelled.clone(),
            ))
        } else {
            Box::new(file)
        };
        let mut builder = rodio::decoder::DecoderBuilder::new()
            .with_data(BufReader::new(reader))
            .with_hint(self.hint.trim_start_matches('.'));
        if let Some(len) = byte_len {
            builder = builder.with_byte_len(len);
        } else if position > 0. {
            // A seek may need data not yet downloaded; only this worker waits.
            builder = builder.with_seekable(true);
            if let Some(len) = self
                .stream
                .as_ref()
                .map(|c| c.total.load(Ordering::Acquire))
                .filter(|n| *n > 0)
            {
                builder = builder.with_byte_len(len);
            }
        }
        let mut decoder = builder
            .build()
            .map_err(|_| Error::coded("decode_error", "无法解码此音频格式"))?;
        let decoded_duration = decoder
            .total_duration()
            .map(|v| v.as_secs_f64())
            .unwrap_or(0.);
        if track.path.is_empty()
            && track.duration > 0.
            && decoded_duration > 0.
            && decoded_duration < track.duration * 0.8
        {
            return Err(Error::coded(
                "incomplete_audio",
                "音频时长明显不足，可能只有试听片段",
            ));
        }
        let mut duration = if decoded_duration > 0. {
            decoded_duration
        } else {
            track.duration.max(0.)
        };
        let position = if duration > 0. {
            position.clamp(0., duration)
        } else {
            position.max(0.)
        };
        if position > 0. {
            decoder
                .try_seek(Duration::from_secs_f64(position))
                .map_err(|_| Error::new("此音频暂不支持跳转"))?;
        }
        let first = decoder.next();
        if first.is_none() && (duration <= 0. || position < duration) {
            return Err(Error::coded("decode_error", "音频中没有可解码的采样"));
        }
        let samples_per_second = decoder.sample_rate() as f64 * decoder.channels() as f64;
        // Validate complete files in silent test mode, but never drain a live
        // network stream before reporting it ready.
        if null && complete && position == 0. {
            let count = decoder.by_ref().count() + usize::from(first.is_some());
            if count == 0 {
                return Err(Error::coded("decode_error", "音频中没有可解码的采样"));
            }
            duration = count as f64 / samples_per_second;
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err(Error::coded("cancelled", "已取消"));
        }
        let samples = Arc::new(AtomicU64::new(0));
        Ok(PreparedAudio {
            source: Box::new(PrimedSource {
                source: Box::new(decoder),
                first,
                samples: samples.clone(),
            }),
            duration,
            position,
            cancelled,
            samples,
            samples_per_second,
        })
    }
}

pub struct Player {
    stream: Option<OutputStream>,
    sink: Option<Sink>,
    pub null: bool,
    duration: f64,
    base: f64,
    started: Instant,
    paused: bool,
    loaded: bool,
    cancelled: Option<Arc<AtomicBool>>,
    progress: Option<(Arc<AtomicU64>, f64)>,
}
impl Default for Player {
    fn default() -> Self {
        Self {
            stream: None,
            sink: None,
            null: std::env::var("WOCLI_AUDIO_OUTPUT").is_ok_and(|v| v == "null"),
            duration: 0.,
            base: 0.,
            started: Instant::now(),
            paused: false,
            loaded: false,
            cancelled: None,
            progress: None,
        }
    }
}
impl Player {
    pub fn load(&mut self, audio: PreparedAudio, volume: u8, paused: bool) -> Result<()> {
        if !self.null && self.stream.is_none() {
            let stream = rodio::OutputStreamBuilder::from_default_device()
                .and_then(|b| {
                    b.with_buffer_size(rodio::cpal::BufferSize::Fixed(512))
                        .open_stream()
                })
                .or_else(|_| {
                    rodio::OutputStreamBuilder::from_default_device().and_then(|b| {
                        b.with_buffer_size(rodio::cpal::BufferSize::Fixed(1024))
                            .open_stream()
                    })
                })
                .or_else(|_| rodio::OutputStreamBuilder::open_default_stream())
                .map_err(|_| Error::coded("audio_device", "无法打开系统音频输出设备"))?;
            self.stream = Some(stream);
        }
        self.stop();
        self.duration = audio.duration;
        self.base = audio.position;
        self.cancelled = Some(audio.cancelled);
        self.progress = Some((audio.samples, audio.samples_per_second));
        if !self.null {
            let sink = Sink::connect_new(self.stream.as_ref().unwrap().mixer());
            sink.set_volume(volume as f32 / 100.);
            if paused {
                sink.pause();
            }
            sink.append(audio.source);
            self.sink = Some(sink);
        }
        self.started = Instant::now();
        self.paused = paused;
        self.loaded = true;
        Ok(())
    }
    pub fn position(&self) -> f64 {
        if !self.loaded {
            return 0.;
        }
        let value = if let Some(sink) = &self.sink {
            self.base + sink.get_pos().as_secs_f64()
        } else {
            self.base
                + if self.paused {
                    0.
                } else {
                    self.started.elapsed().as_secs_f64()
                }
        };
        if self.duration > 0. {
            value.min(self.duration)
        } else {
            value
        }
    }
    pub fn duration(&self) -> f64 {
        self.duration
    }
    pub fn pause(&mut self, paused: bool) {
        let position = self.position();
        if let Some(sink) = &self.sink {
            if paused {
                sink.pause();
            } else {
                sink.play();
            }
        } else {
            self.base = position;
        }
        self.started = Instant::now();
        self.paused = paused;
    }
    pub fn volume(&self, volume: u8) {
        if let Some(sink) = &self.sink {
            sink.set_volume(volume as f32 / 100.);
        }
    }
    pub fn finished(&self) -> bool {
        self.loaded
            && !self.paused
            && self
                .sink
                .as_ref()
                .map(|v| v.empty())
                .unwrap_or_else(|| self.duration > 0. && self.position() >= self.duration)
    }
    pub fn end_error(&self) -> Option<Error> {
        if !self.null && self.finished() && self.duration > 0. {
            if let Some((samples, rate)) = &self.progress {
                let decoded = self.base + samples.load(Ordering::Relaxed) as f64 / rate;
                if decoded + 0.25 < self.duration * 0.8 {
                    return Some(Error::coded("decode_error", "音频解码提前中断"));
                }
            }
        }
        None
    }
    pub fn stop(&mut self) {
        if let Some(cancelled) = self.cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.loaded = false;
        self.progress = None;
        self.duration = 0.;
        self.base = 0.;
        self.paused = false;
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
pub(crate) fn test_wav(seconds: u32) -> Vec<u8> {
    let samples = 8000 * seconds;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + samples * 2).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((samples * 2).to_le_bytes());
    wav.resize(44 + samples as usize * 2, 0);
    wav
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_decoder_seeks_backwards_and_decodes_remaining_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seek.wav");
        std::fs::write(&path, test_wav(10)).unwrap();
        let input = AudioInput {
            path: path.clone(),
            hint: "wav".into(),
            stream: None,
        };
        let track = Track::local(&path).unwrap();
        let first = input
            .prepare(&track, 5., Arc::new(AtomicBool::new(false)), false)
            .unwrap();
        assert_eq!(first.source.count(), 40000);
        let back = input
            .prepare(&track, 1., Arc::new(AtomicBool::new(false)), false)
            .unwrap();
        assert_eq!(back.source.count(), 72000);
    }
    #[test]
    fn decoding_an_incomplete_body_is_a_failure_even_with_a_valid_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("truncated.wav");
        let wav = test_wav(10);
        std::fs::write(&path, &wav[..32044]).unwrap();
        let input = AudioInput {
            path: path.clone(),
            hint: "wav".into(),
            stream: None,
        };
        let audio = input
            .prepare(
                &Track::local(&path).unwrap(),
                0.,
                Arc::new(AtomicBool::new(false)),
                false,
            )
            .unwrap();
        let (sink, output) = Sink::new();
        sink.append(audio.source);
        let player = Player {
            sink: Some(sink),
            null: false,
            duration: audio.duration,
            loaded: true,
            progress: Some((audio.samples, audio.samples_per_second)),
            stream: None,
            base: 0.,
            started: Instant::now(),
            paused: false,
            cancelled: Some(audio.cancelled),
        };
        // Consume a hardware-free Sink through the same rodio adapters as real output.
        output.take(80000).for_each(drop);
        assert!(player.finished());
        assert_eq!(player.end_error().unwrap().code, "decode_error");
    }
    #[test]
    fn streaming_duration_rejects_a_preview_and_null_output_does_not_drain_stream() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview.wav");
        let wav = test_wav(10);
        std::fs::write(&path, &wav[..40000]).unwrap();
        let coord = Arc::new(StreamCoord::new(wav.len() as u64));
        coord.notify_progress(40000);
        let input = AudioInput {
            path,
            hint: "wav".into(),
            stream: Some(coord),
        };
        let track = Track {
            duration: 30.,
            ..Track::default()
        };
        assert_eq!(
            input
                .prepare(&track, 0., Arc::new(AtomicBool::new(false)), false)
                .err()
                .unwrap()
                .code,
            "incomplete_audio"
        );
        let audio = input
            .prepare(
                &Track::default(),
                0.,
                Arc::new(AtomicBool::new(false)),
                true,
            )
            .unwrap();
        assert_eq!(audio.duration, 10.);
    }
}
