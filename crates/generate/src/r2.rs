//! r2/features.py and sampling.IncrementalState: explicit, committed-prefix state, and the R2 graph calls.
use crate::grid::Grid;
use crate::jsmath;
use crate::machine::{Io, Outputs, Tensor};
use crate::math::{clip, lower_bound, round, sign};
use crate::models::Models;
use std::collections::BTreeSet;
use std::f64::consts::PI;

type Lanes = [f64; 4];

fn psi(d: f64, scale: f64) -> [f64; 2] {
    [clip(d / scale, -32.0, 32.0), sign(d) * jsmath::log1p(d.abs() / scale)]
}

fn psi_pair(ms: f64, beats: f64) -> [f64; 4] {
    let (a, b) = (psi(ms, 1000.0), psi(beats, 1.0));
    [a[0], a[1], b[0], b[1]]
}

fn phase(beat: f64) -> [f64; 6] {
    let mut out = [0.0; 6];
    for (i, p) in [1.0, 4.0, 16.0].into_iter().enumerate() {
        let angle = 2.0 * PI * beat / p;
        out[2 * i] = jsmath::sin(angle);
        out[2 * i + 1] = jsmath::cos(angle);
    }
    out
}

fn onehot(value: f64, size: usize) -> impl Iterator<Item = f64> {
    let at = clip(value, 0.0, size as f64 - 1.0);
    (0..size).map(move |i| (i as f64 == at) as i32 as f64)
}

fn right_bound(a: &[f64], x: f64) -> usize {
    let mut i = lower_bound(a, x);
    while i < a.len() && a[i] == x {
        i += 1;
    }
    i
}

fn nan4() -> Lanes {
    [f64::NAN; 4]
}

pub struct ChartState {
    pub k: usize,
    pub head_ms: Vec<f64>,
    pub song_ms: f64,
    pub head_beats: Vec<f64>,
    pub song_beat: f64,
    pub grid: std::rc::Rc<Grid>,
    pub actions: Vec<[i32; 4]>,
    pub gap: Vec<Lanes>,
    pub held: Vec<[i32; 4]>,
    pub start: Vec<Lanes>,
    pub birth: Vec<[i32; 4]>,
    pub release: Vec<Lanes>,
    pub attack: Vec<[i32; 4]>,
    pub ln_head: Vec<[i32; 4]>,
    pub last_attack: Vec<Lanes>,
    pub last_release: Vec<Lanes>,
    pub cum_heads: Vec<i32>,
    pub cum_held: Vec<i32>,
    pub busy: Vec<i32>,
}

impl ChartState {
    pub fn new(head_ms: &[f64], song_ms: f64, grid: std::rc::Rc<Grid>) -> Self {
        Self {
            k: head_ms.len(),
            head_ms: head_ms.to_vec(),
            song_ms,
            head_beats: head_ms.iter().map(|&t| grid.beat(t)).collect(),
            song_beat: grid.beat(song_ms),
            grid,
            actions: Vec::new(),
            gap: Vec::new(),
            held: vec![[0; 4]],
            start: vec![nan4()],
            birth: vec![[-1; 4]],
            release: Vec::new(),
            attack: Vec::new(),
            ln_head: Vec::new(),
            last_attack: vec![nan4()],
            last_release: vec![nan4()],
            cum_heads: vec![0],
            cum_held: vec![0],
            busy: vec![0],
        }
    }

    pub fn time(&self, k: usize) -> f64 {
        if k < self.k { self.head_ms[k] } else { self.song_ms }
    }

    pub fn beat(&self, k: usize) -> f64 {
        if k < self.k { self.head_beats[k] } else { self.song_beat }
    }

