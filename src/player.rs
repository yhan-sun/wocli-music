use crate::model::{Error, Result, Track};
use rodio::{OutputStream, Sink, Source};
use std::{
    fs::File,
    path::Path,
    time::{Duration, Instant},
};

pub struct Player {
    stream: Option<OutputStream>,
    sink: Option<Sink>,
    null: bool,
    duration: f64,
    base: f64,
    started: Instant,
    paused: bool,
    loaded: bool,
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
        }
    }
}
impl Player {
    pub fn load(&mut self, path: &Path, track: &Track, volume: u8) -> Result<()> {
        let file = File::open(path)?;
        let hint = path.extension().and_then(|e| e.to_str());
        self.load_reader(std::io::BufReader::new(file), track, volume, hint, false)
    }

    pub fn load_reader<R>(
        &mut self,
        reader: std::io::BufReader<R>,
        track: &Track,
        volume: u8,
        format_hint: Option<&str>,
        is_streaming: bool,
    ) -> Result<()>
    where
        R: std::io::Read + std::io::Seek + Send + Sync + 'static,
    {
        self.stop();
        let mut builder = rodio::decoder::DecoderBuilder::new().with_data(reader);
        if let Some(hint) = format_hint {
            let clean = hint.trim_start_matches('.');
            if !clean.is_empty() {
                builder = builder.with_hint(clean);
            }
        }
        let decoder = builder
            .build()
            .map_err(|_| Error::coded("decode_error", "无法解码此音频格式"))?;
        let duration = decoder
            .total_duration()
            .map(|v| v.as_secs_f64())
            .unwrap_or(track.duration);
        if !is_streaming
            && track.path.is_empty()
            && track.duration > 0.
            && duration > 0.
            && duration < track.duration * 0.8
        {
            return Err(Error::coded(
                "incomplete_audio",
                "音频时长明显不足，可能只有试听片段",
            ));
        }
        self.duration = if track.duration > 0. {
            track.duration
        } else if duration > 0. {
            duration
        } else {
            0.
        };
        if self.null {
            let rate = decoder.sample_rate() as f64 * decoder.channels() as f64;
            let count = decoder.count();
            if count == 0 {
                return Err(Error::coded("decode_error", "音频中没有可解码的采样"));
            }
            if !is_streaming {
                self.duration = count as f64 / rate;
            }
        } else {
            if self.stream.is_none() {
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
            let sink = Sink::connect_new(self.stream.as_ref().unwrap().mixer());
            sink.set_volume(volume as f32 / 100.);
            sink.append(decoder);
            self.sink = Some(sink);
        }
        self.started = Instant::now();
        self.base = 0.;
        self.paused = false;
        self.loaded = true;
        Ok(())
    }
    pub fn position(&self) -> f64 {
        if !self.loaded {
            return 0.;
        }
        if let Some(sink) = &self.sink {
            return sink.get_pos().as_secs_f64().min(self.duration.max(0.));
        }
        let value = self.base
            + if self.paused {
                0.
            } else {
                self.started.elapsed().as_secs_f64()
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
        if let Some(sink) = &self.sink {
            if paused {
                sink.pause();
            } else {
                sink.play();
            }
        }
        let position = self.position();
        self.base = position;
        self.started = Instant::now();
        self.paused = paused;
    }
    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        if !seconds.is_finite() || !self.loaded {
            return Err(Error::new("当前没有可跳转的音频"));
        }
        let position = seconds.clamp(0., self.duration.max(0.));
        if let Some(sink) = &self.sink {
            sink.try_seek(Duration::from_secs_f64(position))
                .map_err(|_| Error::new("此音频暂不支持跳转"))?;
        }
        self.base = position;
        self.started = Instant::now();
        Ok(())
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
    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.loaded = false;
        self.duration = 0.;
        self.base = 0.;
        self.paused = false;
    }
}
