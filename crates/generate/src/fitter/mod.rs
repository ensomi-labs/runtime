//! The beat grid fitter: audio_rows/fitter, from BeatThis logits and the Log-Mel to timing segments.
mod alias;
mod change_detection;
mod mel_phase;
mod peaks;
mod refinement;
mod scoring;
mod segment_fit;
mod segments;
mod splitting;

use crate::models::FitterConfig;

#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub offset_ms: f64,
    pub beat_length_ms: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct SegmentFit {
    pub start_frame: usize,
    pub end_frame: usize,
    pub score: f64,
    pub beat_length_ms: f64,
    pub offset_ms: f64,
    pub raw_bpm: f64,
    pub tempo_multiplier: f64,
}

impl SegmentFit {
    pub fn frame_count(&self) -> f64 {
        (self.end_frame - self.start_frame) as f64
    }
}

pub struct GridCandidate {
    pub score: f64,
    pub bpm: f64,
    pub beat_length_ms: f64,
    pub offset_ms: f64,
}

pub use scoring::{order, unique};

pub fn bpm(beat_length_ms: f64) -> f64 {
    60000.0 / beat_length_ms
}

fn sigmoid32(x: f32) -> f64 {
    crate::math::f32r(1.0 / (1.0 + libm::exp(-(x as f64))))
}

fn fit_segments(beat: &[f32], downbeat: &[f32], config: &FitterConfig) -> Vec<Segment> {
    let signal: Vec<f64> = beat.iter().map(|&x| sigmoid32(x)).collect();
    let down: Vec<f64> = downbeat.iter().map(|&x| sigmoid32(x)).collect();
    let times: Vec<f64> = (0..signal.len()).map(|i| i as f64 / 50.0 * 1000.0).collect();
    let mut c = config.clone();
    if signal.len() as f64 / 50.0 >= c.long_prediction_duration_seconds {
        c.max_segments = f64::max(c.max_segments, c.long_max_segments);
        c.max_grid_candidates_per_segment = f64::max(c.max_grid_candidates_per_segment, c.long_max_grid_candidates_per_segment);
        c.max_split_candidates_per_segment = f64::max(c.max_split_candidates_per_segment, c.long_max_split_candidates_per_segment);
        c.downbeat_refine_candidate_count = f64::max(c.downbeat_refine_candidate_count, c.long_downbeat_refine_candidate_count);
    }
    let initial = segment_fit::fit_segment(&signal, &times, &down, 0, signal.len(), &c);
    let splits = splitting::split_segments(&signal, &times, &down, &initial, &c);
    let aliased = alias::canonicalize_aliases(&splits, &signal, &times, &down, &c);
    let raw = segments::timing_segments(&aliased, &times, &c);
    refinement::refine_timing(&raw, &times, &signal, &c)
}

/// The fitted timing segments for a song, from the beat and downbeat logits and the Log-Mel.
pub fn fit_grid(beat: &[f32], downbeat: &[f32], mel: &[f32], song_ms: f64, config: &FitterConfig) -> Vec<Segment> {
    let mut segments = fit_segments(beat, downbeat, config);
    segments = peaks::refine_peaks(&segments, beat, 50.0);
    segments = peaks::downbeat_phase(&segments, beat, downbeat, 50.0);
    segments = mel_phase::mel_phase(&segments, mel, beat, song_ms);
    mel_phase::mel_tempo(&segments, mel, beat, song_ms)
}
