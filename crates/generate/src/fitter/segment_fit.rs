use super::scoring::{best_bpm, best_candidate, candidate_bpms, centered, refine_offset, score_grid};
use super::{GridCandidate, SegmentFit};
use crate::math::median;
use crate::models::FitterConfig;

pub fn fit_segment(signal: &[f64], times: &[f64], downbeat: &[f64], start: usize, end: usize, c: &FitterConfig) -> SegmentFit {
    let (s, n) = centered(&signal[start..end]);
    let t = &times[start..end];
    if n == 0.0 {
        return SegmentFit { start_frame: start, end_frame: end, score: f64::NEG_INFINITY, beat_length_ms: 60000.0 / c.min_bpm, offset_ms: 0.0, raw_bpm: c.min_bpm, tempo_multiplier: 1.0 };
    }
    let (d, dn) = centered(&downbeat[start..end]);
    let mut candidates = Vec::new();
    let step = if t.len() < 2 { 20.0 } else { median(&t[1..].iter().zip(t).map(|(x, y)| x - y).collect::<Vec<_>>()) };
    let fps = if step <= 0.0 { 50.0 } else { 1000.0 / step };
    for b in candidate_bpms(&s, fps, c) {
        let length = 60000.0 / b;
        let mut o = 0.0;
        while o < length {
            candidates.push(GridCandidate { score: score_grid(&s, n, t, length, o, c.pulse_width_ms), bpm: b, beat_length_ms: length, offset_ms: o });
            o += c.offset_step_ms;
        }
    }
    let (mut score, mut ds, mut b, mut length, mut offset) = best_candidate(&candidates, Some(&d), dn, t, c);
    (score, offset, ds) = refine_offset(&s, n, t, length, offset, score, ds, c, Some(&d), dn);
    let (raw_bpm, raw_score, raw_ds) = (b, score, ds);
    let (double, doff, dds) = best_bpm(&s, n, t, raw_bpm * 2.0, c, Some(&d), dn);
    let mut mult = 1.0;
    let rejects = raw_ds.is_finite() && dds.is_finite() && double <= raw_score + c.downbeat_tie_score_margin && raw_ds > dds + c.downbeat_tie_score_margin;
    if double != f64::NEG_INFINITY && double >= raw_score * c.double_tempo_score_ratio_threshold && !rejects {
        b = raw_bpm * 2.0;
        length = 60000.0 / b;
        offset = doff;
        score = double;
        mult = 2.0;
    }
    let _ = b;
    SegmentFit { start_frame: start, end_frame: end, score, beat_length_ms: length, offset_ms: offset, raw_bpm, tempo_multiplier: mult }
}