    /// Commits row `actions.len()`: the action codes and the release times of its gap releases.
    pub fn append(&mut self, codes: [i32; 4], gap: Lanes) {
        let k = self.actions.len();
        let t = self.time(k);
        let (held, starts, births) = (self.held[k], self.start[k], self.birth[k]);
        let attack: [i32; 4] = std::array::from_fn(|l| if held[l] != 0 { (codes[l] >= 3) as i32 } else { (codes[l] == 1 || codes[l] == 2) as i32 });
        let ln: [i32; 4] = std::array::from_fn(|l| if held[l] != 0 { (codes[l] == 4) as i32 } else { (codes[l] == 2) as i32 });
        let closed: [bool; 4] = std::array::from_fn(|l| held[l] != 0 && codes[l] != 0);
        let release: Lanes = std::array::from_fn(|l| if closed[l] { if codes[l] == 1 { t } else { gap[l] } } else { f64::NAN });
        self.actions.push(codes);
        self.gap.push(gap);
        self.attack.push(attack);
        self.ln_head.push(ln);
        self.release.push(release);
        self.held.push(std::array::from_fn(|l| ((held[l] != 0 && !closed[l]) || ln[l] != 0) as i32));
        self.start.push(std::array::from_fn(|l| if ln[l] != 0 { t } else if closed[l] { f64::NAN } else { starts[l] }));
        self.birth.push(std::array::from_fn(|l| if ln[l] != 0 { k as i32 } else if closed[l] { -1 } else { births[l] }));
        let last_attack = self.last_attack[k];
        self.last_attack.push(std::array::from_fn(|l| if attack[l] != 0 { t } else { last_attack[l] }));
        let last_release = self.last_release[k];
        self.last_release.push(std::array::from_fn(|l| {
            let (v, r) = (last_release[l], release[l]);
            if r.is_nan() { v } else if v.is_nan() { r } else { crate::math::max(v, r) }
        }));
        self.cum_heads.push(self.cum_heads[k] + attack.iter().sum::<i32>());
        self.cum_held.push(self.cum_held[k] + held.iter().sum::<i32>());
        self.busy.push(self.busy[k] + (0..4).all(|l| held[l] != 0 || attack[l] != 0) as i32);
    }
}

fn lane_query(chart: &ChartState, k: usize) -> [[f64; 15]; 4] {
    let t = chart.time(k);
    let bt = chart.beat(k);
    let age = |x: f64| -> [f64; 4] { if x.is_nan() { [0.0; 4] } else { psi_pair(t - x, bt - chart.grid.beat(x)) } };
    std::array::from_fn(|l| {
        let mut out = [0.0; 15];
        out[0] = chart.held[k][l] as f64;
        out[1..5].copy_from_slice(&age(chart.start[k][l]));
        out[5..9].copy_from_slice(&age(chart.last_attack[k][l]));
        out[9..13].copy_from_slice(&age(chart.last_release[k][l]));
        out[13] = chart.last_attack[k][l].is_nan() as i32 as f64;
        out[14] = chart.last_release[k][l].is_nan() as i32 as f64;
        out
    })
}

pub fn query_features(chart: &ChartState, k: usize) -> Vec<f32> {
    let lanes = lane_query(chart, k);
    let t = chart.time(k);
    let bt = chart.beat(k);
    let prev = k.saturating_sub(1);
    let kk = chart.k;
    let mut shared: Vec<f64> = Vec::with_capacity(90);
    shared.extend(psi_pair(if k == 0 { 0.0 } else { t - chart.time(prev) }, if k == 0 { 0.0 } else { bt - chart.beat(prev) }));
    shared.extend(psi_pair(chart.song_ms - t, chart.song_beat - bt));
    shared.extend([t / chart.song_ms, k as f64 / f64::max(kk as f64, 1.0), (k == 0) as i32 as f64, (k == kk) as i32 as f64]);
    shared.extend(phase(bt));
    shared.extend([chart.grid.bpm(t) / 120.0, chart.grid.meter(t) / 4.0]);
    let lo = right_bound(&chart.head_beats, bt);
    for h in [1.0, 2.0, 4.0, 8.0, 16.0, 32.0] {
        shared.push(if k == kk { 0.0 } else { jsmath::log1p(right_bound(&chart.head_beats, bt + h) as f64 - lo as f64) });
    }
    for i in 1..=16usize {
        let index = k + i;
        let a = index.min(kk - 1);
        let b = index.saturating_sub(1).min(kk - 1);
        if index <= kk - 1 {
            shared.extend(psi_pair(chart.head_ms[a] - chart.head_ms[b], chart.head_beats[a] - chart.head_beats[b]));
        } else {
            shared.extend([0.0; 4]);
        }
    }
    let mut out = Vec::with_capacity(300);
    for order in [[0usize, 1, 2, 3], [3, 2, 1, 0]] {
        for l in order {
            out.extend(lanes[l].iter().map(|&x| x as f32));
        }
        out.extend(shared.iter().map(|&x| x as f32));
    }
    out
}

