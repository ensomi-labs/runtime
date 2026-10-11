//! system/arrange.py: legality, playability, restoring force, control tilts, releases and commit.
use crate::controls::{action_tables, chance, chord_feature, fallback_action, get, log_normalize, logit, number, row_range, scope_statistics, subtype_info, text, tilt, transition, Playability, Scope};
use crate::generate::{Trace, TraceFn};
use crate::grid::Grid;
use crate::jsmath;
use crate::machine::Io;
use crate::math::{clip, f32r, max, mean};
use crate::models::{Masks, Models};
use crate::r2::{build_candidates, Candidates, ChartState, Placed, R2Stepper};
use crate::random::RandomSource;
use crate::strain::{reference_workload, StrainState};
use serde_json::{json, Map, Value};
use std::rc::Rc;

fn log_softmax32(logits: &[f32], support: &[bool]) -> Vec<f32> {
    let mut peak = f64::NEG_INFINITY;
    for i in 0..logits.len() {
        if support[i] {
            peak = max(peak, logits[i] as f64);
        }
    }
    let mut total = 0.0;
    for i in 0..logits.len() {
        if support[i] {
            total += f32r(jsmath::exp(f32r(logits[i] as f64 - peak)));
        }
    }
    let z = f32r(jsmath::log(f32r(total)));
    logits.iter().enumerate().map(|(i, &x)| if support[i] { f32r(f32r(x as f64 - peak) - z) as f32 } else { f32::NEG_INFINITY }).collect()
}

fn truthy(x: f64) -> bool {
    x != 0.0 && !x.is_nan()
}

struct ScopeState {
    request: Scope,
    lo: usize,
    hi: usize,
    n_rows: f64,
    n: f64,
    readout: f64,
    expected: f64,
    expected_chance: f64,
    chord_n: f64,
    chord_n_seen: f64,
    chord_success: f64,
    chord_probability: f64,
    gains: Vec<f64>,
    chord_gains: Vec<f64>,
    attainable: Vec<f64>,
    class_error: f64,
    chord_class_error: f64,
}

pub struct Arrange<'a> {
    pub io: &'a Io,
    pub models: &'a Models,
    pub head_ms: &'a [f64],
    pub song_ms: f64,
    pub grid: Rc<Grid>,
    pub random: &'a mut dyn RandomSource,
    pub target: [f64; 3],
    pub ln_fraction: f64,
    pub requests: &'a [Scope],
    pub masks: Masks,
    pub prefix: String,
    pub trace: &'a mut Option<TraceFn>,
}

pub struct Arrangement {
    pub chart: ChartState,
    pub r: f64,
    pub fallback_count: usize,
    pub fallbacks: Vec<Value>,
    pub mask_firings: [i32; 3],
    pub requests: Vec<Value>,
}

fn share_of_extremes(gains: &[f64]) -> Value {
    if gains.is_empty() { Value::Null } else { number(mean(&gains.iter().map(|x| (x.abs() >= 8.0) as i32 as f64).collect::<Vec<_>>())) }
}

