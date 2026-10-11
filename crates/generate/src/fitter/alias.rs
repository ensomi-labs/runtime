use super::scoring::{best_candidate, centered, order, score_grid};
use super::segments::{aliases, phase_error, timing_segments, weighted_score};
use super::{bpm, GridCandidate, Segment, SegmentFit};
use crate::jsmath;
use crate::math::{argmax, mean, modp, round};
use crate::models::FitterConfig;

struct AliasOption {
    fit: SegmentFit,
    local_score: f64,
    has_strong_evidence: bool,
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= f64::max(1e-6, b.abs() * 1e-9)
}

fn semantic_promotion(current: f64, candidate: f64, score: f64, current_score: f64, c: &FitterConfig) -> bool {
    if current <= 0.0 || candidate <= current {
        return false;
    }
    let below = current < c.alias_preferred_min_bpm;
    let trap = current >= c.alias_semantic_promotion_in_band_min_bpm && current <= c.alias_semantic_promotion_current_max_bpm;
    if (!below && !trap) || candidate < c.alias_preferred_min_bpm || candidate > c.alias_preferred_max_bpm || !score.is_finite() {
        return false;
    }
    if !current_score.is_finite() || current_score <= 0.0 {
        return score > 0.0;
    }
    let ratio = score / current_score;
    if below {
        return current_score <= c.alias_semantic_promotion_low_bpm_max_fit_score && ratio >= c.alias_semantic_promotion_score_ratio_threshold;
    }
    if ratio >= c.alias_semantic_promotion_strong_score_ratio_threshold {
        return true;
    }
    current_score <= c.alias_semantic_promotion_low_confidence_max_fit_score && candidate <= c.alias_semantic_promotion_low_confidence_max_candidate_bpm && ratio >= c.alias_semantic_promotion_low_confidence_score_ratio_threshold
}

fn local_score(score: f64, b: f64, down: f64, current: bool, c: &FitterConfig) -> f64 {
    score + (if down.is_finite() { c.alias_downbeat_score_weight * down } else { 0.0 }) + (if b >= c.alias_preferred_min_bpm && b <= c.alias_preferred_max_bpm { c.alias_preferred_band_bonus } else { 0.0 }) + (if current { c.alias_current_tempo_bonus } else { 0.0 })
}

fn support_ratio(fit: &SegmentFit, candidate: &SegmentFit, signal: &[f64], times: &[f64], demotion: bool, c: &FitterConfig) -> f64 {
    let (base, matched) = if demotion { (fit, candidate) } else { (candidate, fit) };
    let lo = ((times[0] - base.offset_ms) / base.beat_length_ms).ceil() as i64;
    let hi = ((times[times.len() - 1] - base.offset_ms) / base.beat_length_ms).floor() as i64;
    let (mut kept, mut changed) = (Vec::new(), Vec::new());
    let step = if times.len() == 1 { c.pulse_width_ms } else { (times[times.len() - 1] - times[0]) / (times.len() - 1) as f64 };
    let radius = (c.pulse_width_ms / step).ceil() + 2.0;
    for n in lo..=hi {
        let t = base.offset_ms + n as f64 * base.beat_length_ms;
        let center = round((t - times[0]) / step);
        let mut support = 0.0;
        let mut j = f64::max(0.0, center - radius);
        let end = f64::min(signal.len() as f64 - 1.0, center + radius);
        while j <= end {
            let ju = j as usize;
            support = f64::max(support, signal[ju] * f64::max(0.0, 1.0 - (times[ju] - t).abs() / c.pulse_width_ms));
            j += 1.0;
        }
        if phase_error(t, matched.offset_ms, matched.beat_length_ms) <= c.alias_beat_match_tolerance_ms {
            kept.push(support);
        } else {
            changed.push(support);
        }
    }
    if kept.is_empty() || changed.is_empty() || mean(&kept) <= 1e-9 { f64::INFINITY } else { mean(&changed) / mean(&kept) }
}

