//! Streaming rate conversion with no delay: every output sample is computed for its exact place on the input's
//! timeline, so features from a converted song and from a microphone at the processing rate line up.
//!
//! A Kaiser-windowed sinc, polyphase over the reduced rate ratio, in exact integer positions. Only the band below 8 kHz
//! reaches the features, far inside the passband; the Swift app's reference goes through `AVAudioConverter`. The soxr
//! tables under `assets/resampler` convert to 22050 Hz only, and the generator's converters work on whole songs.

/// Sinc zero crossings on each side of the centre.
const ZEROS: f64 = 16.0;
/// The cutoff against the lower Nyquist frequency.
const ROLLOFF: f64 = 0.95;
const KAISER_BETA: f64 = 9.0;

pub struct Resampler {
    up: u64,
    down: u64,
    radius: usize,
    /// `up` rows of `2 * radius + 1` coefficients, one row per position between input samples.
    table: Vec<f32>,
    /// Input not yet fully used. It starts at the input index `base - radius`; before the song is silence.
    input: Vec<f32>,
    base: u64,
    /// Input samples taken and output samples made since the start.
    taken: u64,
    made: u64,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let divisor = gcd(from, to);
        let (up, down) = (to / divisor, from / divisor);
        let cutoff = ROLLOFF * (f64::from(to) / f64::from(from)).min(1.0);
        let radius = (ZEROS / cutoff).ceil() as usize;
        let width = 2 * radius + 1;
        let mut table = Vec::with_capacity(up as usize * width);
        for phase in 0..up {
            let fraction = f64::from(phase) / f64::from(up);
            let row: Vec<f64> = (0..width).map(|j| kernel(fraction + radius as f64 - j as f64, cutoff, radius as f64 + 1.0)).collect();
            // Unit gain at DC in every phase.
            let sum: f64 = row.iter().sum();
            table.extend(row.iter().map(|c| (c / sum) as f32));
        }
        Self { up: up.into(), down: down.into(), radius, table, input: vec![0.0; radius], base: 0, taken: 0, made: 0 }
    }

    /// Appends the output this input makes possible. It lags the input by the filter's radius, but carries no delay.
    pub fn process(&mut self, samples: &[f32], output: &mut Vec<f32>) {
        self.input.extend_from_slice(samples);
        self.taken += samples.len() as u64;
        self.run(u64::MAX, output);
    }

    /// Ends the input: it is followed by silence, and output stops where the input does.
    pub fn finish(&mut self, output: &mut Vec<f32>) {
        self.input.resize(self.input.len() + 2 * self.radius + 1, 0.0);
        self.run((self.taken * self.up).div_ceil(self.down), output);
    }

    /// Forgets the input, as if it were new.
    pub fn reset(&mut self) {
        self.input.clear();
        self.input.resize(self.radius, 0.0);
        (self.base, self.taken, self.made) = (0, 0, 0);
    }

    fn run(&mut self, limit: u64, output: &mut Vec<f32>) {
        let width = 2 * self.radius + 1;
        let mut start;
        loop {
            let at = self.made * self.down;
            start = (at / self.up - self.base) as usize;
            if self.made >= limit || start + width > self.input.len() {
                break;
            }
            let phase = (at % self.up) as usize;
            output.push(dot(&self.input[start..start + width], &self.table[phase * width..(phase + 1) * width]));
            self.made += 1;
        }
        // Nothing before the next output's window is read again.
        let used = start.min(self.input.len());
        self.input.drain(..used);
        self.base += used as u64;
    }
}

/// The sum of products in eight accumulators, which vectorises without fast-math.
fn dot(x: &[f32], c: &[f32]) -> f32 {
    let ((xs, x_rest), (cs, c_rest)) = (x.as_chunks::<8>(), c.as_chunks::<8>());
    let mut lanes = [0f32; 8];
    for (x, c) in xs.iter().zip(cs) {
        for l in 0..8 {
            lanes[l] += x[l] * c[l];
        }
    }
    let mut sum = lanes.iter().sum::<f32>();
    for (x, c) in x_rest.iter().zip(c_rest) {
        sum += x * c;
    }
    sum
}