pub async fn arrange(a: Arrange<'_>) -> Result<Arrangement, String> {
    let Arrange { io, models, head_ms, song_ms, grid, random, target, ln_fraction, requests, masks, prefix, trace } = a;
    let k_rows = head_ms.len();
    let actions = &models.actions;
    let tables = action_tables(actions);
    let mut chart = ChartState::new(head_ms, song_ms, grid.clone());
    let mut stepper = R2Stepper::new(io.clone(), models)?;
    let mut guard = Playability::new(masks.clone());
    let kappa = [3.0, 3.0, 3.0];
    let mut stream_rows = vec![false; k_rows];
    let mut scopes: Vec<ScopeState> = requests
        .iter()
        .map(|request| {
            let (lo, hi) = row_range(head_ms, get(request, "a_ms"), get(request, "b_ms"), song_ms);
            if text(request, "concept") == "stream" || request.contains_key("chord_target") {
                for row in &mut stream_rows[lo..hi] {
                    *row = true;
                }
            }
            ScopeState {
                request: request.clone(),
                lo,
                hi,
                n_rows: f64::max(0.0, hi as f64 - f64::max(1.0, lo as f64)),
                n: 0.0,
                readout: 0.0,
                expected: 0.0,
                expected_chance: 0.0,
                chord_n: hi as f64 - lo as f64,
                chord_n_seen: 0.0,
                chord_success: 0.0,
                chord_probability: 0.0,
                gains: Vec::new(),
                chord_gains: Vec::new(),
                attainable: Vec::new(),
                class_error: 0.0,
                chord_class_error: 0.0,
            }
        })
        .collect();
    let mut fallbacks: Vec<Value> = Vec::new();
    let mut fallback_rows: Vec<usize> = Vec::new();
    let mut mask_firings = [0i32; 3];
    let mut strain = StrainState::default();
    let mut workload = 0.0;
    let rejection_names = ["reattack", "quad", "jack"];
    for k in 0..=k_rows {
        let held = chart.held[k];
        let pattern = (0..4).fold(0usize, |s, l| s + held[l] as usize * (1 << l));
        let table = &tables[pattern];
        let (heads, ln, features) = (&table.heads, &table.ln, &table.features);
        let support = if k == k_rows { &table.final_row } else { &table.support };
        let output = stepper.predict(&chart, k).await?;
        let plain = log_softmax32(&output.action_logits, support);
        let mut logp: Vec<f64> = plain.iter().map(|&x| x as f64).collect();
        let mut fallback: Option<usize> = None;
        if k < k_rows {
            if ln_fraction == 0.0 {
                for i in 0..logp.len() {
                    if ln[i].iter().any(|&x| x != 0) {
                        logp[i] = f64::NEG_INFINITY;
                    }
                }
            }
            let legal = logp.clone();
            let rejected = guard.rejections(&chart, k, &table.masks);
            for (j, bad) in rejected.iter().enumerate() {
                if bad.iter().enumerate().any(|(i, &v)| v && logp[i].is_finite()) {
                    mask_firings[j] += 1;
                }
            }
            for i in 0..logp.len() {
                if rejected[0][i] || rejected[1][i] || rejected[2][i] {
                    logp[i] = f64::NEG_INFINITY;
                }
            }
            if !logp.iter().any(|x| x.is_finite()) {
                let elapsed: [f64; 4] = std::array::from_fn(|l| head_ms[k] - chart.last_attack[k][l]);
                let chosen = fallback_action(&legal, heads, &elapsed)?;
                let lane = heads[chosen].iter().position(|&x| x != 0).unwrap();
                let mut record = Map::new();
                record.insert("row".into(), json!(k));
                record.insert("time_ms".into(), number(head_ms[k]));
                record.insert("lane".into(), json!(lane));
                record.insert("elapsed_ms".into(), number(elapsed[lane]));
                record.insert("floor_ms".into(), number(masks.reattack_floor_ms));
                record.insert("action_index".into(), json!(chosen));
                record.insert("action".into(), json!(actions[chosen]));
                record.insert("held_on_entry".into(), json!(held.iter().map(|&h| h != 0).collect::<Vec<_>>()));
                record.insert("selected_lane_held".into(), json!(held[lane] != 0));
                record.insert("violated_masks".into(), json!((0..3).filter(|&j| rejected[j][chosen]).map(|j| rejection_names[j]).collect::<Vec<_>>()));
                fallbacks.push(Value::Object(record));
                fallback_rows.push(k);
                logp[chosen] = 0.0;
                fallback = Some(chosen);
            }
            logp = log_normalize(&logp);
        }
        if k < k_rows && k > 0 && kappa.iter().any(|&x| x != 0.0) {
            let start = k.saturating_sub(64);
            let n = (k - start) as f64;
            let mut running = [
                (chart.cum_held[k] - chart.cum_held[start]) as f64 / (4.0 * n),
                (chart.cum_heads[k] - chart.cum_heads[start]) as f64 / n,
                (chart.busy[k] - chart.busy[start]) as f64 / n,
            ];
            let mut gains = kappa;
            let include: Vec<usize> = (start..k).filter(|&j| !stream_rows[j]).collect();
            if stream_rows[k] || include.is_empty() {
                gains[1] = 0.0;
            } else if include.len() != k - start {
                running[1] = mean(&include.iter().map(|&j| chart.attack[j].iter().sum::<i32>() as f64).collect::<Vec<_>>());
            }
            let force: Vec<f64> = (0..3).map(|j| gains[j] * (running[j] - target[j])).collect();
            logp = (0..logp.len()).map(|i| logp[i] - (0.0 + features[i][0] * force[0] + features[i][1] * force[1] + features[i][2] * force[2])).collect();
        }
        let mut candidates: Option<Candidates> = None;
        let mut active: Option<usize> = None;
        let mut concept_feature: Vec<bool> = Vec::new();
        let mut allowed: Vec<bool> = Vec::new();
        if k < k_rows {
            for (si, scope) in scopes.iter_mut().enumerate() {
                if k < scope.lo || k >= scope.hi {
                    continue;
                }
                let r = scope.request.clone();
                let concept = text(&r, "concept").to_string();
                let a_ms = get(&r, "a_ms");
                let visible = !held.iter().any(|&h| h != 0) || {
                    let c = candidates.get_or_insert_with(|| build_candidates(head_ms[k - 1], chart.time(k), &grid));
                    c.times[0] >= a_ms
                };
                let mut base = log_normalize(&logp);
                if r.contains_key("chord_target") {
                    let subtype = text(&r, "subtype").to_string();
                    let cf: Vec<bool> = features.iter().map(|f| chord_feature(f[1] as i32, &subtype)).collect();
                    let mut mass = 0.0;
                    for (i, x) in base.iter().enumerate() {
                        mass += if cf[i] { jsmath::exp(*x) } else { 0.0 };
                    }
                    scope.chord_probability += mass;
                    if visible {
                        let needed = (scope.chord_n * get(&r, "chord_target") - scope.chord_success) / (scope.chord_n - scope.chord_n_seen);
                        let weight = clip(logit(needed) - logit(scope.chord_probability / (scope.chord_n_seen + 1.0)), -8.0, 8.0);
                        let (changed, error) = tilt(&base, &cf, &table.groups["chord"], weight);
                        scope.chord_gains.push(weight);
                        scope.chord_class_error = max(scope.chord_class_error, error);
                        if truthy(weight) {
                            base = changed;
                            logp = base.clone();
                        }
                    }
                }
                if k > 0 {
                    let previous = chart.attack[k - 1];
                    let before = k.checked_sub(2).map(|i| chart.attack[i]);
                    let previous_count = previous.iter().sum::<i32>();
                    let eligible: Vec<[i32; 4]> = heads.iter().map(|h| std::array::from_fn(|l| (h[l] != 0 && held[l] == 0) as i32)).collect();
                    let is_jack = concept == "jack";
                    concept_feature = eligible
                        .iter()
                        .map(|h| {
                            let (j, s) = transition(h, &previous, before.as_ref());
                            (if is_jack { j } else { s }) && h.iter().any(|&x| x != 0)
                        })
                        .collect();
                    allowed = base.iter().map(|x| x.is_finite()).collect();
                    let chances: Vec<f64> = features.iter().map(|f| chance(previous_count, f[1] as i32)).collect();
                    let readout: Vec<f64> = heads
                        .iter()
                        .enumerate()
                        .map(|(i, h)| {
                            let (j, s) = transition(h, &previous, before.as_ref());
                            if is_jack { j as i32 as f64 - chances[i] } else { s as i32 as f64 }
                        })
                        .collect();
                    let (mut expected, mut expected_chance) = (0.0, 0.0);
                    for i in 0..base.len() {
                        let mass = jsmath::exp(base[i]);
                        expected += mass * readout[i];
                        expected_chance += mass * chances[i];
                    }
                    if visible && !r.get("chord_only").is_some_and(|v| v.as_bool() == Some(true)) {
                        let mut weight = 0.0;
                        if r.contains_key("w") {
                            weight = get(&r, "w");
                        } else if truthy(scope.n) {
                            let offset = if is_jack { scope.expected_chance / scope.n } else { 0.0 };
                            let needed = (scope.n_rows * get(&r, "target") - scope.readout) / (scope.n_rows - scope.n);
                            weight = clip(logit(needed + offset) - logit(scope.expected / scope.n + offset), -8.0, 8.0);
                        }
                        let group = if concept == "stream" && r.contains_key("subtype") { "stream_subtype" } else { concept.as_str() };
                        let (changed, error) = tilt(&base, &concept_feature, &table.groups[group], weight);
                        scope.gains.push(weight);
                        scope.class_error = max(scope.class_error, error);
                        if truthy(weight) {
                            logp = changed;
                        }
                    }
                    scope.expected += expected;
                    scope.expected_chance += expected_chance;
                }
                active = Some(si);
                break;
            }
        }
        let chosen = match fallback {
            Some(c) => c,
            None => random.gumbel(&logp, &format!("{prefix}/{k}/action"))?,
        };
        let codes = actions[chosen];
        let orientation = random.orientation(&format!("{prefix}/{k}/orientation"))?;
        let mut lanes: Vec<usize> = (0..4).filter(|&l| held[l] != 0 && codes[l] >= 2).collect();
        let mut releases = [f64::NAN; 4];
        if orientation != 0 {
            lanes.reverse();
        }
        if !lanes.is_empty() {
            let mut placed: Placed = [None; 4];
            for l in 0..4 {
                if held[l] != 0 && codes[l] == 1 {
                    placed[l] = Some((chart.time(k), true));
                }
            }
            let cands = candidates.get_or_insert_with(|| build_candidates(head_ms[k - 1], chart.time(k), &grid));
            for lane in lanes {
                let scores = stepper.release(&output.hands, &chart, k, codes, lane, orientation, &placed, cands).await?;
                let scores64: Vec<f64> = scores.iter().map(|&x| x as f64).collect();
                let index = random.gumbel(&scores64, &format!("{prefix}/{k}/release_{lane}"))?;
                let u = cands.times[index];
                releases[lane] = u;
                placed[lane] = Some((u, false));
            }
        }
        if let Some(t) = trace.as_mut() {
            t(Trace::Chosen { name: format!("{prefix}/{k}/chosen"), codes, releases })?;
        }
        chart.append(codes, releases);
        if k < k_rows {
            let actual = chart.attack[k];
            guard.observe(&actual);
            let (next, charge) = strain.append(head_ms[k], &actual.map(|x| x as f64));
            strain = next;
            workload += charge;
            if let Some(si) = active {
                let scope = &mut scopes[si];
                let r = &scope.request;
                if k > 0 {
                    let previous = chart.attack[k - 1];
                    let (jack, stream) = transition(&actual, &previous, k.checked_sub(2).map(|i| chart.attack[i]).as_ref());
                    let ch = chance(previous.iter().sum::<i32>(), actual.iter().sum::<i32>());
                    scope.readout += if text(r, "concept") == "jack" { jack as i32 as f64 - ch } else { stream as i32 as f64 };
                    scope.n += 1.0;
                    let count = actual.iter().sum::<i32>();
                    let reachable = allowed.iter().enumerate().any(|(i, &v)| v && concept_feature[i] && (text(r, "concept") != "stream" || features[i][1] == count as f64));
                    scope.attainable.push(reachable as i32 as f64);
                }
                if r.contains_key("chord_target") {
                    scope.chord_success += chord_feature(actual.iter().sum::<i32>(), text(r, "subtype")) as i32 as f64;
                    scope.chord_n_seen += 1.0;
                }
            }
            stepper.commit(&chart, k).await?;
        }
    }
    let requests = scopes
        .iter()
        .map(|s| {
            let r = &s.request;
            let stats = scope_statistics(&chart, get(r, "a_ms"), get(r, "b_ms"));
            let achieved = stats[if text(r, "concept") == "stream" { "S" } else { "J_ex" }].as_f64();
            let target_value = get(r, "target");
            let band: Option<Vec<f64>> = r.get("band").and_then(Value::as_array).map(|b| b.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect());
            let preset_in_band = r.get("preset_in_band").and_then(Value::as_bool).unwrap_or(true);
            let in_band = band.as_ref().map(|b| b[0] <= target_value && target_value <= b[1] && preset_in_band);
            let mut rec = r.clone();
            for (key, value) in &stats {
                rec.insert(key.clone(), value.clone());
            }
            rec.insert("achieved".into(), achieved.map_or(Value::Null, number));
            rec.insert("loop".into(), json!("new"));
            rec.insert("in_band".into(), in_band.map_or(Value::Null, |b| json!(b)));
            rec.insert("target_error".into(), achieved.map_or(Value::Null, |a| number(a - target_value)));
            rec.insert("attainable_maximum".into(), if s.attainable.is_empty() { Value::Null } else { number(mean(&s.attainable)) });
            rec.insert("attainable_statistic".into(), json!(if text(r, "concept") == "jack" { "raw_J" } else { "raw_S" }));
            rec.insert("bound_share".into(), share_of_extremes(&s.gains));
            rec.insert("visible_decisions".into(), json!(s.gains.len()));
            rec.insert("max_class_marginal_error".into(), number(s.class_error));
            rec.insert("fallback_count".into(), json!(fallback_rows.iter().filter(|&&row| row >= s.lo && row < s.hi).count()));
            rec.insert("stream_statistic".into(), if text(r, "concept") == "stream" { json!("S_prime") } else { Value::Null });
            rec.insert("band_status".into(), json!(match in_band { Some(true) => "in band", Some(false) => "out of band, playability not promised", None => "not resolved" }));
            if r.contains_key("chord_target") {
                let chord = stats[subtype_info(text(r, "subtype")).1].as_f64();
                let chord_target = get(r, "chord_target");
                let chord_band: Vec<f64> = r["chord_band"].as_array().map(|b| b.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect()).unwrap_or_default();
                rec.insert("chord_achieved".into(), chord.map_or(Value::Null, number));
                rec.insert("chord_error".into(), chord.map_or(Value::Null, |c| number(c - chord_target)));
                rec.insert("chord_in_band".into(), json!(chord_band[0] <= chord_target && chord_target <= chord_band[1] && preset_in_band));
                rec.insert("chord_bound_share".into(), share_of_extremes(&s.chord_gains));
                rec.insert("chord_visible_decisions".into(), json!(s.chord_gains.len()));
                rec.insert("in_scope_decisions".into(), json!(s.hi - s.lo));
                rec.insert("max_chord_class_marginal_error".into(), number(s.chord_class_error));
            }
            Value::Object(rec)
        })
        .collect();
    let r = (workload / reference_workload(head_ms)).sqrt();
    Ok(Arrangement { chart, r, fallback_count: fallbacks.len(), fallbacks, mask_firings, requests })
}
