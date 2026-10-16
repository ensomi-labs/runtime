//! PCM to the model inputs: mono mixdown, rate conversion, the two spectrograms and the row features.
use crate::fft::Fft;
use crate::machine::{Io, Tensor};
use crate::math::{f32r, half, round};
use crate::models::BEATTHIS_GRAPH;
use serde::Deserialize;

/// Interleaved samples at the request's sample rate.
pub enum Samples {
    F32(Vec<f32>),
    F64(Vec<f64>),
}

impl Samples {
    pub fn len(&self) -> usize {
        match self {
            Samples::F32(v) => v.len(),
            Samples::F64(v) => v.len(),
        }
    }

    fn at(&self, i: usize) -> f64 {
        match self {
            Samples::F32(v) => v[i] as f64,
            Samples::F64(v) => v[i],
        }
    }
}

pub fn mono(pcm: &Samples, channels: usize) -> Vec<f64> {
    (0..pcm.len() / channels)
        .map(|i| {
            let mut v = 0.0;
            for c in 0..channels {
                v += pcm.at(i * channels + c);
            }
            v / channels as f64
        })
        .collect()
}

#[derive(Deserialize)]
struct ResamplerSpec {
    numerator: usize,
    phases: usize,
    radius: usize,
    boundary_frames: usize,
    boundary_samples: usize,
}

/// A polyphase FIR from the soxr tables bundled with the runtime.
pub struct Resampler {
    numerator: usize,
    phases: usize,
    radius: usize,
    boundary_frames: usize,
    boundary_samples: usize,
    coefficients: Vec<f64>,
    boundary: Vec<f64>,
}

/// Reads the three files of a soxr table: the JSON description, the coefficients and the boundary
/// coefficients, in the order of [`resampler_files`].
pub fn load_resampler(files: &[Vec<u8>; 3]) -> Result<Resampler, String> {
    let spec: ResamplerSpec = serde_json::from_slice(&files[0]).map_err(|e| format!("resampler table: {e}"))?;
    let doubles = |bytes: &[u8]| -> Vec<f64> { bytes.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c)).collect() };
    let (coefficients, boundary) = (doubles(&files[1]), doubles(&files[2]));
    if spec.numerator == 0 || spec.phases == 0 || coefficients.len() != spec.phases * (2 * spec.radius + 1) || boundary.len() != spec.boundary_frames * spec.boundary_samples {
        return Err("resampler table: the coefficient files do not have the sizes its description gives".to_string());
    }
    Ok(Resampler {
        numerator: spec.numerator,
        phases: spec.phases,
        radius: spec.radius,
        boundary_frames: spec.boundary_frames,
        boundary_samples: spec.boundary_samples,
        coefficients,
        boundary,
    })
}

pub fn resample<T: Copy>(pcm: &[T], kernel: &Resampler) -> Vec<f32>
where
    f64: From<T>,
{
    let (p, q, r, bf, bs) = (kernel.numerator, kernel.phases, kernel.radius, kernel.boundary_frames, kernel.boundary_samples);
    let (k, b) = (&kernel.coefficients, &kernel.boundary);
    let len = round(pcm.len() as f64 * q as f64 / p as f64) as usize;
    let width = 2 * r + 1;
    let mut out = vec![0f32; len];
    for m in 0..len {
        let mut v = 0.0;
        if m < bf {
            for j in 0..bs.min(pcm.len()) {
                v += f64::from(pcm[j]) * b[m * bs + j];
            }
        } else {
            // Output m is centred on input m * p / q, with phase m * p mod q. The product is a u64: a usize has 32 bits
            // on wasm32, where it wrapped after 304 s of song at 96 kHz.
            let at = m as u64 * p as u64;
            let (i, phase) = ((at / q as u64) as usize, (at % q as u64) as usize);
            let (lo, hi) = (i.saturating_sub(r), (i + r).min(pcm.len() - 1));
            v = dot(&pcm[lo..=hi], &k[phase * width + r + lo - i..]);
        }
        out[m] = v as f32;
    }
    out
}

