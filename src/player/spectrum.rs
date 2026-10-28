//! A bounded, optional tap. The audio callback only attempts a nonblocking push;
//! frequency analysis runs on its own worker and never delays audio.
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

pub(super) const BANDS: usize = 8;
const WINDOW: usize = 512;
const DECIMATE: usize = 4;
const EDGES: [f32; BANDS + 1] = [45., 115., 230., 460., 900., 1700., 2800., 4200., 6000.];

pub(super) struct Spectrum {
    pub enabled: AtomicBool,
    levels: [AtomicU32; BANDS],
}
impl Default for Spectrum {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            levels: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
}
impl Spectrum {
    pub fn levels(&self) -> [f32; BANDS] {
        std::array::from_fn(|i| f32::from_bits(self.levels[i].load(Ordering::Relaxed)))
    }
}
pub(super) struct Tap {
    producer: Producer<f32>,
    count: usize,
    channels: usize,
    frames: usize,
    sum: f32,
    state: Arc<Spectrum>,
}
impl Tap {
    pub fn sample(&mut self, sample: f32) {
        self.count += 1;
        if self.count < self.channels {
            return;
        }
        self.count = 0;
        if !self.state.enabled.load(Ordering::Relaxed) {
            self.frames = 0;
            self.sum = 0.;
            return;
        }
        self.sum += if sample.is_finite() {
            sample.clamp(-1., 1.)
        } else {
            0.
        };
        self.frames += 1;
        if self.frames == DECIMATE {
            let _ = self.producer.push(self.sum / DECIMATE as f32);
            self.sum = 0.;
            self.frames = 0;
        }
    }
}
fn bands(samples: &[f32; WINDOW], rate: f32) -> [f32; BANDS] {
    // In-place radix-2 FFT. Average energy within each frequency range rather
    // than sampling a single pitch, so ordinary music fills the display.
    let mut real = [0.; WINDOW];
    let mut imag = [0.; WINDOW];
    for (n, &sample) in samples.iter().enumerate() {
        let index = n.reverse_bits() >> (usize::BITS - 9);
        let window = 0.5 - 0.5 * (std::f32::consts::TAU * n as f32 / (WINDOW - 1) as f32).cos();
        real[index] = sample * window;
    }
    let mut size = 2;
    while size <= WINDOW {
        let angle = -std::f32::consts::TAU / size as f32;
        let (step_im, step_re) = angle.sin_cos();
        for start in (0..WINDOW).step_by(size) {
            let (mut wr, mut wi) = (1., 0.);
            for j in 0..size / 2 {
                let a = start + j;
                let b = a + size / 2;
                let tr = wr * real[b] - wi * imag[b];
                let ti = wr * imag[b] + wi * real[b];
                real[b] = real[a] - tr;
                imag[b] = imag[a] - ti;
                real[a] += tr;
                imag[a] += ti;
                (wr, wi) = (wr * step_re - wi * step_im, wr * step_im + wi * step_re);
            }
        }
        size *= 2;
    }
    let mut power = [0.; BANDS];
    let mut counts = [0u32; BANDS];
    for k in 1..WINDOW / 2 {
        let frequency = k as f32 * rate / WINDOW as f32;
        if let Some(i) = (0..BANDS).find(|&i| frequency >= EDGES[i] && frequency < EDGES[i + 1]) {
            power[i] += real[k] * real[k] + imag[k] * imag[k];
            counts[i] += 1;
        }
    }
    std::array::from_fn(|i| {
        if counts[i] == 0 {
            0.
        } else {
            ((power[i] / counts[i] as f32).sqrt() * 4. / WINDOW as f32)
                .sqrt()
                .mul_add(2.0, 0.)
                .clamp(0., 1.)
        }
    })
}
pub(super) fn start(rate: u32, channels: u16) -> std::io::Result<(Tap, Arc<Spectrum>)> {
    let (producer, consumer) = RingBuffer::new(WINDOW * 4);
    let state = Arc::new(Spectrum::default());
    let worker = state.clone();
    thread::Builder::new()
        .name("wocli-spectrum".into())
        .spawn(move || analyze(consumer, worker, rate as f32 / DECIMATE as f32))?;
    Ok((
        Tap {
            producer,
            count: 0,
            channels: channels as usize,
            frames: 0,
            sum: 0.,
            state: state.clone(),
        },
        state,
    ))
}
fn analyze(mut consumer: Consumer<f32>, state: Arc<Spectrum>, rate: f32) {
    let mut window = [0.; WINDOW];
    let mut used = 0;
    while !consumer.is_abandoned() {
        if !state.enabled.load(Ordering::Relaxed) {
            while consumer.pop().is_ok() {}
            used = 0;
            for level in &state.levels {
                level.store(0, Ordering::Relaxed);
            }
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        while let Ok(sample) = consumer.pop() {
            window[used] = sample;
            used += 1;
            if used == WINDOW {
                for (target, value) in state.levels.iter().zip(bands(&window, rate)) {
                    target.store(value.to_bits(), Ordering::Relaxed);
                }
                used = 0;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tap_feeds_bounded_worker_only_when_enabled() {
        let (mut tap, state) = start(48000, 1).unwrap();
        state.enabled.store(true, Ordering::Relaxed);
        for n in 0..(WINDOW * DECIMATE * 2) {
            let sample = (std::f32::consts::TAU * 2200. * n as f32 / 48000.).sin() * 0.5;
            tap.sample(sample);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while state.levels()[5] < 0.05 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(state.levels()[5] > state.levels()[1]);
        state.enabled.store(false, Ordering::Relaxed);
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while state.levels() != [0.; BANDS] && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.levels(), [0.; BANDS]);
    }
    #[test]
    fn low_and_high_tones_have_different_frequency_peaks() {
        for (frequency, expected) in [(160., 1), (2200., 5)] {
            let mut samples = [0.; WINDOW];
            for (n, sample) in samples.iter_mut().enumerate() {
                *sample = (std::f32::consts::TAU * frequency * n as f32 / 12000.).sin();
            }
            let values = bands(&samples, 12000.);
            let peak = values
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert_eq!(peak, expected, "{values:?}");
        }
        assert_eq!(bands(&[0.; WINDOW], 12000.), [0.; BANDS]);
    }
}
