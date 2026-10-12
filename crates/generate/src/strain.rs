//! r2/strain.py: the frozen ras-v1 workload. Strain continues across section boundaries.

pub const REFERENCE_LANES: [usize; 4] = [0, 2, 1, 3];

fn tau_i() -> f64 {
    -1.0 / libm::log(0.125)
}

fn tau_g() -> f64 {
    -1.0 / libm::log(0.30)
}

#[derive(Clone, Default)]
pub struct StrainState {
    lane: [f64; 4],
    overall: f64,
    time_ms: f64,
}

impl StrainState {
    fn decayed(&self, time: f64) -> StrainState {
        let dt = (time - self.time_ms) / 1000.0;
        StrainState { lane: self.lane.map(|x| x * libm::exp(-dt / tau_i())), overall: self.overall * libm::exp(-dt / tau_g()), time_ms: time }
    }

    fn cost(&self, heads: &[f64; 4]) -> f64 {
        let (mut m, mut dot) = (0.0, 0.0);
        for l in 0..4 {
            m += heads[l];
            dot += heads[l] * self.lane[l];
        }
        tau_i() * (dot + m / 2.0) + 0.25 * tau_g() * (self.overall * m + m * m / 2.0)
    }

    /// Adds one row of heads at `time` (ms); returns the new state and the row's charge.
    pub fn append(&self, time: f64, heads: &[f64; 4]) -> (StrainState, f64) {
        let s = self.decayed(time);
        let mut lane = s.lane;
        for l in 0..4 {
            lane[l] += heads[l];
        }
        let next = StrainState { lane, overall: s.overall + heads.iter().fold(0.0, |a, b| a + b), time_ms: time };
        let charge = s.cost(heads);
        (next, charge)
    }
}

pub fn reference_workload(heads: &[f64]) -> f64 {
    let mut state = StrainState::default();
    let mut total = 0.0;
    for (i, &t) in heads.iter().enumerate() {
        let mut q = [0.0; 4];
        q[REFERENCE_LANES[i % 4]] = 1.0;
        let (next, charge) = state.append(t, &q);
        state = next;
        total += charge;
    }
    total
}
