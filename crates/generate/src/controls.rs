//! system/controls.py: class-preserving controls and corpus-scope lookup.
use crate::jsmath;
use crate::math::{argmax, clip, lower_bound, max, mean, median, quantile};
use crate::models::{Calibration, Models};
use crate::r2::ChartState;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

pub type Row = [i32; 4];

pub struct ActionTable {
    pub heads: Vec<Row>,
    pub ln: Vec<Row>,
    /// Per action: long heads, heads and whether every lane is covered by a head or a hold.
    pub features: Vec<[f64; 3]>,
    pub groups: HashMap<&'static str, Vec<Vec<usize>>>,
    /// Lanes with a head, one bit per lane.
    pub masks: Vec<u32>,
    /// Actions allowed on a head row and on the final row.
    pub support: Vec<bool>,
    pub final_row: Vec<bool>,
}

pub fn action_tables(actions: &[[i32; 4]]) -> Vec<ActionTable> {
    (0..16)
        .map(|pattern| {
            let held: [bool; 4] = std::array::from_fn(|l| (pattern >> l) & 1 == 1);
            let heads: Vec<Row> = actions.iter().map(|c| std::array::from_fn(|l| if held[l] { (c[l] == 3 || c[l] == 4) as i32 } else { (c[l] == 1 || c[l] == 2) as i32 })).collect();
            let ln: Vec<Row> = actions.iter().map(|c| std::array::from_fn(|l| if held[l] { (c[l] == 4) as i32 } else { (c[l] == 2) as i32 })).collect();
            let features: Vec<[f64; 3]> = (0..actions.len())
                .map(|i| [ln[i].iter().sum::<i32>() as f64, heads[i].iter().sum::<i32>() as f64, (0..4).all(|l| heads[i][l] != 0 || held[l]) as i32 as f64])
                .collect();
            let mut groups: HashMap<&'static str, Vec<Vec<usize>>> = HashMap::new();
            for concept in ["jack", "stream_subtype", "stream", "chord"] {
                let mut keyed: BTreeMap<Vec<i64>, Vec<usize>> = BTreeMap::new();
                for (i, f) in features.iter().enumerate() {
                    let key: Vec<i64> = match concept {
                        "jack" => f.iter().map(|&x| x as i64).collect(),
                        "stream_subtype" => vec![(f[0] > 0.0) as i64, f[1] as i64, f[2] as i64],
                        _ => vec![(f[0] > 0.0) as i64, f[2] as i64],
                    };
                    keyed.entry(key).or_default().push(i);
                }
                groups.insert(concept, keyed.into_values().collect());
            }
            let masks = heads.iter().map(|h| (0..4).fold(0u32, |m, l| m | (h[l] as u32) << l)).collect();
            let support = (0..actions.len()).map(|i| (0..4).all(|l| held[l] || actions[i][l] <= 2) && heads[i].iter().any(|&x| x != 0)).collect();
            let final_row = actions.iter().map(|codes| (0..4).all(|l| codes[l] == if held[l] { 2 } else { 0 })).collect();
            ActionTable { heads, ln, features, groups, masks, support, final_row }
        })
        .collect()
}

pub fn log_normalize(logp: &[f64]) -> Vec<f64> {
    let mut peak = f64::NEG_INFINITY;
    for &x in logp {
        if x > peak {
            peak = x;
        }
    }
    let mut sum = 0.0;
    for &x in logp {
        if x.is_finite() {
            sum += jsmath::exp(x - peak);
        }
    }
    let z = peak + jsmath::log(sum);
    logp.iter().map(|x| x - z).collect()
}

fn log_add_exp(a: f64, b: f64) -> f64 {
    if a == f64::NEG_INFINITY {
        return b;
    }
    if b == f64::NEG_INFINITY {
        return a;
    }
    let m = max(a, b);
    m + jsmath::log1p(jsmath::exp(-(a - b).abs()))
}