pub fn history_token(chart: &ChartState, k: usize) -> Vec<f32> {
    let t = chart.time(k);
    let bt = chart.beat(k);
    let prev = k.saturating_sub(1);
    let lanes: Vec<Vec<f64>> = (0..4)
        .map(|l| {
            let release = chart.release[k][l];
            let start = chart.start[k][l];
            let valid = !release.is_nan() && !start.is_nan();
            let mut out: Vec<f64> = onehot(chart.actions[k][l] as f64, 5).collect();
            out.push(chart.held[k][l] as f64);
            out.push((!release.is_nan()) as i32 as f64);
            if valid {
                out.extend(psi_pair(release - t, chart.grid.beat(release) - bt));
                out.extend(psi_pair(release - chart.time(prev), chart.grid.beat(release) - chart.beat(prev)));
                out.extend(psi_pair(release - start, chart.grid.beat(release) - chart.grid.beat(start)));
            } else {
                out.extend([0.0; 12]);
            }
            out
        })
        .collect();
    let mut shared: Vec<f64> = Vec::new();
    shared.extend(psi_pair(t - chart.time(prev), bt - chart.beat(prev)));
    shared.extend(psi_pair(t - chart.head_ms[0], bt - chart.head_beats[0]));
    shared.extend(phase(bt));
    shared.extend(onehot(chart.attack[k].iter().sum::<i32>() as f64, 5));
    shared.extend(onehot(chart.ln_head[k].iter().sum::<i32>() as f64, 5));
    let mut out = Vec::with_capacity(200);
    for order in [[0usize, 1, 2, 3], [3, 2, 1, 0]] {
        for l in order {
            out.extend(lanes[l].iter().map(|&x| x as f32));
        }
        out.extend(shared.iter().map(|&x| x as f32));
    }
    out
}

/// The release times a gap release can take between two rows, and their grid features.
pub struct Candidates {
    pub times: Vec<f64>,
    pub beats: Vec<f64>,
    pub statics: Vec<Vec<f64>>,
}

pub fn build_candidates(a: f64, b: f64, grid: &Grid) -> Candidates {
    let mut grid_times: BTreeSet<i64> = BTreeSet::new();
    let segments = grid.offsets.len();
    let mut s = grid.segment(a);
    while s <= grid.segment(b) {
        let lo = if s == 0 { f64::NEG_INFINITY } else { grid.offsets[s] };
        let hi = if s == segments - 1 { f64::INFINITY } else { grid.offsets[s + 1] };
        let left = crate::math::max(a - 1.0, lo);
        let right = crate::math::min(b + 1.0, hi);
        if right >= left {
            for denominator in [32.0, 24.0] {
                let step = grid.lengths[s] / denominator;
                let n0 = ((left - grid.offsets[s]) / step).ceil();
                let n1 = ((right - grid.offsets[s]) / step).floor();
                let mut n = n0;
                while n <= n1 {
                    let at = grid.offsets[s] + n * step;
                    n += 1.0;
                    if at < lo || at >= hi {
                        continue;
                    }
                    let u = (at + 0.5).floor();
                    if u > a && u < b {
                        grid_times.insert(u as i64);
                    }
                }
            }
        }
        s += 1;
    }
    let bb = grid.beat(b);
    let ba = grid.beat(a);
    let mid = ((a + b) / 2.0 + 0.5).floor();
    let eighth = (grid.time(bb - 0.125) + 0.5).floor();
    let quarter = (grid.time(bb - 0.25) + 0.5).floor();
    let mut all = grid_times.clone();
    for x in [mid, eighth, quarter] {
        if x > a && x < b {
            all.insert(x as i64);
        }
    }
    let times: Vec<f64> = all.into_iter().map(|t| t as f64).collect();
    let beats: Vec<f64> = times.iter().map(|&t| grid.beat(t)).collect();
    let statics = times
        .iter()
        .enumerate()
        .map(|(i, &u)| {
            let mut snap = [0.0; 10];
            let local = grid.local_beat(u);
            let length = grid.lengths[grid.segment(u)];
            for (j, d) in [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0].into_iter().enumerate() {
                if (local * d - round(local * d)).abs() / d * length <= 1.0 {
                    snap[j] = 1.0;
                    break;
                }
            }
            let mut out = Vec::with_capacity(30);
            out.extend(psi_pair(u - b, beats[i] - bb));
            out.extend(psi_pair(u - a, beats[i] - ba));
            out.push((u - a) / (b - a));
            out.extend([(u == mid) as i32 as f64, (u == eighth) as i32 as f64, (u == quarter) as i32 as f64, grid_times.contains(&(u as i64)) as i32 as f64]);
            out.extend(snap);
            out.push(grid.bpm(u) / 120.0);
            out.extend(phase(beats[i]));
            out
        })
        .collect();
    Candidates { times, beats, statics }
}

/// Lanes already placed on this row: the release time and whether it is on the row.
pub type Placed = [Option<(f64, bool)>; 4];

pub struct ReleaseFactor {
    pub codes: [i32; 4],
    pub slot: i64,
    pub lane_features: Vec<f32>,
    pub features: Vec<f32>,
    pub relations: Vec<f32>,
}