/// The sum of `x[j] * c[j]` over `x`, in four float64 accumulators, so that an addition does not wait for the one
/// before it.
fn dot<T: Copy>(x: &[T], c: &[f64]) -> f64
where
    f64: From<T>,
{
    let ((xs, x_rest), (cs, c_rest)) = (x.as_chunks::<4>(), c[..x.len()].as_chunks::<4>());
    let mut a = [0.0; 4];
    for (x, c) in xs.iter().zip(cs) {
        for l in 0..4 {
            a[l] += f64::from(x[l]) * c[l];
        }
    }
    let mut v = (a[0] + a[1]) + (a[2] + a[3]);
    for (&x, c) in x_rest.iter().zip(c_rest) {
        v += f64::from(x) * c;
    }
    v
}

/// The log spectrogram of one frontend: BeatThis (`beatthis`, reflect-padded 1024-point frames every
/// 441 samples) or the rows model's Log-Mel (960-point frames every 240 samples). 128 values per frame.
pub fn spectrogram(pcm: Vec<f32>, window: Vec<f32>, filter: &[f32], beatthis: bool) -> Vec<f32> {
    Spectrogram::new(pcm, window, filter, beatthis).finish()
}

/// A spectrogram computed a few frames at a time, so that it can run while a graph does.
pub struct Spectrogram {
    pcm: Vec<f32>,
    window: Vec<f32>,
    beatthis: bool,
    frames: usize,
    next: usize,
    out: Vec<f32>,
    input: Vec<f64>,
    fft: Fft,
    power: Vec<f32>,
    /// Only nonzero Mel weights contribute, in ascending FFT-bin order: band m sums entries ends[m] to ends[m + 1].
    ends: [usize; 129],
    bins: Vec<usize>,
    weights: Vec<f32>,
}

impl Spectrogram {
    pub fn new(pcm: Vec<f32>, window: Vec<f32>, filter: &[f32], beatthis: bool) -> Self {
        let n = window.len();
        let bins = n / 2 + 1;
        let hop = if beatthis { 441 } else { 240 };
        let frames = if beatthis { 1 + pcm.len() / hop } else { pcm.len().div_ceil(hop) };
        let mut ends = [0usize; 129];
        let (mut bin_list, mut weights) = (Vec::new(), Vec::new());
        for m in 0..128 {
            for k in 0..bins {
                let w = filter[if beatthis { k * 128 + m } else { m * bins + k }];
                if w != 0.0 {
                    bin_list.push(k);
                    weights.push(w);
                }
            }
            ends[m + 1] = bin_list.len();
        }
        Spectrogram { pcm, window, beatthis, frames, next: 0, out: vec![0f32; frames * 128], input: vec![0f64; n], fft: Fft::new(n), power: vec![0f32; bins], ends, bins: bin_list, weights }
    }

    /// Computes up to `count` more frames; returns whether any are left.
    pub fn advance(&mut self, count: usize) -> bool {
        let n = self.window.len();
        let beatthis = self.beatthis;
        let hop = if beatthis { 441 } else { 240 };
        let scale = if beatthis { 1.0 / 32.0 } else { 1.0 };
        let len = self.pcm.len() as i64;
        let end = self.frames.min(self.next.saturating_add(count));
        for f in self.next..end {
            let start = (f * hop) as i64 - if beatthis { n as i64 / 2 } else { 0 };
            for j in 0..n {
                let mut i = start + j as i64;
                if beatthis {
                    if i < 0 {
                        i = -i;
                    }
                    if i >= len {
                        i = 2 * len - 2 - i;
                    }
                }
                self.input[j] = if i >= 0 && i < len { f32r(self.pcm[i as usize] as f64 * self.window[j] as f64) } else { 0.0 };
            }
            self.fft.run(&self.input);
            for k in 0..n / 2 + 1 {
                let re = f32r(self.fft.re[k] * scale);
                let im = f32r(self.fft.im[k] * scale);
                let mag = f32r(f32r(f32r(re * re) + f32r(im * im)).sqrt());
                self.power[k] = if beatthis { mag as f32 } else { f32r(mag * mag) as f32 };
            }
            for m in 0..128 {
                let mut v = 0.0;
                for e in self.ends[m]..self.ends[m + 1] {
                    v += self.power[self.bins[e]] as f64 * self.weights[e] as f64;
                }
                self.out[f * 128 + m] = if beatthis { libm::log1p(f32r(1000.0 * f32r(v))) as f32 } else { libm::log(f64::max(f32r(v), 1e-5)) as f32 };
            }
        }
        self.next = end;
        end < self.frames
    }

