//! PCM to feature frames: the energy and the PCEN mel bands of each analysis window, the two values Sonalign reads.
//!
//! Ported from the Swift app's `MicFeaturePayloadExtractor` and `MicFeatureStreamBuffer`, at its default configuration.
//! That extractor's other features (onsets, chroma, CENS, landmarks, SNR) belong to an earlier engine and are not
//! ported: the live path hands Sonalign these two and the frame's time. Single-precision steps stay single-precision
//! so that frames agree with the Swift ones to rounding, not merely in substance.

use crate::resample::Resampler;
use crate::{Error, HOP_MS, HOP_SAMPLES, PROCESSING_RATE, WINDOW_MS, WINDOW_SAMPLES};
use sonalign::FeatureFrame;
use std::f64::consts::PI;

const BANDS: usize = 24;
const MINIMUM_FREQUENCY: f64 = 40.0;
const MAXIMUM_FREQUENCY: f64 = 8_000.0;
const PCEN_SMOOTHING: f32 = 0.025;
const PCEN_ALPHA: f32 = 0.98;
const PCEN_DELTA: f32 = 2.0;
const PCEN_ROOT: f32 = 0.5;
const PCEN_EPSILON: f32 = 0.000_001;
const SILENCE_FLOOR_DBFS: f64 = -120.0;
/// Whole songs pass through in blocks, so that no second copy of them is held.
const BLOCK_SAMPLES: usize = 8_192;

/// Frames of a whole mono song at `sample_rate`, timed from its start.
pub fn frames(pcm: &[f32], sample_rate: f64) -> Result<Vec<FeatureFrame>, Error> {
    whole(pcm.chunks(BLOCK_SAMPLES), pcm.len(), sample_rate)
}

/// [`frames`] of little-endian f32 PCM.
pub fn frames_f32le(pcm: &[u8], sample_rate: f64) -> Result<Vec<FeatureFrame>, Error> {
    let (samples, rest) = pcm.as_chunks::<4>();
    if !rest.is_empty() {
        return Err(Error::Misaligned);
    }
    let blocks = samples.chunks(BLOCK_SAMPLES).map(|block| block.iter().map(|bytes| f32::from_le_bytes(*bytes)).collect::<Vec<f32>>());
    whole(blocks, samples.len(), sample_rate)
}

fn whole<B: AsRef<[f32]>>(blocks: impl Iterator<Item = B>, samples: usize, sample_rate: f64) -> Result<Vec<FeatureFrame>, Error> {
    let mut stream = Stream::new(sample_rate, 0.0)?;
    let mut frames = Vec::with_capacity((samples as f64 * PROCESSING_RATE as f64 / sample_rate) as usize / HOP_SAMPLES + 1);
    for block in blocks {
        let block = block.as_ref();
        if block.iter().any(|sample| !sample.is_finite()) {
            return Err(Error::NotFinite);
        }
        stream.push(block, &mut frames);
    }
    stream.finish(&mut frames);
    Ok(frames)
}

/// Blocks of mono PCM at one rate to frames, cut at the same places however the blocks fall.
pub struct Stream {
    resampler: Option<Resampler>,
    pending: Vec<f32>,
    /// Where the next window starts on the timeline, in milliseconds.
    start_ms: f64,
    extractor: Extractor,
}

impl Stream {
    /// A stream whose first sample lies at `start_ms`. Its time is carried through conversion: a converted sample keeps
    /// the time of the input it was computed for, whenever it could be computed.
    pub fn new(sample_rate: f64, start_ms: f64) -> Result<Self, Error> {
        if sample_rate.fract() != 0.0 || !(8_000.0..=192_000.0).contains(&sample_rate) {
            return Err(Error::SampleRate);
        }
        let rate = sample_rate as u32;
        let resampler = (rate != PROCESSING_RATE).then(|| Resampler::new(rate, PROCESSING_RATE));
        Ok(Self { resampler, pending: Vec::with_capacity(2 * WINDOW_SAMPLES), start_ms, extractor: Extractor::new() })
    }

    /// Appends the frames this block completes. The block's samples must be finite.
    pub fn push(&mut self, samples: &[f32], frames: &mut impl Extend<FeatureFrame>) {
        match &mut self.resampler {
            Some(resampler) => resampler.process(samples, &mut self.pending),
            None => self.pending.extend_from_slice(samples),
        }
        self.drain(frames);
    }

    /// Ends the song: the conversion's last samples are computed, and windows that do not fill stay unused.
    pub fn finish(&mut self, frames: &mut impl Extend<FeatureFrame>) {
        if let Some(resampler) = &mut self.resampler {
            resampler.finish(&mut self.pending);
        }
        self.drain(frames);
    }