/// Tilts the probability inside each group toward the feature, keeping each group's mass.
pub fn tilt(logp: &[f64], feature: &[bool], groups: &[Vec<usize>], weight: f64) -> (Vec<f64>, f64) {
    if weight == 0.0 {
        return (logp.to_vec(), 0.0);
    }
    let mut out = logp.to_vec();
    let mut error = 0.0;
    for group in groups {
        let indices: Vec<usize> = group.iter().copied().filter(|&i| logp[i].is_finite()).collect();
        if indices.is_empty() {
            continue;
        }
        let (mut old_z, mut new_z) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for &i in &indices {
            old_z = log_add_exp(old_z, logp[i]);
            new_z = log_add_exp(new_z, logp[i] + weight * feature[i] as i32 as f64);
        }
        let (mut old_mass, mut new_mass) = (0.0, 0.0);
        for &i in &indices {
            out[i] = logp[i] + weight * feature[i] as i32 as f64 + old_z - new_z;
            old_mass += jsmath::exp(logp[i]);
            new_mass += jsmath::exp(out[i]);
        }
        error = max(error, (new_mass - old_mass).abs());
    }
    (out, error)
}

/// Whether the row repeats a lane of the previous row (a jack) and whether it is a stream step.
pub fn transition(heads: &Row, previous: &Row, before_previous: Option<&Row>) -> (bool, bool) {
    let jack = (0..4).any(|l| heads[l] != 0 && previous[l] != 0);
    let back = before_previous.is_some_and(|b| previous.iter().sum::<i32>() != 3 && !(0..4).any(|l| previous[l] != 0 && b[l] != 0) && (0..4).all(|l| heads[l] == b[l]));
    (jack, !jack && !back)
}

fn choose(n: i32, k: i32) -> f64 {
    if k > n {
        return 0.0;
    }
    let mut x = 1.0;
    for j in 1..=k {
        x = x * (n - j + 1) as f64 / j as f64;
    }
    x
}

/// The chance that a row of `b` heads repeats a lane of a row of `a` heads.
pub fn chance(a: i32, b: i32) -> f64 {
    if a != 0 && b != 0 { 1.0 - choose(4 - a, b) / choose(4, b) } else { 0.0 }
}

/// The concept and the row share a pattern subtype is measured by.
pub fn subtype_info(subtype: &str) -> (&'static str, &'static str) {
    match subtype {
        "jumpstream" => ("stream", "jump_share"),
        "chordstream" => ("stream", "hand_share"),
        _ => ("jack", "chord_share"),
    }
}

pub fn chord_feature(count: i32, subtype: &str) -> bool {
    if subtype == "jumpstream" { count == 2 } else { count >= if subtype == "chordstream" { 3 } else { 2 } }
}

pub fn logit(v: f64) -> f64 {
    let p = clip(v, 0.01, 0.99);
    jsmath::log(p / (1.0 - p))
}

pub fn row_range(heads: &[f64], a: f64, b: f64, song_ms: f64) -> (usize, usize) {
    (lower_bound(heads, a), if b >= song_ms { heads.len() } else { lower_bound(heads, b) })
}

pub struct Playability {
    config: crate::models::Masks,
    runs: [i32; 4],
    quad_run: i32,
}

impl Playability {
    pub fn new(config: crate::models::Masks) -> Self {
        Self { config, runs: [0; 4], quad_run: 0 }
    }

    /// For each action, whether the re-attack floor, the quad run cap and the jack run cap forbid it.
    pub fn rejections(&self, chart: &ChartState, k: usize, masks: &[u32]) -> [Vec<bool>; 3] {
        let gap = if k > 0 { chart.head_ms[k] - chart.head_ms[k - 1] } else { f64::INFINITY };
        let edges = &self.config.gap_tercile_edges_ms;
        let mut tercile = 0;
        while tercile < edges.len() && edges[tercile] <= gap {
            tercile += 1;
        }
        let cap = self.config.jack_run_caps[tercile];
        let quad = self.quad_run as f64 >= self.config.quad_run_cap;
        let (mut reattack, mut jack) = (0u32, 0u32);
        for l in 0..4 {
            if chart.head_ms[k] - chart.last_attack[k][l] < self.config.reattack_floor_ms {
                reattack |= 1 << l;
            }
            if self.runs[l] as f64 >= cap {
                jack |= 1 << l;
            }
        }
        [masks.iter().map(|m| m & reattack != 0).collect(), masks.iter().map(|&m| m == 15 && quad).collect(), masks.iter().map(|m| m & jack != 0).collect()]
    }