    /// Computes the remaining frames and returns all of them.
    pub fn finish(mut self) -> Vec<f32> {
        self.advance(usize::MAX);
        self.out
    }
}

/// BeatThis logits for the whole song, from overlapping chunks run one graph call each.
pub struct Activations {
    pub beat: Vec<f32>,
    pub downbeat: Vec<f32>,
    pub beat_probability: Vec<f32>,
    pub downbeat_probability: Vec<f32>,
}

pub async fn beat_activations(io: &Io, spect: &[f32]) -> Result<Activations, String> {
    let frames = (spect.len() / 128) as i64;
    let mut starts = Vec::new();
    let mut s = -6;
    while s < frames - 6 {
        starts.push(s);
        s += 1488;
    }
    if frames > 1488 {
        // The first start, -6, is below frames - 6.
        *starts.last_mut().unwrap() = frames - 1494;
    }
    let mut outputs = Vec::new();
    for &start in &starts {
        let left = (-start).max(0);
        let lo = start.max(0);
        let hi = (start + 1500).min(frames);
        let right = 0.max(6.min(start + 1500 - frames));
        let mut data = vec![0f32; ((left + hi - lo + right) * 128) as usize];
        data[(left * 128) as usize..((left + hi - lo) * 128) as usize].copy_from_slice(&spect[(lo * 128) as usize..(hi * 128) as usize]);
        let rows = data.len() / 128;
        let mut out = io.run(BEATTHIS_GRAPH, vec![("spect", Tensor::f32(&[1, rows, 128], data))]).await?;
        outputs.push((out.take_f32("beat_logits", rows)?, out.take_f32("downbeat_logits", rows)?));
    }
    let mut beat = vec![-1000f32; frames as usize];
    let mut downbeat = vec![-1000f32; frames as usize];
    for c in (0..starts.len()).rev() {
        let start = starts[c];
        let len = (frames.min(start + 1494) - (start + 6)) as usize;
        let at = (start + 6) as usize;
        beat[at..at + len].copy_from_slice(&outputs[c].0[6..6 + len]);
        downbeat[at..at + len].copy_from_slice(&outputs[c].1[6..6 + len]);
    }
    let sigmoid = |x: f32| f32r(1.0 / (1.0 + libm::exp(-(x as f64)))) as f32;
    Ok(Activations { beat_probability: beat.iter().map(|&x| sigmoid(x)).collect(), downbeat_probability: downbeat.iter().map(|&x| sigmoid(x)).collect(), beat, downbeat })
}

