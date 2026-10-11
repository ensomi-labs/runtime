//! PCM to the model inputs: mono mixdown, rate conversion, the two spectrograms and the row features.
use crate::fft::Fft;
use crate::jsmath;
use crate::machine::{Io, Tensor};
use crate::math::{f32r, half, round};
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
    let doubles = |bytes: &[u8]| -> Vec<f64> { bytes.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).collect() };
    Ok(Resampler {
        numerator: spec.numerator,
        phases: spec.phases,
        radius: spec.radius,
        boundary_frames: spec.boundary_frames,
        boundary_samples: spec.boundary_samples,
        coefficients: doubles(&files[1]),
        boundary: doubles(&files[2]),
    })
}

pub fn resample(pcm: &[f64], kernel: &Resampler) -> Vec<f32> {
    let (p, q, r, bf, bs) = (kernel.numerator, kernel.phases, kernel.radius as i64, kernel.boundary_frames, kernel.boundary_samples);
    let (k, b) = (&kernel.coefficients, &kernel.boundary);
    let len = round(pcm.len() as f64 * q as f64 / p as f64) as usize;
    let width = 2 * r + 1;
    let mut out = vec![0f32; len];
    for m in 0..len {
        let mut v = 0.0;
        if m < bf {
            for j in 0..bs.min(pcm.len()) {
                v += pcm[j] * b[m * bs + j];
            }
        } else {
            let i = (m * p / q) as i64;
            let phase = (m * p % q) as i64;
            let mut j = (-r).max(-i);
            let end = r.min(pcm.len() as i64 - 1 - i);
            while j <= end {
                v += pcm[(i + j) as usize] * k[(phase * width + j + r) as usize];
                j += 1;
            }
        }
        out[m] = v as f32;
    }
    out
}

/// The log spectrogram of one frontend: BeatThis (`beatthis`, reflect-padded 1024-point frames every
/// 441 samples) or the rows model's Log-Mel (960-point frames every 240 samples). 128 values per frame.
pub fn spectrogram(pcm: &[f32], window: &[f32], filter: &[f32], beatthis: bool) -> Vec<f32> {
    let n = window.len();
    let bins = n / 2 + 1;
    let hop = if beatthis { 441 } else { 240 };
    let scale = if beatthis { 1.0 / 32.0 } else { 1.0 };
    let frames = if beatthis { 1 + pcm.len() / hop } else { pcm.len().div_ceil(hop) };
    let mut out = vec![0f32; frames * 128];
    let mut input = vec![0f64; n];
    let mut fft = Fft::new(n);
    let mut power = vec![0f32; bins];
    // Only nonzero Mel weights contribute, in ascending FFT-bin order: band m sums entries ends[m] to ends[m + 1].
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
    let len = pcm.len() as i64;
    for f in 0..frames {
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
            input[j] = if i >= 0 && i < len { f32r(pcm[i as usize] as f64 * window[j] as f64) } else { 0.0 };
        }
        fft.run(&input);
        for k in 0..bins {
            let re = f32r(fft.re[k] * scale);
            let im = f32r(fft.im[k] * scale);
            let mag = f32r(f32r(f32r(re * re) + f32r(im * im)).sqrt());
            power[k] = if beatthis { mag as f32 } else { f32r(mag * mag) as f32 };
        }
        for m in 0..128 {
            let mut v = 0.0;
            for e in ends[m]..ends[m + 1] {
                v += power[bin_list[e]] as f64 * weights[e] as f64;
            }
            out[f * 128 + m] = if beatthis { jsmath::log1p(f32r(1000.0 * f32r(v))) as f32 } else { jsmath::log(f64::max(f32r(v), 1e-5)) as f32 };
        }
    }
    out
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
        let mut out = io.run("beatthis", vec![("spect", Tensor::f32(&[1, rows, 128], data))]).await?;
        outputs.push((out.take_f32("beat_logits")?, out.take_f32("downbeat_logits")?));
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
    let sigmoid = |x: f32| f32r(1.0 / (1.0 + jsmath::exp(-(x as f64)))) as f32;
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
pub fn mel_pcm(pcm: &[f64], sample_rate: f64) -> Vec<f32> {
    let length = ((pcm.len() as f64 - 1.0) * 24000.0 / sample_rate).floor() as usize + 1;
    let mut out = vec![0f32; length];
    let mut peak = 0.0f64;
    for i in 0..length {
        let at = i as f64 * sample_rate / 24000.0;
        let lo = at.floor();
        let hi = (lo as usize + 1).min(pcm.len() - 1);
        let v = pcm[lo as usize] + (at - lo) * (pcm[hi] - pcm[lo as usize]);
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
pub fn sinc_resample(pcm: &[f64], sample_rate: f64, target_rate: f64) -> Vec<f32> {
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
            let sinc = if z == 0.0 { 1.0 } else { jsmath::sin(z) / z };
            let window = 0.42 + 0.5 * jsmath::cos(PI * x / radius) + 0.08 * jsmath::cos(2.0 * PI * x / radius);
            let w = sinc * window * scale;
            let sample = if j >= 0 && (j as usize) < pcm.len() { pcm[j as usize] } else { 0.0 };
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