    pub fn observe(&mut self, heads: &Row) {
        for l in 0..4 {
            self.runs[l] = if heads[l] != 0 { self.runs[l] + 1 } else { 0 };
        }
        self.quad_run = if heads.iter().all(|&x| x != 0) { self.quad_run + 1 } else { 0 };
    }
}

pub fn fallback_action(logp: &[f64], heads: &[Row], elapsed: &[f64; 4]) -> Result<usize, String> {
    let lane = |i: usize| heads[i].iter().position(|&x| x != 0).unwrap();
    let mut candidates: Vec<usize> = (0..logp.len()).filter(|&i| logp[i].is_finite() && heads[i].iter().sum::<i32>() == 1).collect();
    let oldest = candidates.iter().fold(f64::NEG_INFINITY, |a, &i| max(a, elapsed[lane(i)]));
    candidates.retain(|&i| elapsed[lane(i)] == oldest);
    if candidates.is_empty() {
        return Err("generation_failed: no single-head action to fall back to".to_string());
    }
    Ok(candidates[argmax(&candidates.iter().map(|&i| logp[i]).collect::<Vec<_>>())])
}

/// The scope table's columns by name.
pub type ScopeTable = HashMap<String, Vec<f64>>;

pub fn load_scopes(models: &Models) -> Result<ScopeTable, String> {
    models.calibration.scope_table.columns.iter().map(|c| Ok((c.name.clone(), models.scope_column(c)?))).collect()
}

struct Matched {
    indices: Vec<usize>,
    length: f64,
    scale: Option<Value>,
}

fn matched_scopes(table: &ScopeTable, band: f64, length_ms: f64, gap_ms: f64, subtype: Option<&str>, chord_target: f64) -> Matched {
    let lengths = [8.0, 16.0, 32.0];
    let length = lengths[argmax(&lengths.map(|x: f64| -(jsmath::log(x) - jsmath::log(length_ms / 1000.0)).abs()))];
    let indices: Vec<usize> = (0..table["band"].len()).filter(|&i| table["band"][i] == band && table["length_s"][i] == length).collect();
    let lg: Vec<f64> = indices.iter().map(|&i| jsmath::log(table["median_gap_ms"][i])).collect();
    let (distances, scale): (Vec<f64>, Option<Value>) = match subtype {
        None => (lg.iter().map(|x| (x - jsmath::log(gap_ms)).abs()).collect(), None),
        Some(sub) => {
            let share: Vec<f64> = indices.iter().map(|&i| table[subtype_info(sub).1][i]).collect();
            let means = [mean(&lg), mean(&share)];
            let sd: Vec<f64> = [&lg, &share]
                .iter()
                .enumerate()
                .map(|(j, values)| {
                    let s = mean(&values.iter().map(|x| (x - means[j]) * (x - means[j])).collect::<Vec<_>>()).sqrt();
                    if s == 0.0 || s.is_nan() { 1.0 } else { s }
                })
                .collect();
            let distances = (0..indices.len()).map(|i| jsmath::hypot(&[(lg[i] - jsmath::log(gap_ms)) / sd[0], (share[i] - chord_target) / sd[1]])).collect();
            (distances, Some(json!({"population_scopes": indices.len(), "mean": means, "sd": sd, "ddof": 0, "constant_coordinate_scale": 1})))
        }
    };
    let mut ranked: Vec<(usize, f64)> = indices.iter().copied().zip(distances).collect();
    ranked.sort_by(|a, b| crate::fitter::order(a.1 - b.1));
    Matched { indices: ranked.into_iter().take(256).map(|x| x.0).collect(), length, scale }
}

