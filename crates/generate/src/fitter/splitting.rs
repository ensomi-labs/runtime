use super::change_detection::{detect_changes, SplitCandidate};
use super::scoring::centered;
use super::segment_fit::fit_segment;
use super::segments::{fits_mergeable, weighted_score};
use super::SegmentFit;
use crate::models::FitterConfig;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

struct EvaluatedSplit {
    segment_index: usize,
    candidate: SplitCandidate,
    left_fit: SegmentFit,
    right_fit: SegmentFit,
    improvement: f64,
}

fn split_better(a: &EvaluatedSplit, b: &EvaluatedSplit) -> bool {
    a.improvement > b.improvement || (a.improvement == b.improvement && a.candidate.score > b.candidate.score)
}

struct Fits<'a> {
    signal: &'a [f64],
    times: &'a [f64],
    downbeat: &'a [f64],
    c: &'a FitterConfig,
    cache: HashMap<(usize, usize), SegmentFit>,
}

impl Fits<'_> {
    fn fit(&mut self, start: usize, end: usize) -> SegmentFit {
        if let Some(f) = self.cache.get(&(start, end)) {
            return *f;
        }
        let f = fit_segment(self.signal, self.times, self.downbeat, start, end, self.c);
        self.cache.insert((start, end), f);
        f
    }
}

pub fn split_segments(signal: &[f64], times: &[f64], downbeat: &[f64], initial: &SegmentFit, c: &FitterConfig) -> Vec<SegmentFit> {
    let mut remaining = c.max_segments - 1.0;
    let mut fits_cache = Fits { signal, times, downbeat, c, cache: HashMap::from([((initial.start_frame, initial.end_frame), *initial)]) };
    let mut fits = vec![*initial];
    let candidates = detect_changes(signal, times, downbeat, initial, c);
    if candidates.len() >= 2 && !((candidates.len() as f64) < c.initial_batch_split_min_candidate_count && initial.score > c.initial_batch_split_max_parent_score) {
        let take = f64::min(remaining, c.initial_batch_split_candidate_count) as usize;
        let mut selected: Vec<usize> = candidates.iter().take(take).map(|x| x.frame).collect();
        selected.sort();
        let mut edges = vec![initial.start_frame];
        edges.extend(selected);
        edges.push(initial.end_frame);
        let batch: Vec<SegmentFit> = (0..edges.len() - 1).map(|i| fits_cache.fit(edges[i], edges[i + 1])).collect();
        let alias = batch.len() as f64 >= c.merge_alias_min_segments && centered(downbeat).1 > 0.0;
        let loose = batch.len() as f64 >= c.merge_many_similar_min_segments;
        let mut merged: Vec<SegmentFit> = Vec::new();
        for f in &batch {
            match merged.last().copied() {
                Some(prev) if fits_mergeable(&prev, f, times, c, alias && f.score <= c.merge_alias_max_fit_score, loose) => {
                    *merged.last_mut().unwrap() = fits_cache.fit(prev.start_frame, f.end_frame);
                }
                _ => merged.push(*f),
            }
        }
        if merged.len() >= 2 && weighted_score(&merged) - initial.score >= c.split_score_improvement_threshold {
            fits = merged;
        }
    }
    remaining -= (fits.len() - 1) as f64;
    let mut splits: HashMap<(usize, usize), Option<EvaluatedSplit>> = HashMap::new();
    while remaining > 0.0 {
        let mut best: Option<EvaluatedSplit> = None;
        for i in 0..fits.len() {
            let f = fits[i];
            let key = (f.start_frame, f.end_frame);
            if let Entry::Vacant(entry) = splits.entry(key) {
                let mut local: Option<EvaluatedSplit> = None;
                for candidate in detect_changes(signal, times, downbeat, &f, c).into_iter().take(c.max_split_candidates_per_segment as usize) {
                    let left = fits_cache.fit(f.start_frame, candidate.frame);
                    let right = fits_cache.fit(candidate.frame, f.end_frame);
                    if fits_mergeable(&left, &right, times, c, false, false) {
                        continue;
                    }
                    let value = EvaluatedSplit { segment_index: i, improvement: weighted_score(&[left, right]) - f.score, candidate, left_fit: left, right_fit: right };
                    if local.as_ref().is_none_or(|l| split_better(&value, l)) {
                        local = Some(value);
                    }
                }
                entry.insert(local);
            }
            let Some(cached) = splits.get(&key).and_then(Option::as_ref) else { continue };
            let value = EvaluatedSplit { segment_index: i, candidate: SplitCandidate { frame: cached.candidate.frame, score: cached.candidate.score }, left_fit: cached.left_fit, right_fit: cached.right_fit, improvement: cached.improvement };
            if best.as_ref().is_none_or(|b| split_better(&value, b)) {
                best = Some(value);
            }
        }
        match best {
            Some(b) if b.improvement >= c.split_score_improvement_threshold => {
                fits.splice(b.segment_index..=b.segment_index, [b.left_fit, b.right_fit]);
                remaining -= 1.0;
            }
            _ => break,
        }
    }
    fits
}
