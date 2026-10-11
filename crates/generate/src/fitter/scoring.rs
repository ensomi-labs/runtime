//! audio_rows/fitter/scoring.py. Scores and fitting arithmetic are float64.
use super::GridCandidate;
use crate::math::{mean, modp, norm, round};
use crate::models::FitterConfig;
use std::cmp::Ordering;
use std::collections::HashSet;

/// The order a JavaScript comparator that returns `a - b` gives: NaN and zero are ties.
pub fn order(difference: f64) -> Ordering {
    if difference > 0.0 {
        Ordering::Greater
    } else if difference < 0.0 {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

/// Keeps the first of equal values, as a `Set` does.
pub fn unique(values: impl IntoIterator<Item = f64>) -> Vec<f64> {
    let mut seen = HashSet::new();
    values.into_iter().filter(|v| seen.insert(if *v == 0.0 { 0 } else { v.to_bits() })).collect()
}

pub fn centered(signal: &[f64]) -> (Vec<f64>, f64) {
    let m = mean(signal);
    let c: Vec<f64> = signal.iter().map(|x| x - m).collect();
    let n = norm(&c);
    (c, n)
}

pub fn score_grid(signal: &[f64], signal_norm: f64, times: &[f64], length: f64, offset: f64, width: f64) -> f64 {
    let count = times.len();
    if count == 0 {
        return f64::NEG_INFINITY;
    }
    let first = times[0];
    let last = times[count - 1];
    let step = if count == 1 { width } else { (last - first) / (count - 1) as f64 };
    let lo = ((first - offset - width) / length).floor();
    let hi = ((last - offset + width) / length).ceil();
    let radius = (width / step).ceil() + 2.0;
    let (mut s, mut ss, mut dot) = (0.0, 0.0, 0.0);
    if length <= width * 2.0 || (hi - lo + 1.0) * (2.0 * radius + 1.0) >= count as f64 || step <= 0.0 {
        for i in 0..count {
            let phase = modp(times[i] - offset, length);
            let w = f64::max(0.0, 1.0 - f64::min(phase, length - phase) / width);
            s += w;
            ss += w * w;
            dot += signal[i] * w;
        }
    } else {
        let mut b = lo;
        while b <= hi {
            let t = offset + b * length;
            let c = round((t - first) / step);
            let from = f64::max(0.0, c - radius) as usize;
            let to = f64::min((count - 1) as f64, c + radius);
            let mut i = from;
            while (i as f64) <= to {
                let w = 1.0 - (times[i] - t).abs() / width;
                if w > 0.0 {
                    s += w;
                    ss += w * w;
                    dot += signal[i] * w;
                }
                i += 1;
            }
            b += 1.0;
        }
    }
    let v = ss - s * s / count as f64;
    if v <= 0.0 { f64::NEG_INFINITY } else { dot / (signal_norm * v.sqrt()) }
}

fn fractional_parts() -> Vec<f64> {
    let mut parts: Vec<f64> = (0..8).map(|i| (i + 1) as f64 / 9.0).collect();
    parts.extend([1.0, 3.0, 5.0, 7.0].map(|x| x / 8.0));
    parts.push(1.0 / 3.0);
    parts.push(2.0 / 3.0);
    let mut parts = unique(parts);
    parts.sort_by(|a, b| order(a - b));
    parts
}

pub fn quantize_bpm(bpm: f64, c: &FitterConfig) -> f64 {
    c.min_bpm + round((bpm - c.min_bpm) / c.bpm_step) * c.bpm_step
}

pub fn candidate_bpms(signal: &[f64], fps: f64, c: &FitterConfig) -> Vec<f64> {
    let mut scores: Vec<(f64, f64)> = Vec::new();
    let first = (fps * 60.0 / c.max_bpm).ceil() as i64;
    let last = ((fps * 60.0 / c.min_bpm).floor() as i64).min(signal.len() as i64 - 1);
    for lag in first..=last {
        let (mut ll, mut rr, mut lr) = (0.0, 0.0, 0.0);
        for i in 0..signal.len() - lag as usize {
            let l = signal[i];
            let r = signal[i + lag as usize];
            ll += l * l;
            rr += r * r;
            lr += l * r;
        }
        let d = f64::sqrt(ll) * f64::sqrt(rr);
        if d != 0.0 {
            scores.push((lr / d, lag as f64));
        }
    }
    scores.sort_by(|a, b| {
        let d = b.0 - a.0;
        if d != 0.0 && !d.is_nan() { order(d) } else { order(b.1 - a.1) }
    });
    let (mut exact, mut expanded) = (Vec::new(), Vec::new());
    for &(_, lag) in scores.iter().take(c.autocorrelation_candidate_count as usize) {
        for mult in [1.0, 2.0, 0.5, 3.0, 4.0, 1.0 / 3.0, 0.25] {
            let value = 60.0 * fps / lag * mult;
            exact.push(value);
            if value < c.min_bpm || value > c.max_bpm {
                continue;
            }
            let radius = f64::max(c.bpm_search_window_min_bpm, value * c.bpm_search_window_ratio);
            let lo = f64::max(c.min_bpm, quantize_bpm(value - radius, c));
            let hi = f64::min(c.max_bpm, quantize_bpm(value + radius, c));
            let mut v = lo;
            while v < hi + c.bpm_step * 0.5 {
                expanded.push(v);
                v += c.bpm_step;
            }
        }
    }
    let mut unique_bpms = unique(exact.iter().chain(&expanded).map(|&x| quantize_bpm(x, c)).filter(|&x| x >= c.min_bpm && x <= c.max_bpm));
    if unique_bpms.is_empty() {
        let mut x = c.min_bpm;
        while x < c.max_bpm + c.bpm_step * 0.5 {
            unique_bpms.push(x);
            x += c.bpm_step;
        }
    }
    let mut count = 0.0;
    let mut limited = Vec::new();
    for &value in &unique_bpms {
        let n = f64::max(1.0, (60000.0 / value / c.offset_step_ms).ceil());
        if !limited.is_empty() && count + n > c.max_grid_candidates_per_segment {
            continue;
        }
        limited.push(value);
        count += n;
    }
    let fractions = fractional_parts();
    let mut result = Vec::new();
    for &value in &limited {
        let mut candidates = vec![value];
        if value >= 80.0 {
            for i in [value.floor() - 1.0, value.floor(), value.floor() + 1.0] {
                for &f in &fractions {
                    if (i + f - value).abs() <= 0.25 {
                        candidates.push(i + f);
                    }
                }
            }
        }
        candidates.sort_by(|a, b| {
            let d = (a - value).abs() - (b - value).abs();
            if d != 0.0 && !d.is_nan() { order(d) } else { order(a - b) }
        });
        for v in candidates {
            let rounded = round(v * 1e6) / 1e6;
            if rounded >= c.min_bpm && rounded <= c.max_bpm {
                result.push(rounded);
            }
        }
    }
    unique(result)
}

pub fn better(score: f64, downbeat: f64, best: f64, best_downbeat: f64, c: &FitterConfig) -> bool {
    let margin = c.downbeat_tie_score_margin;
    if score > best + margin {
        return true;
    }
    if score < best - margin {
        return false;
    }
    if downbeat.is_finite() || best_downbeat.is_finite() {
        if downbeat > best_downbeat + margin {
            return true;
        }
        if downbeat < best_downbeat - margin {
            return false;
        }
    }
    score > best
}

/// The best downbeat score over the phases of a four-beat bar, and the offset that gave it.
pub fn best_downbeat(signal: Option<&[f64]>, signal_norm: f64, times: &[f64], length: f64, offset: f64, c: &FitterConfig) -> (f64, f64) {
    let signal = match signal {
        Some(s) if signal_norm != 0.0 => s,
        _ => return (f64::NEG_INFINITY, offset),
    };
    let (mut best, mut at) = (f64::NEG_INFINITY, offset);
    for j in 0..c.downbeat_period_beats as usize {
        let o = offset + j as f64 * length;
        let s = score_grid(signal, signal_norm, times, length * c.downbeat_period_beats, o, c.pulse_width_ms);
        if s > best {
            best = s;
            at = o;
        }
    }
    (best, at)
}

/// Score, downbeat score, bpm, beat length and offset of the best candidate.
pub fn best_candidate(candidates: &[GridCandidate], downbeat: Option<&[f64]>, down_norm: f64, times: &[f64], c: &FitterConfig) -> (f64, f64, f64, f64, f64) {
    if candidates.is_empty() {
        return (f64::NEG_INFINITY, f64::NEG_INFINITY, c.min_bpm, 60000.0 / c.min_bpm, 0.0);
    }
    let mut sorted: Vec<&GridCandidate> = candidates.iter().collect();
    sorted.sort_by(|a, b| order(b.score - a.score));
    let mut best = sorted[0];
    if (quantize_bpm(best.bpm, c) - best.bpm).abs() > 1e-6 {
        if let Some(coarse) = sorted.iter().find(|x| (quantize_bpm(x.bpm, c) - x.bpm).abs() <= 1e-6) {
            if best.score < coarse.score + 0.08 {
                best = coarse;
            }
        }
    }
    if downbeat.is_none() || down_norm == 0.0 {
        return (best.score, f64::NEG_INFINITY, best.bpm, best.beat_length_ms, best.offset_ms);
    }
    let (mut score, mut ds, mut b, mut length, mut offset) = (f64::NEG_INFINITY, f64::NEG_INFINITY, best.bpm, best.beat_length_ms, best.offset_ms);
    for candidate in sorted.iter().take(c.downbeat_refine_candidate_count as usize) {
        let (d, o) = best_downbeat(downbeat, down_norm, times, candidate.beat_length_ms, candidate.offset_ms, c);
        if better(candidate.score, d, score, ds, c) {
            score = candidate.score;
            ds = d;
            b = candidate.bpm;
            length = candidate.beat_length_ms;
            offset = o;
        }
    }
    (score, ds, b, length, offset)
}

/// Refines the offset by `offset_refine_step_ms` around `offset`; returns score, offset and downbeat score.
#[allow(clippy::too_many_arguments)]
pub fn refine_offset(signal: &[f64], signal_norm: f64, times: &[f64], length: f64, mut offset: f64, mut score: f64, mut ds: f64, c: &FitterConfig, downbeat: Option<&[f64]>, down_norm: f64) -> (f64, f64, f64) {
    let step = c.offset_refine_step_ms;
    let center = round(offset / step) * step;
    let count = (c.offset_refine_radius_ms / step).ceil() as i64;
    for i in -count..=count {
        let o = center + i as f64 * step;
        let s = score_grid(signal, signal_norm, times, length, o, c.pulse_width_ms);
        let d = match downbeat {
            Some(d) if down_norm != 0.0 => score_grid(d, down_norm, times, length * c.downbeat_period_beats, o, c.pulse_width_ms),
            _ => f64::NEG_INFINITY,
        };
        if better(s, d, score, ds, c) {
            score = s;
            ds = d;
            offset = o;
        }
    }
    (score, offset, ds)
}

/// The best offset for one tempo: score, offset and downbeat score.
pub fn best_bpm(signal: &[f64], signal_norm: f64, times: &[f64], bpm: f64, c: &FitterConfig, downbeat: Option<&[f64]>, down_norm: f64) -> (f64, f64, f64) {
    if bpm < c.min_bpm || bpm > c.max_bpm {
        return (f64::NEG_INFINITY, 0.0, f64::NEG_INFINITY);
    }
    let length = 60000.0 / bpm;
    let mut candidates = Vec::new();
    let mut offset = 0.0;
    while offset < length {
        candidates.push(GridCandidate { score: score_grid(signal, signal_norm, times, length, offset, c.pulse_width_ms), bpm, beat_length_ms: length, offset_ms: offset });
        offset += c.offset_step_ms;
    }
    let (s, d, _, _, o) = best_candidate(&candidates, downbeat, down_norm, times, c);
    refine_offset(signal, signal_norm, times, length, o, s, d, c, downbeat, down_norm)
}