fn options_for(fit: &SegmentFit, signal: &[f64], times: &[f64], downbeat: &[f64], c: &FitterConfig, lower: bool) -> Vec<AliasOption> {
    let raw = &signal[fit.start_frame..fit.end_frame];
    let (s, n) = centered(raw);
    let t = &times[fit.start_frame..fit.end_frame];
    let (d, dn) = centered(&downbeat[fit.start_frame..fit.end_frame]);
    let fit_bpm = bpm(fit.beat_length_ms);
    let original = AliasOption { fit: *fit, local_score: local_score(fit.score, fit_bpm, f64::NEG_INFINITY, true, c), has_strong_evidence: true };
    if n == 0.0 {
        return vec![AliasOption { local_score: fit.score, ..original }];
    }
    let mut options: Vec<(f64, AliasOption)> = vec![(round(fit_bpm * 1e6) / 1e6, original)];
    let base = if fit.raw_bpm.is_finite() && fit.raw_bpm > 0.0 { fit.raw_bpm } else { fit_bpm / fit.tempo_multiplier };
    for &mult in &c.alias_tempo_multipliers {
        let b = base * mult;
        if !lower && dn == 0.0 && (fit.tempo_multiplier - 1.0).abs() > 1e-9 && b < fit_bpm {
            continue;
        }
        if b < c.min_bpm || b > c.max_bpm || !b.is_finite() || close(b, fit_bpm) {
            continue;
        }
        let length = 60000.0 / b;
        let count = if length > fit.beat_length_ms { f64::max(1.0, round(length / fit.beat_length_ms)) as usize } else { 1 };
        let (mut offsets, mut seen): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
        for i in 0..count {
            let o = modp(fit.offset_ms + i as f64 * fit.beat_length_ms, length);
            let key = round(o * 1e6) / 1e6;
            if !seen.contains(&key) {
                seen.push(key);
                offsets.push(o);
            }
        }
        offsets.sort_by(|a, b| order(a - b));
        let candidates: Vec<GridCandidate> = offsets.iter().map(|&o| GridCandidate { score: score_grid(&s, n, t, length, o, c.pulse_width_ms), bpm: b, beat_length_ms: length, offset_ms: o }).collect();
        let (score, ds, _, _, offset) = best_candidate(&candidates, Some(&d), dn, t, c);
        let semantic = semantic_promotion(fit_bpm, b, score, fit.score, c);
        let score_close = !fit.score.is_finite() || score + c.alias_score_tie_margin >= fit.score || (fit.score > 0.0 && score >= fit.score * c.alias_score_ratio_threshold);
        if !score.is_finite() || (!score_close && !semantic) {
            continue;
        }
        let promotion = b > fit_bpm;
        let next = SegmentFit { score, beat_length_ms: length, offset_ms: offset, tempo_multiplier: mult, ..*fit };
        let ratio = support_ratio(fit, &next, raw, t, !promotion, c);
        let strong = semantic || (ratio.is_finite() && if promotion { ratio >= c.alias_promotion_inserted_support_ratio_threshold } else { ratio <= c.alias_demotion_dropped_support_ratio_threshold });
        if !strong {
            continue;
        }
        let local = local_score(score, b, ds, close(b, fit_bpm), c) + if semantic { c.alias_semantic_promotion_bonus } else { 0.0 };
        let key = round(b * 1e6) / 1e6;
        let option = AliasOption { fit: next, local_score: local, has_strong_evidence: strong };
        match options.iter_mut().find(|(k, _)| *k == key) {
            Some((_, old)) => {
                if local > old.local_score {
                    *old = option;
                }
            }
            None => options.push((key, option)),
        }
    }
    let mut out: Vec<AliasOption> = options.into_iter().map(|(_, o)| o).collect();
    out.sort_by(|a, b| order(bpm(a.fit.beat_length_ms) - bpm(b.fit.beat_length_ms)));
    out
}

