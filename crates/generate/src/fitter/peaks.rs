use super::Segment;
use crate::grid::canonical_length;
use crate::math::{argmax, interp, line, max, mean, median, min, modp, pow2, round};

pub struct BeatPeaks {
    pub times: Vec<f64>,
    pub weights: Vec<f64>,
}

pub fn beat_peaks(logits: &[f32], fps: f64) -> BeatPeaks {
    let n = logits.len();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for i in 0..n {
        if logits[i] as f64 <= 0.0 {
            continue;
        }
        let mut peak = f64::NEG_INFINITY;
        for j in i.saturating_sub(3)..=(n - 1).min(i + 3) {
            peak = max(peak, logits[j] as f64);
        }
        if (logits[i] as f64) < peak {
            continue;
        }
        match groups.last_mut() {
            Some(prev) if *prev.last().unwrap() + 1 == i => prev.push(i),
            _ => groups.push(vec![i]),
        }
    }
    let probability: Vec<f64> = logits.iter().map(|&x| 1.0 / (1.0 + libm::exp(-(x as f64)))).collect();
    let (mut times, mut weights) = (Vec::new(), Vec::new());
    for group in &groups {
        let indices: Vec<f64> = group.iter().map(|&i| i as f64).collect();
        let center = round(mean(&indices));
        weights.push(group.iter().fold(f64::NEG_INFINITY, |a, &i| max(a, probability[i])));
        let mut frame = mean(&indices);
        if center >= 2.0 && center + 2.0 < n as f64 {
            let (mut sum, mut w) = (0.0, 0.0);
            let mut i = center - 2.0;
            while i <= center + 2.0 {
                sum += i * probability[i as usize];
                w += probability[i as usize];
                i += 1.0;
            }
            frame = sum / w;
        }
        times.push(frame * 1000.0 / fps);
    }
    BeatPeaks { times, weights }
}

fn refine_one(times: &[f64], weights: &[f64], mut offset: f64, mut length: f64) -> Option<(f64, f64)> {
    if times.is_empty() {
        return None;
    }
    let window = min(0.25 * length, 50.0);
    let residuals = |offset: f64, length: f64| -> Vec<f64> { times.iter().map(|&t| t - (offset + round((t - offset) / length) * length)).collect() };
    let mut residual = residuals(offset, length);
    let mut keep: Vec<usize> = (0..times.len()).filter(|&i| residual[i].abs() < window).collect();
    if keep.len() < 8 || (keep.len() as f64) / (times.len() as f64) < 0.4 {
        return None;
    }
    let fit = |keep: &[usize], offset: f64, length: f64| {
        let x: Vec<f64> = keep.iter().map(|&i| round((times[i] - offset) / length)).collect();
        let y: Vec<f64> = keep.iter().map(|&i| times[i]).collect();
        let w: Vec<f64> = keep.iter().map(|&i| weights[i]).collect();
        line(&x, &y, Some(&w))
    };
    (offset, length) = fit(&keep, offset, length);
    residual = residuals(offset, length);
    let middle = median(&keep.iter().map(|&i| residual[i]).collect::<Vec<_>>());
    let scale = 1.4826 * median(&keep.iter().map(|&i| (residual[i] - middle).abs()).collect::<Vec<_>>());
    keep = (0..times.len()).filter(|&i| residual[i].abs() < min(window, max(3.0 * scale, 5.0))).collect();
    if keep.len() < 8 {
        return Some((offset, length));
    }
    Some(fit(&keep, offset, length))
}

pub fn refine_peaks(segments: &[Segment], logits: &[f32], fps: f64) -> Vec<Segment> {
    let BeatPeaks { times, weights } = beat_peaks(logits, fps);
    let mut refined: Vec<Segment> = segments
        .iter()
        .enumerate()
        .map(|(j, s)| {
            let lo = if j == 0 { f64::NEG_INFINITY } else { s.offset_ms };
            let hi = segments.get(j + 1).map_or(f64::INFINITY, |n| n.offset_ms);
            let ids: Vec<usize> = (0..times.len()).filter(|&i| times[i] >= lo && times[i] < hi).collect();
            let t: Vec<f64> = ids.iter().map(|&i| times[i]).collect();
            let w: Vec<f64> = ids.iter().map(|&i| weights[i]).collect();
            match refine_one(&t, &w, s.offset_ms, s.beat_length_ms) {
                Some((offset_ms, beat_length_ms)) => Segment { offset_ms, beat_length_ms },
                None => *s,
            }
        })
        .collect();
    if refined.len() > 1 {
        let aligned: Vec<f64> = refined.iter().map(|s| s.beat_length_ms * pow2(round(libm::log2(refined[0].beat_length_ms / s.beat_length_ms)))).collect();
        if aligned.iter().all(|x| (x / aligned[0] - 1.0).abs() < 0.003) {
            let spans: Vec<f64> = refined.iter().enumerate().map(|(i, s)| refined.get(i + 1).map_or(logits.len() as f64 * 1000.0 / fps, |n| n.offset_ms) - s.offset_ms).collect();
            let longest = refined[argmax(&spans)];
            if let Some((offset_ms, beat_length_ms)) = refine_one(&times, &weights, longest.offset_ms, longest.beat_length_ms) {
                refined = vec![Segment { offset_ms, beat_length_ms }];
            }
        }
    }
    refined.iter().map(|s| Segment { offset_ms: round(s.offset_ms), ..*s }).collect()
}

pub fn downbeat_phase(segments: &[Segment], beat: &[f32], downbeat: &[f32], fps: f64) -> Vec<Segment> {
    if segments.is_empty() {
        return segments.to_vec();
    }
    let times = beat_peaks(beat, fps).times;
    if times.is_empty() {
        return segments.to_vec();
    }
    let Segment { offset_ms: offset, beat_length_ms: length } = segments[0];
    let period = max(round(4.0 * canonical_length(length) / length), 1.0);
    let end = segments.get(1).map_or(times[times.len() - 1], |s| s.offset_ms);
    let lo = ((times[0] - offset) / length).ceil();
    let hi = ((end - offset) / length).floor();
    if hi - lo + 1.0 < period {
        return segments.to_vec();
    }
    let xp: Vec<f64> = (0..downbeat.len()).map(|i| i as f64 * 1000.0 / fps).collect();
    let prob: Vec<f64> = downbeat.iter().map(|&x| 1.0 / (1.0 + libm::exp(-(x as f64)))).collect();
    let mut scores = vec![0.0; period as usize];
    let mut n = lo;
    while n <= hi {
        scores[modp(n - lo, period) as usize] += interp(offset + n * length, &xp, &prob);
        n += 1.0;
    }
    let mut shift = modp(lo + argmax(&scores) as f64, period);
    if shift > period / 2.0 {
        shift -= period;
    }
    let mut out = vec![Segment { offset_ms: round(offset + shift * length), beat_length_ms: length }];
    out.extend_from_slice(&segments[1..]);
    out
}