/// The 130 row features per 10 ms frame: standardized Log-Mel and the interpolated beat and downbeat probabilities.
pub fn features(log_mel: &[f32], beat: &[f32], downbeat: &[f32]) -> Vec<f32> {
    let mut mean = 0.0;
    for &x in log_mel {
        mean += x as f64;
    }
    mean = f32r(mean / log_mel.len() as f64);
    let mut variance = 0.0;
    for &x in log_mel {
        let d = f32r(x as f64 - mean);
        variance += f32r(d * d);
    }
    let sd = f32r(f32r(variance / log_mel.len() as f64).sqrt());
    let frames = log_mel.len() / 128;
    let mut out = vec![0f32; frames * 130];
    for f in 0..frames {
        for m in 0..128 {
            out[f * 130 + m] = half(f32r(f32r(log_mel[f * 128 + m] as f64 - mean) / f32r(sd + 1e-6))) as f32;
        }
        let at = (f as f64 + 2.0) / 2.0;
        let i = (at.floor() as usize).min(beat.len() - 1);
        let j = (i + 1).min(beat.len() - 1);
        let t = at - at.floor();
        let (b0, b1, d0, d1) = (beat[i] as f64, beat[j] as f64, downbeat[i] as f64, downbeat[j] as f64);
        out[f * 130 + 128] = half(b0 + t * (b1 - b0)) as f32;
        out[f * 130 + 129] = half(d0 + t * (d1 - d0)) as f32;
    }
    out
}

/// PCM input has no decoder quantization: pydub's linear rate-conversion convention for Mel.
pub fn mel_pcm<T: Copy>(pcm: &[T], sample_rate: f64) -> Vec<f32>
where
    f64: From<T>,
{
    let length = ((pcm.len() as f64 - 1.0) * 24000.0 / sample_rate).floor() as usize + 1;
    let mut out = vec![0f32; length];
    let mut peak = 0.0f64;
    for i in 0..length {
        let at = i as f64 * sample_rate / 24000.0;
        let lo = at.floor();
        let hi = (lo as usize + 1).min(pcm.len() - 1);
        let (a, b) = (f64::from(pcm[lo as usize]), f64::from(pcm[hi]));
        let v = a + (at - lo) * (b - a);
        out[i] = v as f32;
        peak = peak.max((out[i] as f64).abs());
    }
    if peak != 0.0 {
        for x in out.iter_mut() {
            *x = f32r(*x as f64 / peak) as f32;
        }
    }
    out
}

/// Windowed-sinc conversion for PCM rates outside the release's soxr table rates.
pub fn sinc_resample<T: Copy>(pcm: &[T], sample_rate: f64, target_rate: f64) -> Vec<f32>
where
    f64: From<T>,
{
    use std::f64::consts::PI;
    let ratio = sample_rate / target_rate;
    let scale = f64::min(1.0, 1.0 / ratio) * 0.95;
    let radius = (128.0 / scale).ceil();
    let len = round(pcm.len() as f64 / ratio) as usize;
    let mut out = vec![0f32; len];
    for m in 0..len {
        let at = m as f64 * ratio;
        let (lo, hi) = ((at - radius).ceil() as i64, (at + radius).floor() as i64);
        let (mut value, mut weight) = (0.0, 0.0);
        for j in lo..=hi {
            let x = at - j as f64;
            let z = PI * x * scale;
            let sinc = if z == 0.0 { 1.0 } else { libm::sin(z) / z };
            let window = 0.42 + 0.5 * libm::cos(PI * x / radius) + 0.08 * libm::cos(2.0 * PI * x / radius);
            let w = sinc * window * scale;
            let sample = if j >= 0 && (j as usize) < pcm.len() { f64::from(pcm[j as usize]) } else { 0.0 };
            value += sample * w;
            weight += w;
        }
        out[m] = (value / weight) as f32;
    }
    out
}

/// The sample rates the soxr tables cover.
const TABLE_RATES: [u32; 9] = [11025, 16000, 24000, 32000, 44100, 48000, 88200, 96000, 192000];

/// The three files of the soxr table for `sample_rate`, if the release bundles one.
pub fn resampler_files(sample_rate: f64) -> Option<[String; 3]> {
    let rate = TABLE_RATES.iter().find(|&&r| r as f64 == sample_rate)?;
    Some([format!("resample-{rate}.json"), format!("resample-{rate}.bin"), format!("resample-{rate}-boundary.bin")])
}