    /// Starts again at `start_ms`, after audio went missing. Nothing carries across the gap.
    pub fn restart(&mut self, start_ms: f64) {
        if let Some(resampler) = &mut self.resampler {
            resampler.reset();
        }
        self.pending.clear();
        self.start_ms = start_ms;
        self.extractor.smoothers = None;
    }

    fn drain(&mut self, frames: &mut impl Extend<FeatureFrame>) {
        let mut start = 0;
        while self.pending.len() - start >= WINDOW_SAMPLES {
            let (energy_dbfs, pcen_mel) = self.extractor.frame(&self.pending[start..start + WINDOW_SAMPLES]);
            frames.extend([FeatureFrame { recorded_time_ms: self.start_ms + WINDOW_MS, energy_dbfs, pcen_mel }]);
            start += HOP_SAMPLES;
            self.start_ms += HOP_MS;
        }
        self.pending.drain(..start);
    }
}

struct Extractor {
    hann: Vec<f32>,
    windowed: Vec<f64>,
    fft: Fft,
    /// Spectral power from FFT bin 1 on, as far as a mel band reads.
    power: Vec<f32>,
    /// Only nonzero weights count, in ascending bin order: band b sums entries `ends[b]` to `ends[b + 1]`.
    ends: [usize; BANDS + 1],
    /// Indices into `power`.
    bins: Vec<usize>,
    weights: Vec<f32>,
    smoothers: Option<[f32; BANDS]>,
}

impl Extractor {
    fn new() -> Self {
        // A symmetric window. The Swift extractor's denominator is count - 1.
        let hann = (0..WINDOW_SAMPLES).map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / (WINDOW_SAMPLES - 1) as f64).cos()) as f32).collect();
        let (ends, bins, weights) = triangles();
        let last = bins.iter().copied().max().unwrap_or(0);
        Self { hann, windowed: vec![0.0; WINDOW_SAMPLES], fft: Fft::new(WINDOW_SAMPLES), power: vec![0.0; last + 1], ends, bins, weights, smoothers: None }
    }

    /// The energy in dBFS and the PCEN mel bands of one window.
    fn frame(&mut self, samples: &[f32]) -> (f64, Vec<f32>) {
        let energy = energy_dbfs(samples);
        let mean = (samples.iter().map(|&sample| f64::from(sample)).sum::<f64>() / samples.len() as f64) as f32;
        for (i, &sample) in samples.iter().enumerate() {
            self.windowed[i] = f64::from((sample - mean) * self.hann[i]);
        }
        self.fft.run(&self.windowed);
        // vDSP's real FFT, under the Swift extractor, returns twice the DFT. PCEN is not scale-free, so that factor
        // stays part of the features, and reference and query must both have it.
        let scale = 1.0 / WINDOW_SAMPLES as f32;
        for (i, power) in self.power.iter_mut().enumerate() {
            let magnitude = ((2.0 * self.fft.re[i + 1]) as f32).hypot((2.0 * self.fft.im[i + 1]) as f32) * scale;
            *power = magnitude * magnitude;
        }
        let mut mel = [0f32; BANDS];
        for (band, value) in mel.iter_mut().enumerate() {
            for entry in self.ends[band]..self.ends[band + 1] {
                *value += self.power[self.bins[entry]] * self.weights[entry];
            }
        }
        (energy, self.pcen(&mel))
    }

    fn pcen(&mut self, mel: &[f32; BANDS]) -> Vec<f32> {
        let smoothers = self.smoothers.get_or_insert(*mel);
        let delta_root = PCEN_DELTA.powf(PCEN_ROOT);
        (0..BANDS)
            .map(|band| {
                let smoothed = (1.0 - PCEN_SMOOTHING) * smoothers[band] + PCEN_SMOOTHING * mel[band];
                smoothers[band] = smoothed;
                let normalized = mel[band] / smoothed.max(PCEN_EPSILON).powf(PCEN_ALPHA);
                ((normalized + PCEN_DELTA).powf(PCEN_ROOT) - delta_root).max(0.0)
            })
            .collect()
    }
}

fn energy_dbfs(samples: &[f32]) -> f64 {
    let mean_square = samples.iter().map(|&sample| f64::from(sample) * f64::from(sample)).sum::<f64>() / samples.len() as f64;
    let rms = mean_square.sqrt();
    if rms > 0.0 { (20.0 * rms.log10()).max(SILENCE_FLOOR_DBFS) } else { SILENCE_FLOOR_DBFS }
}

fn hertz_to_mel(hertz: f64) -> f64 {
    2_595.0 * (1.0 + hertz / 700.0).log10()
}