pub fn release_factor(chart: &ChartState, k: usize, codes: [i32; 4], lane: usize, orientation: u32, placed: &Placed, candidates: &Candidates) -> ReleaseFactor {
    let start = chart.start[k][lane];
    let start_beat = chart.grid.beat(start);
    let n = candidates.times.len();
    let mut features = vec![0f32; n * 34];
    let mut relations = vec![0f32; n * 28];
    for i in 0..n {
        let u = candidates.times[i];
        let ub = candidates.beats[i];
        let s = &candidates.statics[i];
        let mut row: Vec<f64> = Vec::with_capacity(34);
        row.extend(&s[..8]);
        row.extend(psi_pair(u - start, ub - start_beat));
        row.extend(&s[8..]);
        for (j, x) in row.iter().enumerate() {
            features[i * 34 + j] = *x as f32;
        }
        for slot in 0..4 {
            let physical = if orientation == 0 { slot } else { 3 - slot };
            let Some((r, on_row)) = placed[physical] else { continue };
            let mut rel: Vec<f64> = Vec::with_capacity(7);
            rel.extend(psi_pair(u - r, ub - chart.grid.beat(r)));
            rel.push((chart.birth[k][physical] == chart.birth[k][lane] && chart.birth[k][physical] >= 0) as i32 as f64);
            rel.push(1.0);
            rel.push(on_row as i32 as f64);
            for (j, x) in rel.iter().enumerate() {
                relations[i * 28 + slot * 7 + j] = *x as f32;
            }
        }
    }
    let oriented = if orientation != 0 { [codes[3], codes[2], codes[1], codes[0]] } else { codes };
    ReleaseFactor { codes: oriented, slot: if orientation != 0 { 3 - lane as i64 } else { lane as i64 }, lane_features: lane_query(chart, k)[lane].iter().map(|&x| x as f32).collect(), features, relations }
}

/// The R2 graphs with their carried state.
pub struct R2Stepper {
    io: Io,
    commit_inputs: Vec<String>,
    commit_outputs: Vec<(String, String)>,
    state: Vec<(String, Tensor)>,
}

pub struct Prediction {
    pub hands: Tensor,
    pub action_logits: Vec<f32>,
}

impl R2Stepper {
    pub fn new(io: Io, models: &Models) -> Result<Self, String> {
        Ok(Self {
            io,
            commit_inputs: models.graph("r2_commit")?.inputs.clone(),
            commit_outputs: models.r2_commit_outputs.clone(),
            state: models.r2_initial.iter().map(|(name, spec)| Ok((name.clone(), models.tensor(spec)?))).collect::<Result<_, String>>()?,
        })
    }

    fn state(&self, name: &str) -> Tensor {
        self.state.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone()).expect("R2 state tensor")
    }

    pub async fn predict(&self, chart: &ChartState, k: usize) -> Result<Prediction, String> {
        let mut out = self
            .io
            .run("r2_predict", vec![("last", self.state("last")), ("query", Tensor::f32(&[1, 2, 150], query_features(chart, k))), ("row_condition", Tensor::f32(&[1, 68], vec![0.0; 68]))])
            .await?;
        Ok(Prediction { hands: out.take("hands")?, action_logits: out.take_f32("action_logits")? })
    }

    pub async fn release(&self, hands: &Tensor, chart: &ChartState, k: usize, codes: [i32; 4], lane: usize, orientation: u32, placed: &Placed, candidates: &Candidates) -> Result<Vec<f32>, String> {
        let factor = release_factor(chart, k, codes, lane, orientation, placed, candidates);
        let n = candidates.times.len();
        let mut out = self
            .io
            .run(
                "r2_release",
                vec![
                    ("hands", hands.clone()),
                    ("orientation", Tensor::i64(&[1], vec![orientation as i64])),
                    ("codes", Tensor::i64(&[1, 4], factor.codes.iter().map(|&c| c as i64).collect())),
                    ("slot", Tensor::i64(&[1], vec![factor.slot])),
                    ("lane_features", Tensor::f32(&[1, 15], factor.lane_features)),
                    ("relations", Tensor::f32(&[n, 28], factor.relations)),
                    ("candidates", Tensor::f32(&[n, 34], factor.features)),
                    ("pair_condition", Tensor::f32(&[n, 68], vec![0.0; n * 68])),
                ],
            )
            .await?;
        out.take_f32("release_scores")
    }

    pub async fn commit(&mut self, chart: &ChartState, k: usize) -> Result<(), String> {
        let mut feeds = vec![("history_token", Tensor::f32(&[2, 100], history_token(chart, k)))];
        let names: Vec<String> = self.commit_inputs.iter().filter(|n| *n != "history_token").cloned().collect();
        let tensors: Vec<Tensor> = names.iter().map(|n| self.state(n)).collect();
        for (name, tensor) in names.iter().zip(tensors) {
            feeds.push((name.as_str(), tensor));
        }
        let mut output: Outputs = self.io.run("r2_commit", feeds).await?;
        let mut next = Vec::new();
        for (out, name) in &self.commit_outputs {
            next.push((name.clone(), output.take(out)?));
        }
        self.state = next;
        Ok(())
    }
}
