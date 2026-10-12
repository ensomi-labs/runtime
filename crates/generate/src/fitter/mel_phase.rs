use super::peaks::beat_peaks;
use super::Segment;
use crate::grid::corrected_grid;
use crate::math::{argmax, clip, line, lower_bound, mean, median, round, pow2};

fn onset_envelope(log_mel: &[f32]) -> Vec<f64> {
    let mut out = vec![0.0; log_mel.len() / 128];
    for f in 1..out.len() {
        let mut sum = 0.0;
        for m in 0..128 {
            sum += f64::max(0.0, log_mel[f * 128 + m] as f64 - log_mel[(f - 1) * 128 + m] as f64);
        }
        out[f] = sum;
    }
    out
}

fn onset_times(env: &[f64], times: &[f64], window: f64) -> Vec<f64> {
    let peaks: Vec<usize> = (1..env.len().saturating_sub(1)).filter(|&i| env[i] > env[i - 1] && env[i] >= env[i + 1]).collect();
    let at: Vec<f64> = peaks.iter().map(|&i| (i * 10 + 15) as f64).collect();
    times
        .iter()
        .map(|&t| {
            let lo = lower_bound(&at, t - window);
            let hi = lower_bound(&at, t + window);
            if lo == hi {
                return f64::NAN;
            }
            let mut best = lo;
            for j in lo + 1..hi {
                if env[peaks[j]] > env[peaks[best]] {
                    best = j;
                }
            }
            let i = peaks[best];
            let (a, b, c) = (env[i - 1], env[i], env[i + 1]);
            let curvature = a - 2.0 * b + c;
            let delta = if curvature.abs() > 1e-12 { clip(0.5 * (a - c) / curvature, -0.5, 0.5) } else { 0.0 };
            (i * 10 + 15) as f64 + 10.0 * delta
        })
        .collect()
}

/// The beat times of the grid between `lo` and `hi`, with the segment each belongs to.
fn beats(grid: &[Segment], lo: f64, hi: f64) -> (Vec<f64>, Vec<usize>) {
    let (mut times, mut indices) = (Vec::new(), Vec::new());
    for (j, s) in grid.iter().enumerate() {
        let end = grid.get(j + 1).map_or(f64::max(hi, s.offset_ms) + s.beat_length_ms, |n| n.offset_ms);
        let first = if j == 0 { (f64::min(lo - grid[0].offset_ms, 0.0) / grid[0].beat_length_ms).floor() } else { 0.0 };
        let last = if j < grid.len() - 1 { ((end - s.offset_ms) / s.beat_length_ms - 1e-9).ceil() - 1.0 } else { ((end - s.offset_ms) / s.beat_length_ms).ceil() };
        let mut n = first;
        while n <= last {
            let t = s.offset_ms + n * s.beat_length_ms;
            if t >= lo && t <= hi {
                times.push(t);
                indices.push(j);
            }
            n += 1.0;
        }
    }
    (times, indices)
}

fn order_by_offset(segments: &[Segment]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by(|&a, &b| super::scoring::order(segments[a].offset_ms - segments[b].offset_ms));
    order
}

pub fn mel_phase(segments: &[Segment], mel: &[f32], logits: &[f32], song_ms: f64) -> Vec<Segment> {
    let peaks = beat_peaks(logits, 50.0).times;
    if peaks.is_empty() {
        return segments.to_vec();
    }
    let order = order_by_offset(segments);
    let grid = corrected_grid(segments, song_ms);
    let (times, indices) = beats(&grid, peaks[0] - 30.0, peaks[peaks.len() - 1] - 30.0);
    let onsets = onset_times(&onset_envelope(mel), &times.iter().map(|t| t + 16.0).collect::<Vec<_>>(), 60.0);
    let offsets: Vec<f64> = onsets.iter().zip(&times).map(|(t, b)| t - b).collect();
    let found: Vec<f64> = offsets.iter().copied().filter(|x| x.is_finite()).collect();
    if found.len() < 8 {
        return segments.to_vec();
    }
    let song = median(&found);
    let mut shifts: Vec<f64> = (0..grid.len())
        .map(|j| {
            let values: Vec<f64> = offsets.iter().enumerate().filter(|(i, x)| x.is_finite() && indices[*i] == j).map(|(_, &x)| x).collect();
            (if values.len() >= 32 { median(&values) } else { song }) - 14.873
        })
        .collect();
    if grid.len() != segments.len() {
        shifts = vec![shifts[0]; segments.len()];
    }
    let mut out = segments.to_vec();
    for (j, &k) in order.iter().enumerate() {
        out[k] = Segment { offset_ms: segments[k].offset_ms + shifts[j], ..segments[k] };
    }
    out
}