/// The triangular mel filters of [`MINIMUM_FREQUENCY`] to [`MAXIMUM_FREQUENCY`] over the FFT bins.
fn triangles() -> ([usize; BANDS + 1], Vec<usize>, Vec<f32>) {
    let (lower, upper) = (hertz_to_mel(MINIMUM_FREQUENCY), hertz_to_mel(MAXIMUM_FREQUENCY));
    let point = |i: usize| lower + (upper - lower) * i as f64 / (BANDS + 1) as f64;
    let (mut ends, mut bins, mut weights) = ([0; BANDS + 1], Vec::new(), Vec::new());
    for band in 0..BANDS {
        let (low, center, high) = (point(band), point(band + 1), point(band + 2));
        for bin in 1..=WINDOW_SAMPLES / 2 {
            let frequency = PROCESSING_RATE as f64 * bin as f64 / WINDOW_SAMPLES as f64;
            if !(MINIMUM_FREQUENCY..=MAXIMUM_FREQUENCY).contains(&frequency) {
                continue;
            }
            let mel = hertz_to_mel(frequency);
            let weight = if mel >= low && mel <= center {
                (mel - low) / (center - low)
            } else if mel > center && mel <= high {
                (high - mel) / (high - center)
            } else {
                0.0
            };
            if weight > 0.0 {
                bins.push(bin - 1);
                weights.push(weight as f32);
            }
        }
        ends[band + 1] = bins.len();
    }
    (ends, bins, weights)
}

/// A power-of-two FFT of a real signal. Double precision: rounding here would only add to the rounding of vDSP's, which
/// the frames of the two implementations are compared against.
struct Fft {
    re: Vec<f64>,
    im: Vec<f64>,
    reversed: Vec<usize>,
    cos: Vec<f64>,
    sin: Vec<f64>,
}

impl Fft {
    fn new(n: usize) -> Self {
        let bits = n.trailing_zeros();
        let reversed = (0..n).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        let cos = (0..n / 2).map(|k| (2.0 * PI * k as f64 / n as f64).cos()).collect();
        let sin = (0..n / 2).map(|k| -(2.0 * PI * k as f64 / n as f64).sin()).collect();
        Self { re: vec![0.0; n], im: vec![0.0; n], reversed, cos, sin }
    }