/// The impulse response `distance` input samples from the centre.
fn kernel(distance: f64, cutoff: f64, half_width: f64) -> f64 {
    use std::f64::consts::PI;
    let z = PI * cutoff * distance;
    let sinc = if z == 0.0 { 1.0 } else { z.sin() / z };
    let edge = distance / half_width;
    cutoff * sinc * bessel_i0(KAISER_BETA * (1.0 - edge * edge).max(0.0).sqrt()) / bessel_i0(KAISER_BETA)
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, quarter) = (1.0, 1.0, x * x / 4.0);
    for k in 1..64 {
        term *= quarter / (k * k) as f64;
        sum += term;
    }
    sum
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn tone(hertz: f64, rate: f64, count: usize) -> Vec<f32> {
        (0..count).map(|i| (2.0 * PI * hertz * i as f64 / rate).sin() as f32).collect()
    }

    fn convert(input: &[f32], from: u32, to: u32) -> Vec<f32> {
        let mut resampler = Resampler::new(from, to);
        let mut output = Vec::new();
        resampler.process(input, &mut output);
        resampler.finish(&mut output);
        output
    }

    #[test]
    fn ratios_reduce() {
        for (from, up, down) in [(44_100, 160, 147), (22_050, 320, 147), (96_000, 1, 2), (16_000, 3, 1), (11_025, 640, 147)] {
            let resampler = Resampler::new(from, 48_000);
            assert_eq!((resampler.up, resampler.down), (up, down));
        }
    }

    #[test]
    fn output_length_follows_the_ratio() {
        assert_eq!(convert(&vec![0.0; 44_100], 44_100, 48_000).len(), 48_000);
        assert_eq!(convert(&vec![0.0; 44_101], 44_100, 48_000).len(), 48_002);
        assert_eq!(convert(&vec![0.0; 96_000], 96_000, 48_000).len(), 48_000);
        assert_eq!(convert(&[], 44_100, 48_000).len(), 0);
    }

    #[test]
    fn a_tone_keeps_its_phase_and_amplitude_at_every_ratio() {
        for from in [8_000, 16_000, 22_050, 32_000, 44_100, 88_200, 96_000, 192_000] {
            let output = convert(&tone(1_000.0, f64::from(from), from as usize), from, 48_000);
            let expected = tone(1_000.0, 48_000.0, output.len());
            // The first and last 20 ms sit on the song's edges.
            let worst = (960..output.len() - 960).map(|i| (output[i] - expected[i]).abs()).fold(0.0, f32::max);
            assert!(worst < 1e-3, "{from} Hz: {worst}");
        }
    }

    #[test]
    fn the_features_band_passes_flat() {
        // 8 kHz is the highest frequency a mel band reads.
        let output = convert(&tone(8_000.0, 44_100.0, 44_100), 44_100, 48_000);
        let expected = tone(8_000.0, 48_000.0, output.len());
        let worst = (960..output.len() - 960).map(|i| (output[i] - expected[i]).abs()).fold(0.0, f32::max);
        assert!(worst < 1e-3, "{worst}");
    }

    #[test]
    fn an_impulse_lands_where_it_belongs_with_no_delay() {
        // Input sample 4410 is 100 ms in, which is output sample 4800.
        let mut input = vec![0.0; 8_820];
        input[4_410] = 1.0;
        let output = convert(&input, 44_100, 48_000);
        let peak = output.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert_eq!(peak, 4_800);
        // Symmetric about it: the filter is linear phase.
        for d in 1..30 {
            assert!((output[4_800 - d] - output[4_800 + d]).abs() < 1e-5, "{d}");
        }
    }

    #[test]
    fn downsampling_rejects_what_would_fold_into_the_features() {
        // 42 kHz at 96 kHz would fold to 6 kHz at 48 kHz.
        let output = convert(&tone(42_000.0, 96_000.0, 96_000), 96_000, 48_000);
        let worst = (960..output.len() - 960).map(|i| output[i].abs()).fold(0.0, f32::max);
        assert!(worst < 1e-4, "{worst}");
    }

    #[test]
    fn blocks_do_not_change_the_output() {
        let input = tone(997.0, 44_100.0, 10_000);
        let whole = convert(&input, 44_100, 48_000);
        for size in [1, 13, 441, 9_999] {
            let mut resampler = Resampler::new(44_100, 48_000);
            let mut output = Vec::new();
            for block in input.chunks(size) {
                resampler.process(block, &mut output);
            }
            resampler.finish(&mut output);
            assert_eq!(output, whole, "blocks of {size}");
        }
    }

    #[test]
    fn reset_forgets_the_input() {
        let input = tone(997.0, 44_100.0, 2_000);
        let mut resampler = Resampler::new(44_100, 48_000);
        let (mut first, mut second) = (Vec::new(), Vec::new());
        resampler.process(&input, &mut first);
        resampler.reset();
        resampler.process(&input, &mut second);
        assert_eq!(first, second);
    }
}