fn drift(x: &[f64], y: &[f64]) -> Option<(f64, f64)> {
    let med = median(y);
    let mut keep: Vec<usize> = (0..y.len()).filter(|&i| (y[i] - med).abs() <= 25.0).collect();
    if keep.len() < 8 {
        return None;
    }
    let (mut a, mut b) = (0.0, 0.0);
    for _ in 0..2 {
        (a, b) = line(&keep.iter().map(|&i| x[i]).collect::<Vec<_>>(), &keep.iter().map(|&i| y[i]).collect::<Vec<_>>(), None);
        let residual: Vec<f64> = (0..y.len()).map(|i| y[i] - a - b * x[i]).collect();
        let middle = median(&keep.iter().map(|&i| residual[i]).collect::<Vec<_>>());
        let bound = f64::max(3.0, 3.0 * 1.4826 * median(&keep.iter().map(|&i| (residual[i] - middle).abs()).collect::<Vec<_>>()));
        let next: Vec<usize> = (0..y.len()).filter(|&i| residual[i].abs() <= bound).collect();
        if next.len() < 8 {
            break;
        }
        keep = next;
    }
    Some((a, b))
}

pub fn mel_tempo(segments: &[Segment], mel: &[f32], logits: &[f32], song_ms: f64) -> Vec<Segment> {
    let peaks = beat_peaks(logits, 50.0).times;
    if peaks.is_empty() {
        return segments.to_vec();
    }
    let order = order_by_offset(segments);
    let grid = corrected_grid(segments, song_ms);
    let lo = peaks[0] - 30.0;
    let hi = peaks[peaks.len() - 1] - 30.0;
    let (times, indices) = beats(&grid, lo, hi);
    let onsets = onset_times(&onset_envelope(mel), &times.iter().map(|t| t + 16.0).collect::<Vec<_>>(), 60.0);
    let offsets: Vec<f64> = onsets.iter().zip(&times).map(|(t, b)| t - b).collect();
    let mut out = grid.clone();
    let mut refitted = false;
    for j in 0..grid.len() {
        let ids: Vec<usize> = (0..times.len()).filter(|&i| offsets[i].is_finite() && indices[i] == j).collect();
        if ids.len() < 16 {
            continue;
        }
        let center = mean(&ids.iter().map(|&i| times[i]).collect::<Vec<_>>());
        let Some((a, b)) = drift(&ids.iter().map(|&i| times[i] - center).collect::<Vec<_>>(), &ids.iter().map(|&i| offsets[i]).collect::<Vec<_>>()) else { continue };
        let local: Vec<f64> = (0..times.len()).filter(|&i| indices[i] == j).map(|i| times[i]).collect();
        let span = local.iter().fold(f64::NEG_INFINITY, |a, &x| f64::max(a, x)) - local.iter().fold(f64::INFINITY, |a, &x| f64::min(a, x));
        if b.abs() * span < 3.0 || b.abs() > 0.03 || grid[j].beat_length_ms == 750.0 {
            continue;
        }
        let start = grid[j].offset_ms + a - 14.873 + b * (grid[j].offset_ms - center);
        if (j > 0 && start <= out[j - 1].offset_ms) || (j + 1 < grid.len() && start >= grid[j + 1].offset_ms) {
            continue;
        }
        out[j] = Segment { offset_ms: start, beat_length_ms: grid[j].beat_length_ms * (1.0 + b) };
        refitted = true;
    }
    if !refitted {
        return segments.to_vec();
    }
    if grid.len() == 1 {
        let sorted: Vec<Segment> = order.iter().map(|&i| segments[i]).collect();
        let spans: Vec<f64> = sorted.iter().enumerate().map(|(i, s)| sorted.get(i + 1).map_or(f64::max(song_ms, sorted[sorted.len() - 1].offset_ms + 1.0), |n| n.offset_ms) - s.offset_ms).collect();
        let octave = pow2(round(libm::log2(sorted[argmax(&spans)].beat_length_ms / grid[0].beat_length_ms)));
        let bpm = 60000.0 / (out[0].beat_length_ms * octave);
        if (bpm - round(bpm)).abs() <= 0.25 {
            let length = 60000.0 / round(bpm) / octave;
            let middle = 0.5 * (f64::max(out[0].offset_ms, lo) + hi);
            out[0] = Segment { offset_ms: out[0].offset_ms + round((middle - out[0].offset_ms) / out[0].beat_length_ms) * (out[0].beat_length_ms - length), beat_length_ms: length };
        }
        return vec![Segment { offset_ms: out[0].offset_ms + 30.0, beat_length_ms: out[0].beat_length_ms * octave }];
    }
    let mut result = segments.to_vec();
    for (j, &k) in order.iter().enumerate() {
        result[k] = Segment { offset_ms: out[j].offset_ms + 30.0, beat_length_ms: segments[k].beat_length_ms * out[j].beat_length_ms / grid[j].beat_length_ms };
    }
    result
}