    fn run(&mut self, input: &[f64]) {
        let n = input.len();
        for i in 0..n {
            self.re[i] = input[self.reversed[i]];
            self.im[i] = 0.0;
        }
        let mut half = 1;
        while half < n {
            let step = n / (2 * half);
            for start in (0..n).step_by(2 * half) {
                for k in 0..half {
                    let (p, q) = (start + k, start + k + half);
                    let (wr, wi) = (self.cos[k * step], self.sin[k * step]);
                    let (re, im) = (self.re[q] * wr - self.im[q] * wi, self.re[q] * wi + self.im[q] * wr);
                    (self.re[q], self.im[q]) = (self.re[p] - re, self.im[p] - im);
                    (self.re[p], self.im[p]) = (self.re[p] + re, self.im[p] + im);
                }
            }
            half *= 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hertz: f64, amplitude: f64, count: usize) -> Vec<f32> {
        (0..count).map(|i| (amplitude * (2.0 * PI * hertz * i as f64 / PROCESSING_RATE as f64).sin()) as f32).collect()
    }

    #[test]
    fn fft_matches_the_definition() {
        let input: Vec<f64> = (0..16).map(|i| ((i * i) as f64 * 0.37).sin() + 0.25).collect();
        let mut fft = Fft::new(16);
        fft.run(&input);
        for k in 0..16 {
            let (mut re, mut im) = (0.0, 0.0);
            for (n, x) in input.iter().enumerate() {
                let angle = -2.0 * PI * (k * n) as f64 / 16.0;
                re += x * angle.cos();
                im += x * angle.sin();
            }
            assert!((fft.re[k] - re).abs() < 1e-12 && (fft.im[k] - im).abs() < 1e-12, "bin {k}");
        }
    }

    #[test]
    fn energy_is_rms_in_dbfs_with_a_floor() {
        // The peak of a sine of amplitude 0.5 gives an RMS of 0.3536, or -9.03 dBFS.
        assert!((energy_dbfs(&tone(1_000.0, 0.5, 4_800)) + 9.03).abs() < 0.01);
        assert_eq!(energy_dbfs(&[0.0; 16]), SILENCE_FLOOR_DBFS);
        assert_eq!(energy_dbfs(&[1e-30; 16]), SILENCE_FLOOR_DBFS);
    }

    #[test]
    fn mel_filters_cover_40_hz_to_8_khz_in_24_overlapping_bands() {
        let (ends, bins, weights) = triangles();
        // Bin spacing is 46.875 Hz: the first bin in range is the first FFT bin, the last is 170 at 7968.75 Hz.
        assert_eq!(bins.iter().min(), Some(&0));
        assert_eq!(bins.iter().max(), Some(&169));
        assert!(weights.iter().all(|&weight| weight > 0.0 && weight <= 1.0));
        assert_eq!(ends[BANDS], bins.len());
        assert!((0..BANDS).all(|band| ends[band + 1] > ends[band]), "{ends:?}");
    }

    #[test]
    fn a_tone_lights_its_mel_band_and_the_scale_is_vdsps() {
        let mut extractor = Extractor::new();
        // Exactly FFT bin 21. In mel it lies a third of the way from the centre of band 7 to that of band 8.
        let window = tone(21.0 * 46.875, 0.5, WINDOW_SAMPLES);
        let (_, pcen) = extractor.frame(&window);
        let loudest = pcen.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(loudest, 7);
        // Hann halves a sine of amplitude A, for a bin of A N / 4 in a DFT; vDSP's scale, over N, makes it A / 2.
        let peak = extractor.power.iter().copied().fold(0.0, f32::max);
        assert!((peak - 0.0625).abs() < 0.001, "{peak}");
    }

    #[test]
    fn silence_has_no_pcen_and_the_floor_energy() {
        let mut extractor = Extractor::new();
        let (energy, pcen) = extractor.frame(&[0.0; WINDOW_SAMPLES]);
        assert_eq!(energy, SILENCE_FLOOR_DBFS);
        assert_eq!(pcen, vec![0.0; BANDS]);
    }

    #[test]
    fn pcen_starts_from_the_first_frame_and_then_smooths() {
        let mut extractor = Extractor::new();
        let mut mel = [0.0; BANDS];
        mel[0] = 1.0;
        // The first frame is its own smoothed value, so normalized = 1, and PCEN = sqrt(3) - sqrt(2).
        let first = extractor.pcen(&mel);
        assert!((first[0] - (3f32.sqrt() - 2f32.sqrt())).abs() < 1e-6);
        // The same energy after silence stands out against its smoothed self.
        extractor.smoothers = Some([0.01; BANDS]);
        let onset = extractor.pcen(&mel);
        assert!(onset[0] > first[0] * 5.0, "{}", onset[0]);
    }

    #[test]
    fn frames_end_their_windows_on_the_hop_grid() {
        let pcm = tone(440.0, 0.5, WINDOW_SAMPLES + 3 * HOP_SAMPLES + 100);
        let frames = frames(&pcm, 48_000.0).unwrap();
        assert_eq!(frames.len(), 4);
        for (i, frame) in frames.iter().enumerate() {
            assert!((frame.recorded_time_ms - (WINDOW_MS + i as f64 * HOP_MS)).abs() < 1e-9);
        }
    }

    #[test]
    fn frames_do_not_depend_on_how_blocks_fall() {
        let pcm: Vec<f32> = (0..24_000).map(|i| ((i * i) as f32 * 1e-5).sin() * 0.3).collect();
        let whole = frames(&pcm, 44_100.0).unwrap();
        for size in [1, 7, 480, 4_096, 24_000] {
            let mut stream = Stream::new(44_100.0, 0.0).unwrap();
            let mut chunked = Vec::new();
            for block in pcm.chunks(size) {
                stream.push(block, &mut chunked);
            }
            stream.finish(&mut chunked);
            assert_eq!(chunked, whole, "blocks of {size}");
        }
    }

    #[test]
    fn bytes_read_as_the_same_samples() {
        let pcm: Vec<f32> = (0..4_096).map(|i| (i as f32 * 0.03).sin() * 0.3).collect();
        let bytes: Vec<u8> = pcm.iter().flat_map(|sample| sample.to_le_bytes()).collect();
        assert_eq!(frames_f32le(&bytes, 48_000.0).unwrap(), frames(&pcm, 48_000.0).unwrap());
        assert_eq!(frames_f32le(&bytes[1..], 48_000.0), Err(Error::Misaligned));
    }

    #[test]
    fn invalid_audio_is_refused() {
        assert_eq!(frames(&[0.0, f32::NAN], 48_000.0), Err(Error::NotFinite));
        for rate in [0.0, 44_100.5, 7_999.0, 192_001.0, f64::NAN, f64::INFINITY] {
            assert_eq!(frames(&[0.0], rate).err(), Some(Error::SampleRate), "{rate}");
        }
    }
}
