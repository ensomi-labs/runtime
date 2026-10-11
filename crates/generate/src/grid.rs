use crate::fitter::Segment;
use crate::jsmath;
use crate::math::{argmax, round, pow2};

pub fn canonical_length(length: f64) -> f64 {
    let bpm = 60000.0 / length;
    let fold = jsmath::log2(80.0 / bpm).ceil();
    length / pow2(fold)
}

/// audio_rows/grid.corrected_grid: chart clock, longest-tempo collapse and 80/160 boundary.
pub fn corrected_grid(segments: &[Segment], song_ms: f64) -> Vec<Segment> {
    let mut sorted: Vec<Segment> = segments.to_vec();
    sorted.sort_by(|a, b| crate::fitter::order(a.offset_ms - b.offset_ms));
    for s in sorted.iter_mut() {
        s.offset_ms -= 30.0;
    }
    let ends: Vec<f64> = (0..sorted.len()).map(|i| sorted.get(i + 1).map_or(f64::max(song_ms, sorted[i].offset_ms + 1.0), |n| n.offset_ms)).collect();
    let longest = argmax(&sorted.iter().enumerate().map(|(i, s)| ends[i] - s.offset_ms).collect::<Vec<_>>());
    let canonical: Vec<f64> = sorted.iter().map(|s| canonical_length(s.beat_length_ms)).collect();
    let aligned: Vec<f64> = canonical.iter().map(|&x| x * pow2(round(jsmath::log2(canonical[longest] / x)))).collect();
    if aligned.iter().all(|x| (x / aligned[longest] - 1.0).abs() < 0.003) {
        let Segment { offset_ms: mut offset, beat_length_ms: mut length } = sorted[longest];
        let bpm = 60000.0 / length;
        if (bpm - round(bpm)).abs() <= 0.25 {
            let next = 60000.0 / round(bpm);
            offset += round((ends[longest] - offset) / (2.0 * length)) * (length - next);
            length = next;
        }
        sorted = vec![Segment { offset_ms: offset, beat_length_ms: length }];
    }
    sorted
        .iter()
        .map(|s| {
            let mut length = canonical_length(s.beat_length_ms);
            let bpm = 60000.0 / length;
            if (bpm / 80.0 - 1.0).abs() < 0.003 || (bpm / 160.0 - 1.0).abs() < 0.003 {
                length = 750.0;
            }
            Segment { offset_ms: s.offset_ms, beat_length_ms: length }
        })
        .collect()
}

pub struct Grid {
    pub offsets: Vec<f64>,
    pub lengths: Vec<f64>,
    pub origins: Vec<f64>,
    pub meters: Vec<f64>,
}

impl Grid {
    pub fn new(segments: &[Segment]) -> Self {
        let offsets: Vec<f64> = segments.iter().map(|s| s.offset_ms).collect();
        let lengths: Vec<f64> = segments.iter().map(|s| canonical_length(s.beat_length_ms)).collect();
        let meters = vec![4.0; segments.len()];
        let mut origins = vec![0.0];
        for i in 1..segments.len() {
            origins.push(origins[i - 1] + (offsets[i] - offsets[i - 1]) / lengths[i - 1]);
        }
        Self { offsets, lengths, origins, meters }
    }

    fn index(values: &[f64], x: f64) -> usize {
        let (mut l, mut r) = (0, values.len());
        while l < r {
            let m = (l + r) >> 1;
            if values[m] <= x {
                l = m + 1;
            } else {
                r = m;
            }
        }
        l.saturating_sub(1)
    }

    pub fn segment(&self, time: f64) -> usize {
        Self::index(&self.offsets, time)
    }

    pub fn beat(&self, time: f64) -> f64 {
        let s = self.segment(time);
        self.origins[s] + (time - self.offsets[s]) / self.lengths[s]
    }

    pub fn time(&self, beat: f64) -> f64 {
        let s = Self::index(&self.origins, beat);
        self.offsets[s] + (beat - self.origins[s]) * self.lengths[s]
    }

    pub fn bpm(&self, time: f64) -> f64 {
        60000.0 / self.lengths[self.segment(time)]
    }

    pub fn meter(&self, time: f64) -> f64 {
        self.meters[self.segment(time)]
    }

    pub fn local_beat(&self, time: f64) -> f64 {
        let s = self.segment(time);
        (time - self.offsets[s]) / self.lengths[s]
    }
}
