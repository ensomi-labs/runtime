use super::{bpm, Segment, SegmentFit};
use crate::math::{modp, round};
use crate::models::FitterConfig;

pub fn phase_error(time: f64, offset: f64, length: f64) -> f64 {
    let phase = modp(time - offset, length);
    f64::min(phase, length - phase)
}

pub fn nearest_offset(offset: f64, length: f64, target: f64) -> f64 {
    offset + round((target - offset) / length) * length
}

pub fn aliases(a: f64, b: f64, tolerance: f64) -> bool {
    a > 0.0 && b > 0.0 && [0.25, 1.0 / 3.0, 0.5, 2.0, 3.0, 4.0].iter().any(|m| (a * m - b).abs() <= tolerance)
}

pub fn mergeable(a: &Segment, b: &Segment, c: &FitterConfig, allow_alias: bool, loose: bool) -> bool {
    let (ba, bb) = (bpm(a.beat_length_ms), bpm(b.beat_length_ms));
    let mut tolerance = f64::max(c.merge_bpm_tolerance, f64::min(ba, bb) * c.merge_relative_bpm_tolerance);
    if loose {
        tolerance = f64::max(tolerance, c.merge_many_similar_bpm_tolerance);
    }
    if (ba - bb).abs() <= tolerance {
        return loose || phase_error(b.offset_ms, a.offset_ms, a.beat_length_ms) <= c.merge_phase_tolerance_ms;
    }
    allow_alias && aliases(ba, bb, c.merge_alias_bpm_tolerance) && phase_error(b.offset_ms, a.offset_ms, f64::min(a.beat_length_ms, b.beat_length_ms)) <= c.merge_alias_phase_tolerance_ms
}

pub fn raw_segments(fits: &[SegmentFit], times: &[f64]) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for fit in fits {
        let mut o = fit.offset_ms;
        if let Some(last) = out.last() {
            o = nearest_offset(o, fit.beat_length_ms, times[fit.start_frame]);
            while o <= last.offset_ms {
                o += fit.beat_length_ms;
            }
        }
        out.push(Segment { offset_ms: o, beat_length_ms: fit.beat_length_ms });
    }
    out
}

pub fn merge_segments(segments: &[Segment], c: &FitterConfig) -> Vec<Segment> {
    let mut out = vec![segments[0]];
    for s in &segments[1..] {
        if !mergeable(out.last().unwrap(), s, c, false, false) {
            out.push(*s);
        }
    }
    out
}

pub fn timing_segments(fits: &[SegmentFit], times: &[f64], c: &FitterConfig) -> Vec<Segment> {
    merge_segments(&raw_segments(fits, times), c)
}

pub fn fits_mergeable(a: &SegmentFit, b: &SegmentFit, times: &[f64], c: &FitterConfig, alias: bool, loose: bool) -> bool {
    let s = raw_segments(&[*a, *b], times);
    mergeable(&s[0], &s[1], c, alias, loose)
}

pub fn weighted_score(fits: &[SegmentFit]) -> f64 {
    let (mut weight, mut score) = (0.0, 0.0);
    for s in fits {
        weight += s.frame_count();
        score += s.score * s.frame_count();
    }
    if weight > 0.0 { score / weight } else { f64::NEG_INFINITY }
}
