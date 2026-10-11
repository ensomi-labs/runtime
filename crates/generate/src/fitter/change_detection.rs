use super::scoring::{centered, order};
use super::segments::phase_error;
use super::SegmentFit;
use crate::math::{self, lower_bound, max, mean, median, min, round};
use crate::models::FitterConfig;

struct Change {
    time_ms: f64,
    score: f64,
}

pub struct SplitCandidate {
    pub frame: usize,
    pub score: f64,
}

pub fn peak_times(signal: &[f64], times: &[f64], c: &FitterConfig) -> Vec<f64> {
    if signal.len() < 3 {
        return Vec::new();
    }
    let m = mean(signal);
    let sd = mean(&signal.iter().map(|x| (x - m) * (x - m)).collect::<Vec<_>>()).sqrt();
    let peak = signal.iter().fold(f64::NEG_INFINITY, |a, &b| max(a, b));
    let threshold = f64::max(m + sd, peak * 0.35);
    let mut chosen: Vec<usize> = Vec::new();
    for i in 1..signal.len() - 1 {
        if signal[i] >= signal[i - 1] && signal[i] > signal[i + 1] && signal[i] >= threshold {
            match chosen.last().copied() {
                Some(last) if times[i] - times[last] < c.offset_step_ms => {
                    if signal[i] > signal[last] {
                        *chosen.last_mut().unwrap() = i;
                    }
                }
                _ => chosen.push(i),
            }
        }
    }
    chosen.into_iter().map(|i| times[i]).collect()
}

fn boundary(peaks: &[f64], index: usize, downbeats: &[f64], fit: &SegmentFit) -> f64 {
    let time = peaks[index];
    let before = peaks[index.saturating_sub(1)];
    let after = peaks[(peaks.len() - 1).min(index + 1)];
    let lo = min(before, time) - fit.beat_length_ms * 0.25;
    let hi = max(after, time) + fit.beat_length_ms * 0.25;
    let mut nearby = downbeats.iter().copied().filter(|&x| x >= lo && x <= hi);
    match nearby.next() {
        None => time,
        Some(first) => nearby.fold(first, |a, b| if (b - time).abs() < (a - time).abs() { b } else { a }),
    }
}

fn merge_changes(mut candidates: Vec<Change>, distance: f64) -> Vec<Change> {
    candidates.sort_by(|a, b| order(a.time_ms - b.time_ms));
    let mut out: Vec<Change> = Vec::new();
    for v in candidates {
        match out.last_mut() {
            Some(last) if v.time_ms - last.time_ms < distance => {
                if v.score > last.score {
                    *last = v;
                }
            }
            _ => out.push(v),
        }
    }
    out
}

pub fn detect_changes(signal: &[f64], times: &[f64], downbeat: &[f64], fit: &SegmentFit, c: &FitterConfig) -> Vec<SplitCandidate> {
    let s = &signal[fit.start_frame..fit.end_frame];
    let d = &downbeat[fit.start_frame..fit.end_frame];
    let t = &times[fit.start_frame..fit.end_frame];
    let dn = centered(d).1;
    let detection: Vec<f64> = if dn == 0.0 { s.to_vec() } else { s.iter().zip(d).map(|(x, y)| x + y * c.split_downbeat_signal_weight).collect() };
    let peaks = peak_times(&detection, t, c);
    if peaks.len() < 4 {
        return Vec::new();
    }
    let step = if times.len() < 2 { 20.0 } else { median(&times[1..].iter().zip(times).map(|(x, y)| x - y).collect::<Vec<_>>()) };
    let min_frames = f64::max(1.0, round(c.min_segment_duration_ms / step));
    let downbeats = if dn == 0.0 { Vec::new() } else { peak_times(d, t, c) };
    let intervals: Vec<f64> = peaks[1..].iter().zip(&peaks).map(|(x, y)| x - y).collect();
    let phase: Vec<f64> = peaks.iter().map(|&x| phase_error(x, fit.offset_ms, fit.beat_length_ms)).collect();
    let (mut candidates, mut supers) = (Vec::new(), Vec::new());
    let threshold = f64::max(c.offset_step_ms, c.split_phase_change_threshold_ms * 2.0);
    let n = intervals.len() as i64;
    let mut i = 4;
    while i <= n - 4 {
        let u = i as usize;
        let change = (median(&intervals[u..u + 4]) - median(&intervals[u - 4..u])).abs();
        let before = median(&phase[u - 4..u]);
        let after = median(&phase[u..u + 4]);
        let pc = (after - before).abs();
        let residual = f64::max(before, after);
        let score = change / threshold + pc / f64::max(c.split_phase_change_threshold_ms, 1e-6) + residual / f64::max(fit.beat_length_ms, 1e-6);
        if change + 1e-9 >= threshold || pc >= c.split_phase_change_threshold_ms || residual >= c.split_phase_change_threshold_ms * 2.0 {
            candidates.push(Change { time_ms: boundary(&peaks, u, &downbeats, fit), score });
        }
        i += 1;
    }
    let strengths: Vec<f64> = peaks.iter().map(|&x| signal[lower_bound(times, x).min(signal.len() - 1)]).collect();
    let min_change = f64::max(c.split_relative_interval_change_threshold, 1e-9);
    let mut i = 3;
    while i <= n - 3 {
        let u = i as usize;
        let before = median(&intervals[u - 3..u]);
        let after = median(&intervals[u..u + 3]);
        i += 1;
        if before <= 0.0 || after <= 0.0 {
            continue;
        }
        let u = u;
        let change = (after - before).abs();
        let relative = change / f64::max(f64::min(before, after), 1e-6);
        if relative < min_change {
            continue;
        }
        let residual = f64::max(median(&phase[u - 3..u]), median(&phase[u..u + 3]));
        let score = relative / min_change + change / f64::max(c.offset_step_ms, 1e-6) + residual / f64::max(c.split_phase_change_threshold_ms, 1e-6) + 0.25 * (mean(&strengths[u - 3..u + 1]) + mean(&strengths[u..u + 4]) + strengths[u]);
        supers.push(Change { time_ms: boundary(&peaks, u, &downbeats, fit), score });
    }
    let merged = merge_changes(candidates, c.min_segment_duration_ms);
    let extra: Vec<Change> = merge_changes(supers, c.min_segment_duration_ms).into_iter().filter(|x| !merged.iter().any(|y| (x.time_ms - y.time_ms).abs() < c.min_segment_duration_ms)).collect();
    let mut by_frame: Vec<SplitCandidate> = Vec::new();
    for v in merged.iter().chain(&extra) {
        let frame = lower_bound(times, v.time_ms);
        if ((frame as f64) - (fit.start_frame as f64)) < min_frames || ((fit.end_frame as f64) - (frame as f64)) < min_frames {
            continue;
        }
        let mut bonus = 0.0;
        if !downbeats.is_empty() {
            let distance = downbeats.iter().fold(f64::INFINITY, |a, &x| math::min(a, (x - v.time_ms).abs()));
            let tol = f64::min(fit.beat_length_ms * 0.5, c.min_segment_duration_ms * 0.125);
            if distance <= tol {
                bonus = c.downbeat_split_score_bonus * (1.0 - distance / f64::max(tol, 1e-6));
            }
        }
        let score = v.score + bonus;
        match by_frame.iter_mut().find(|old| old.frame == frame) {
            Some(old) => {
                if score > old.score {
                    old.score = score;
                }
            }
            None => by_frame.push(SplitCandidate { frame, score }),
        }
    }
    by_frame.sort_by(|a, b| order(b.score - a.score));
    by_frame
}