/// A pattern scope request or its resolution: an ordered record, as the receipt writes it.
pub type Scope = Map<String, Value>;

pub fn get(scope: &Scope, key: &str) -> f64 {
    scope.get(key).and_then(Value::as_f64).unwrap_or(f64::NAN)
}

pub fn text<'a>(scope: &'a Scope, key: &str) -> &'a str {
    scope.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn number(v: f64) -> Value {
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

fn numbers(values: &[f64]) -> Value {
    Value::Array(values.iter().map(|&v| number(v)).collect())
}

pub fn resolve_requests(heads: &[f64], song_ms: f64, requests: &[Scope], band: u32, cal: &Calibration, table: &ScopeTable) -> Result<Vec<Scope>, String> {
    requests
        .iter()
        .map(|request| {
            let (a_ms, b_ms, q) = (get(request, "a_ms"), get(request, "b_ms"), get(request, "q"));
            let (lo, hi) = row_range(heads, a_ms, b_ms, song_ms);
            let gaps: Vec<f64> = if hi > lo + 1 { heads[lo + 1..hi].iter().enumerate().map(|(i, x)| x - heads[lo + i]).collect() } else { Vec::new() };
            if gaps.is_empty() {
                return Err("A pattern scope must contain at least two head rows".to_string());
            }
            let gap = median(&gaps);
            let sub = request.get("subtype").and_then(Value::as_str);
            let first = matched_scopes(table, band as f64, b_ms - a_ms, gap, None, 0.0);
            let floor = cal.masks[&band.to_string()].reattack_floor_ms;
            let column = |name: &str, indices: &[usize]| -> Vec<f64> { indices.iter().map(|&i| table[name][i]).collect() };
            let mut out = request.clone();
            if let Some(sub) = sub {
                let (concept, share_name) = subtype_info(sub);
                let preset = &cal.subtypes[sub].bands[&band.to_string()];
                let shares = column(share_name, &first.indices);
                let chord_target = quantile(&shares, preset.q_sub);
                let matched = matched_scopes(table, band as f64, b_ms - a_ms, gap, Some(sub), chord_target);
                let vs = column(if concept == "stream" { "S" } else { "J_ex" }, &matched.indices);
                let mut reasons: Vec<&str> = Vec::new();
                if (sub == "jumpstream" || sub == "chordstream") && gap > cal.stream_eligibility_gap_ms {
                    reasons.push("rows do not carry a stream: median gap above stream p90");
                }
                if sub == "chordstream" && 2.0 * gap < floor {
                    reasons.push("two median gaps below re-attack floor");
                }
                if sub == "chordjack" && gap < max(floor, preset.reference_gap_p10_ms) {
                    reasons.push("median gap below re-attack floor or chordjack reference p10");
                }
                out.insert("target".into(), number(quantile(&vs, q)));
                out.insert("band".into(), numbers(&[quantile(&vs, 0.05), quantile(&vs, 0.95)]));
                out.insert("p90".into(), number(quantile(&vs, 0.9)));
                out.insert("neighbors".into(), json!(vs.len()));
                out.insert("length_bin_s".into(), number(first.length));
                out.insert("chord_target".into(), number(chord_target));
                out.insert("chord_band".into(), numbers(&[quantile(&shares, 0.05), quantile(&shares, 0.95)]));
                out.insert("chord_q".into(), number(preset.q_sub));
                out.insert("chord_neighbors".into(), json!(shares.len()));
                out.insert("preset_in_band".into(), json!(preset.in_band));
                out.insert("preset_out_of_band_reasons".into(), preset.out_of_band_reasons.clone());
                out.insert("concept_match_standardization".into(), matched.scale.unwrap_or(Value::Null));
                out.insert("temporal_eligible".into(), json!(reasons.is_empty()));
                out.insert("eligibility_reasons".into(), json!(reasons));
                out.insert("reattack_floor_ms".into(), number(floor));
                out.insert("reference_gap_p10_ms".into(), number(preset.reference_gap_p10_ms));
            } else {
                let vs = column(if text(request, "concept") == "stream" { "S" } else { "J_ex" }, &first.indices);
                out.insert("target".into(), number(quantile(&vs, q)));
                out.insert("band".into(), numbers(&[quantile(&vs, 0.05), quantile(&vs, 0.95)]));
                out.insert("p90".into(), number(quantile(&vs, 0.9)));
                out.insert("neighbors".into(), json!(vs.len()));
                out.insert("length_bin_s".into(), number(first.length));
            }
            out.insert("median_gap_ms".into(), number(gap));
            if text(request, "concept") == "stream" {
                out.insert("rows_carry_stream".into(), json!(gap <= cal.stream_eligibility_gap_ms));
            } else {
                out.insert("reattack_floor_ms".into(), number(floor));
                out.insert("rows_allow_jack".into(), json!(gap >= floor));
            }
            Ok(out)
        })
        .collect()
}

/// The jack and stream statistics of the heads between `a` and `b` ms.
pub fn scope_statistics(chart: &ChartState, a: f64, b: f64) -> Scope {
    let (lo, hi) = row_range(&chart.head_ms, a, b, chart.song_ms);
    let (mut jack, mut stream, mut chances) = (Vec::new(), Vec::new(), Vec::new());
    let counts: Vec<f64> = chart.attack[lo..hi].iter().map(|h| h.iter().sum::<i32>() as f64).collect();
    let (mut chord_pairs, mut identical) = (0, 0);
    for k in lo.max(1)..hi {
        let (j, s) = transition(&chart.attack[k], &chart.attack[k - 1], k.checked_sub(2).map(|i| &chart.attack[i]));
        jack.push(j as i32 as f64);
        stream.push(s as i32 as f64);
        let prev = chart.attack[k - 1].iter().sum::<i32>();
        let now = chart.attack[k].iter().sum::<i32>();
        chances.push(chance(prev, now));
        if prev >= 2 && now >= 2 {
            chord_pairs += 1;
            if chart.attack[k] == chart.attack[k - 1] {
                identical += 1;
            }
        }
    }
    let n = jack.len();
    let share = |test: &dyn Fn(f64) -> bool| -> Value { if counts.is_empty() { Value::Null } else { number(mean(&counts.iter().map(|&x| test(x) as i32 as f64).collect::<Vec<_>>())) } };
    let some = |v: f64| if n > 0 { number(v) } else { Value::Null };
    let mut out = Map::new();
    out.insert("transitions".into(), json!(n));
    out.insert("J".into(), some(if n > 0 { mean(&jack) } else { 0.0 }));
    out.insert("J_chance".into(), some(if n > 0 { mean(&chances) } else { 0.0 }));
    out.insert("J_ex".into(), some(if n > 0 { mean(&jack) - mean(&chances) } else { 0.0 }));
    out.insert("S".into(), some(if n > 0 { mean(&stream) } else { 0.0 }));
    out.insert("heads_per_row".into(), if counts.is_empty() { Value::Null } else { number(mean(&counts)) });
    out.insert("single_note_share".into(), share(&|x| x == 1.0));
    out.insert("jump_share".into(), share(&|x| x == 2.0));
    out.insert("hand_share".into(), share(&|x| x >= 3.0));
    out.insert("chord_share".into(), share(&|x| x >= 2.0));
    let mut mix = Map::new();
    for size in 1..=4 {
        mix.insert(size.to_string(), share(&|x| x == size as f64));
    }
    out.insert("arity_mix".into(), Value::Object(mix));
    out.insert("identical_consecutive_chord_share".into(), some(if n > 0 { identical as f64 / n as f64 } else { 0.0 }));
    out.insert("identical_consecutive_chord_count".into(), json!(identical));
    out.insert("consecutive_chord_transitions".into(), json!(chord_pairs));
    out.insert("identical_given_consecutive_chord_share".into(), if chord_pairs > 0 { number(identical as f64 / chord_pairs as f64) } else { Value::Null });
    out
}
