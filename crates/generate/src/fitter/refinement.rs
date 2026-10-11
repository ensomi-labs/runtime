use super::scoring::{best_bpm, centered, score_grid};
use super::segments::{merge_segments, nearest_offset, phase_error};
use super::{bpm, Segment};
use crate::jsmath;
use crate::math::{argmax, lower_bound, modp, round, sum};
use crate::models::FitterConfig;

struct Family {
    bpm: f64,
    indices: Vec<usize>,
    weight: f64,
    bpm_sum: f64,
}

/// The single dominant tempo of a song that the fit split into many, or None if it does not apply.
fn collapse(segments: &[Segment], times: &[f64], c: &FitterConfig) -> Option<Vec<Segment>> {
    if (segments.len() as f64) < c.refine_dominant_min_segments {
        return None;
    }
    let first = times[0];
    let last = times[times.len() - 1];
    let durations: Vec<f64> = segments
        .iter()
        .enumerate()
        .map(|(i, s)| f64::max(0.0, f64::min(last, segments.get(i + 1).map_or(last, |n| n.offset_ms)) - if i == 0 { first } else { f64::max(first, s.offset_ms) }))
        .collect();
    let total = sum(&durations);
    if total <= 0.0 {
        return None;
    }
    let mut families: Vec<Family> = Vec::new();
    for (i, s) in segments.iter().enumerate() {
        let b = bpm(s.beat_length_ms);
        let duration = durations[i];
        let found = families.iter_mut().find(|f| {
            let center = if f.weight <= 0.0 { b } else { f.bpm_sum / f.weight };
            (b - center).abs() <= f64::max(c.refine_dominant_bpm_tolerance, f64::min(b, center) * c.merge_relative_bpm_tolerance)
        });
        match found {
            Some(f) => {
                f.indices.push(i);
                f.weight += duration;
                f.bpm_sum += b * duration;
                f.bpm = if f.weight <= 0.0 { bpm(segments[f.indices[0]].beat_length_ms) } else { f.bpm_sum / f.weight };
            }
            None => families.push(Family { bpm: b, indices: vec![i], weight: duration, bpm_sum: b * duration }),
        }
    }
    if families.len() < 2 {
        return None;
    }
    let dominant = &families[argmax(&families.iter().map(|f| f.weight).collect::<Vec<_>>())];
    let indices = &dominant.indices;
    let far_tol = f64::max(c.refine_dominant_far_bpm_tolerance, dominant.bpm * c.refine_dominant_far_relative_bpm_tolerance);
    let far = segments.iter().enumerate().filter(|(i, s)| !indices.contains(i) && (bpm(s.beat_length_ms) - dominant.bpm).abs() > far_tol).count();
    let off = segments[1..].iter().enumerate().filter(|(i, s)| phase_error(s.offset_ms, segments[*i].offset_ms, segments[*i].beat_length_ms) > c.refine_dominant_boundary_phase_tolerance_ms).count();
    if (indices.len() as f64) < c.refine_dominant_min_segment_count
        || ((segments.len() - indices.len()) as f64) < c.refine_dominant_min_outlier_segments
        || (far as f64) < c.refine_dominant_min_far_outlier_segments
        || dominant.weight / total < c.refine_dominant_min_duration_ratio
    {
        return None;
    }
    if (f64::max(first, segments[indices[0]].offset_ms) - first) / f64::max(1.0, last - first) > c.refine_dominant_max_first_anchor_ratio || (off as f64) < c.refine_dominant_min_off_lattice_transitions || indices[indices.len() - 1] - indices[0] + 1 == indices.len() {
        return None;
    }
    let b = if (dominant.bpm - round(dominant.bpm)).abs() <= c.refine_dominant_bpm_snap_tolerance { round(dominant.bpm) } else { dominant.bpm };
    let length = 60000.0 / b;
    let (mut x, mut y, mut weight) = (0.0, 0.0, 0.0);
    let (mut longest, mut longest_duration) = (indices[0], -1.0);
    for &i in indices {
        let w = durations[i];
        let a = 2.0 * std::f64::consts::PI * modp(segments[i].offset_ms, length) / length;
        x += w * jsmath::cos(a);
        y += w * jsmath::sin(a);
        weight += w;
        if w > longest_duration {
            longest = i;
            longest_duration = w;
        }
    }
    let mut offset = modp(segments[longest].offset_ms, length);
    if weight > 0.0 && jsmath::hypot(&[x, y]) / weight >= c.refine_circular_phase_min_coherence {
        offset = modp(jsmath::atan2(y, x), 2.0 * std::f64::consts::PI) / (2.0 * std::f64::consts::PI) * length;
    }
    Some(vec![Segment { offset_ms: nearest_offset(offset, length, segments[0].offset_ms), beat_length_ms: length }])
}

fn grid_score(segments: &[Segment], times: &[f64], signal: &[f64], c: &FitterConfig) -> Option<f64> {
    let (mut weighted, mut count) = (0.0, 0.0);
    for (i, s) in segments.iter().enumerate() {
        let start = lower_bound(times, if i == 0 { times[0] } else { f64::max(times[0], s.offset_ms) });
        let end = lower_bound(times, f64::min(times[times.len() - 1] + 1.0, segments.get(i + 1).map_or(f64::INFINITY, |n| n.offset_ms)));
        if end < start + 2 {
            continue;
        }
        let (v, n) = centered(&signal[start..end]);
        if n == 0.0 {
            continue;
        }
        let score = score_grid(&v, n, &times[start..end], s.beat_length_ms, s.offset_ms, c.pulse_width_ms);
        if !score.is_finite() {
            continue;
        }
        weighted += score * (end - start) as f64;
        count += (end - start) as f64;
    }
    if count <= 0.0 { None } else { Some(weighted / count) }
}

pub fn refine_timing(segments: &[Segment], times: &[f64], signal: &[f64], c: &FitterConfig) -> Vec<Segment> {
    let mut collapsed = collapse(segments, times, c);
    if let Some(one) = collapsed.as_mut().filter(|s| s.len() == 1) {
        let (s, n) = centered(signal);
        if n != 0.0 {
            let (_, offset, _) = best_bpm(&s, n, times, bpm(one[0].beat_length_ms), c, None, 0.0);
            one[0].offset_ms = offset;
        }
    }
    let mut refined = segments.to_vec();
    if let Some(next_segments) = collapsed {
        let old = grid_score(segments, times, signal, c);
        let next = grid_score(&next_segments, times, signal, c);
        if old.is_none() || next.is_none() || next.unwrap() >= old.unwrap() - c.refine_structural_max_score_loss {
            refined = next_segments;
        }
    }
    refined = merge_segments(&refined, c);
    let mut previous = f64::NEG_INFINITY;
    let quantum = c.refine_offset_quantum_ms;
    let quantized: Vec<Segment> = refined
        .iter()
        .map(|s| {
            let mut offset = round(s.offset_ms / quantum) * quantum;
            if offset <= previous {
                offset = previous + quantum;
            }
            previous = offset;
            Segment { offset_ms: offset, ..*s }
        })
        .collect();
    merge_segments(&quantized, c)
}