fn switch_count(segments: &[Segment], c: &FitterConfig) -> usize {
    let mut n = 0;
    for i in 1..segments.len() {
        let (a, b) = (bpm(segments[i - 1].beat_length_ms), bpm(segments[i].beat_length_ms));
        if (a - b).abs() > c.merge_bpm_tolerance && aliases(a, b, c.merge_alias_bpm_tolerance) {
            n += 1;
        }
    }
    n
}

pub fn canonicalize_aliases(fits: &[SegmentFit], signal: &[f64], times: &[f64], downbeat: &[f64], c: &FitterConfig) -> Vec<SegmentFit> {
    let options: Vec<Vec<AliasOption>> = fits.iter().map(|f| options_for(f, signal, times, downbeat, c, fits.len() > 1)).collect();
    let mut scores: Vec<f64> = options[0].iter().map(|x| x.local_score).collect();
    let mut paths: Vec<Vec<usize>> = (0..options[0].len()).map(|i| vec![i]).collect();
    for s in 1..options.len() {
        let (mut next_scores, mut next_paths) = (Vec::new(), Vec::new());
        for i in 0..options[s].len() {
            let mut best = f64::NEG_INFINITY;
            let mut path: Vec<usize> = Vec::new();
            for j in 0..scores.len() {
                let b = bpm(options[s][i].fit.beat_length_ms);
                let prev = bpm(options[s - 1][j].fit.beat_length_ms);
                let penalty = if prev <= 0.0 || b <= 0.0 { 0.0 } else { c.alias_continuity_penalty * jsmath::log2(b / prev).abs() };
                let score = scores[j] + options[s][i].local_score - penalty;
                if score > best {
                    best = score;
                    path = paths[j].iter().copied().chain([i]).collect();
                }
            }
            next_scores.push(best);
            next_paths.push(path);
        }
        scores = next_scores;
        paths = next_paths;
    }
    let path = &paths[argmax(&scores)];
    let proposed: Vec<&AliasOption> = path.iter().enumerate().map(|(i, &k)| &options[i][k]).collect();
    let changed: Vec<&&AliasOption> = proposed.iter().enumerate().filter(|(i, x)| !close(bpm(x.fit.beat_length_ms), bpm(fits[*i].beat_length_ms))).map(|(_, x)| x).collect();
    let next: Vec<SegmentFit> = proposed.iter().map(|x| x.fit).collect();
    if changed.is_empty() {
        return next;
    }
    if changed.iter().any(|x| !x.has_strong_evidence) {
        return fits.to_vec();
    }
    if fits.len() as f64 > c.alias_semantic_promotion_low_bpm_max_segments && proposed.iter().enumerate().any(|(i, x)| bpm(fits[i].beat_length_ms) < c.alias_preferred_min_bpm && bpm(x.fit.beat_length_ms) > bpm(fits[i].beat_length_ms)) {
        return fits.to_vec();
    }
    let original_segments = timing_segments(fits, times, c);
    let next_segments = timing_segments(&next, times, c);
    if next_segments.len() > original_segments.len() {
        return fits.to_vec();
    }
    let (old_score, new_score) = (weighted_score(fits), weighted_score(&next));
    if next_segments.len() < original_segments.len() && (!new_score.is_finite() || (old_score.is_finite() && old_score > 0.0 && new_score < old_score * c.alias_collapse_score_ratio_threshold)) {
        return fits.to_vec();
    }
    if switch_count(&next_segments, c) > switch_count(&original_segments, c) {
        return fits.to_vec();
    }
    let (first_next, first) = (bpm(next[0].beat_length_ms), bpm(fits[0].beat_length_ms));
    if first_next < first && first >= c.alias_preferred_min_bpm && first <= c.alias_preferred_max_bpm {
        return fits.to_vec();
    }
    next
}
