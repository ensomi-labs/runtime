//! Replays the four recorded release songs through the native pipeline and compares every stage
//! with the Python release: the TypeScript runtime's `checks/parity.ts`, in Rust.
//!
//! The runtime arranges each song once. Every arrangement decision must be the one the release recorded in its first
//! pass; where the release arranged the song a second time, its chart is that pass's and is not compared.
//!
//! With `--stream`, the generation commits a page per head row, from a `Prepared` made by its own run of the audio
//! stages, and the pages must make the chart.
//!
//! Usage: parity --model-dir <ensomi-r2> --reference <runtime-ref> --assets <resampler tables> --out <report.json>
//!        [--stream] [song...]
use ensomi_generate::{chart_tokens, generate, generate_prepared, prepare, Audio, ChartObject, Commit, GenerationResult, Generator, Grid, Models, Options, Page, ReplayRandom, Request, Samples, Step, Trace, TraceFn};
use ensomi_generate_ort::recording::Recording;
use ensomi_generate_ort::{drive, phase_name, Config, Sessions, Timings};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// The tolerance the TypeScript runtime was checked with; BeatThis is allowed `BEATTHIS_TOLERANCE`, see the report.
const GRAPH_TOLERANCE: f64 = 1e-4;
const BEATTHIS_TOLERANCE: f64 = 2e-4;
const GRID_TOLERANCE_MS: f64 = 1e-4;
const TIMING_TOLERANCE_MS: f64 = 1e-6;
const SONGS: [&str; 4] = ["hesitation-snow", "saigetsu", "catch-the-moment", "adventure"];

struct Diff {
    max_abs: f64,
    rms: f64,
    index: i64,
    nonfinite_equal: bool,
    reference: Option<f64>,
    actual: Option<f64>,
    same_length: bool,
}

fn difference(actual: &[f64], expected: &[f64]) -> Diff {
    let (mut max_abs, mut index, mut nonfinite_equal, mut sse) = (0.0f64, -1i64, true, 0.0);
    for (i, &b) in expected.iter().enumerate() {
        let a = actual.get(i).copied().unwrap_or(f64::NAN);
        if !a.is_finite() || !b.is_finite() {
            if !(a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())) {
                nonfinite_equal = false;
            }
            continue;
        }
        let delta = (a - b).abs();
        sse += delta * delta;
        if delta > max_abs {
            max_abs = delta;
            index = i as i64;
        }
    }
    Diff {
        max_abs,
        rms: (sse / expected.len() as f64).sqrt(),
        index,
        nonfinite_equal,
        reference: (index >= 0).then(|| expected[index as usize]),
        actual: (index >= 0).then(|| actual[index as usize]),
        same_length: actual.len() == expected.len(),
    }
}

impl Diff {
    fn json(&self) -> Value {
        json!({"max_abs": self.max_abs, "rms": self.rms, "index": self.index, "nonfinite_equal": self.nonfinite_equal, "reference": self.reference, "actual": self.actual, "same_length": self.same_length})
    }
}

fn f64s_of(t: &ensomi_generate::Tensor) -> Vec<f64> {
    match &t.data {
        ensomi_generate::Data::F32(v) => v.iter().map(|&x| x as f64).collect(),
        ensomi_generate::Data::I64(v) => v.iter().map(|&x| x as f64).collect(),
        ensomi_generate::Data::Bool(v) => v.iter().map(|&x| x as f64).collect(),
    }
}

/// Every recorded graph call on its recorded inputs, against its recorded outputs.
fn network_replay(recording: &Recording, sessions: &mut Sessions) -> Result<Value, String> {
    let mut report: Map<String, Value> = Map::new();
    let mut entries: HashMap<String, (u32, f64, f64, Map<String, Value>)> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for call in recording.graph_calls() {
        let name = call["name"].as_str().unwrap().to_string();
        let inputs = call["inputs"].as_object().unwrap().iter().map(|(k, id)| (k.clone(), recording.array(id.as_u64().unwrap() as usize).tensor())).collect();
        let start = Instant::now();
        let outputs = sessions.run(&name, inputs)?;
        let seconds = start.elapsed().as_secs_f64();
        let entry = entries.entry(name.clone()).or_insert_with(|| {
            order.push(name.clone());
            (0, 0.0, 0.0, Map::new())
        });
        entry.0 += 1;
        entry.1 += seconds;
        for (key, value) in &outputs {
            let expected = recording.array(call["outputs"][key].as_u64().unwrap() as usize).f64s();
            let diff = difference(&f64s_of(value), &expected);
            let better = entry.3.get(key).is_none_or(|old| diff.max_abs > old["max_abs"].as_f64().unwrap());
            if better {
                let mut record = diff.json();
                record["scope"] = call["scope"].clone();
                record["step"] = call["step"].clone();
                entry.3.insert(key.clone(), record);
            }
            entry.2 = entry.2.max(diff.max_abs);
            let tolerance = if name == "beatthis" { BEATTHIS_TOLERANCE } else { GRAPH_TOLERANCE };
            if diff.max_abs > tolerance || !diff.nonfinite_equal || !diff.same_length {
                return Err(format!("stage 1: {name} {key} scope {} step {}: max_abs {} nonfinite_equal {} same_length {}", call["scope"], call["step"], diff.max_abs, diff.nonfinite_equal, diff.same_length));
            }
        }
    }
    for name in order {
        let (calls, seconds, max_abs, outputs) = entries.remove(&name).unwrap();
        report.insert(name, json!({"calls": calls, "seconds": seconds, "max_abs": max_abs, "outputs": outputs}));
    }
    Ok(Value::Object(report))
}

fn first_text_difference(actual: &str, expected: &str) -> Option<Value> {
    let (a, b): (Vec<&str>, Vec<&str>) = (actual.split('\n').collect(), expected.split('\n').collect());
    (0..a.len().max(b.len())).find(|&i| a.get(i) != b.get(i)).map(|i| json!({"line": i + 1, "actual": a.get(i), "reference": b.get(i)}))
}

fn parse_osu(text: &str) -> Vec<ChartObject> {
    text.split("[HitObjects]\n")
        .nth(1)
        .unwrap()
        .trim()
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|line| {
            let p: Vec<&str> = line.split(',').collect();
            let (lane, start) = ((p[0].parse::<f64>().unwrap() - 64.0) / 128.0, p[2].parse::<f64>().unwrap());
            let hold = p[3].parse::<i64>().unwrap() & 128 != 0;
            ChartObject { lane: lane as usize, start_ms: start, end_ms: hold.then(|| p[5].split(':').next().unwrap().parse::<f64>().unwrap()) }
        })
        .collect()
}

fn timing_range(lines: &[&str]) -> (usize, usize) {
    let start = lines.iter().position(|l| *l == "[TimingPoints]").map_or(0, |i| i + 1);
    let mut end = start;
    while start > 0 && lines.get(end).is_some_and(|l| l.trim_start_matches('-').starts_with(|c: char| c.is_ascii_digit())) {
        end += 1;
    }
    (start, end)
}

fn without_timing_points(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let (start, end) = timing_range(&lines);
    [&lines[..start], &lines[end..]].concat().join("\n")
}

fn timing_rows(text: &str) -> Vec<Vec<f64>> {
    let lines: Vec<&str> = text.split('\n').collect();
    let (start, end) = timing_range(&lines);
    lines[start..end].iter().map(|l| l.split(',').map(|x| x.parse::<f64>().unwrap_or(f64::NAN)).collect()).collect()
}

fn strip_generator(text: &str) -> String {
    text.split('\n').filter(|l| !(l.starts_with("// rows=") || l.starts_with("// @ensomi/runtime ") || l.starts_with("// ensomi-runtime "))).collect::<Vec<_>>().join("\n")
}

fn read_files(paths: &[String], base: &Path) -> Result<HashMap<String, Vec<u8>>, String> {
    paths.iter().map(|p| Ok((p.clone(), std::fs::read(base.join(p)).map_err(|e| format!("{p}: {e}"))?))).collect()
}

struct Context {
    models: Arc<Models>,
    sessions: Sessions,
    /// The chart is committed per head row, from a `Prepared`.
    stream: bool,
}

/// Runs a generator that commits per head row to the end, keeping its pages.
fn drive_pages(generator: &mut Generator, sessions: &mut Sessions) -> Result<(GenerationResult, Vec<Page>), String> {
    let (mut outputs, mut pages) = (None, Vec::new());
    loop {
        match generator.step(outputs.take())? {
            Step::Run { graph, inputs } => outputs = Some(sessions.run(&graph, inputs)?),
            Step::Progress(_) => {}
            Step::Commit(page) => pages.push(page),
            Step::Done(result) => return Ok((*result, pages)),
        }
    }
}

/// Whether the pages make the chart: one per head row and the final row, their coverage rising, the last one
/// ending the stream, their tokens those of the whole chart.
fn pages_make_chart(pages: &[Page], result: &GenerationResult, heads: usize) -> Value {
    let tokens: Vec<_> = pages.iter().flat_map(|p| p.tokens.iter().copied()).collect();
    let rising = pages.windows(2).all(|w| w[0].coverage_ms < w[1].coverage_ms);
    let ends = pages.last().is_some_and(|p| p.end_of_stream) && pages.iter().filter(|p| p.end_of_stream).count() == 1;
    let passed = pages.len() == heads + 1 && rising && ends && tokens == chart_tokens(&result.chart);
    json!({"passed": passed, "pages": pages.len(), "tokens": tokens.len(), "coverage_rising": rising, "one_end_of_stream": ends})
}

fn clock() -> Rc<dyn Fn() -> f64> {
    let start = Instant::now();
    Rc::new(move || start.elapsed().as_secs_f64() * 1000.0)
}

fn song_inputs(model_dir: &Path, reference: &Path, assets: &Path, recording: &Recording, song: &str) -> Result<(Audio, Request, String, String, String), String> {
    let audio_event = recording.event("audio").ok_or("no audio event")?;
    let receipt: Value = serde_json::from_slice(&std::fs::read(model_dir.join("examples").join(song).join("receipt.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let expected_osu = std::fs::read_to_string(model_dir.join("examples").join(song).join("generated.osu")).map_err(|e| e.to_string())?;
    let summary: Value = serde_json::from_slice(&std::fs::read(reference.join(song).join("summary.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let audio_name = summary["audio"].as_str().ok_or("no audio in summary")?.rsplit('/').next().unwrap().to_string();
    let channels = *audio_event["pcm"].shape.get(1).unwrap_or(&1);
    let sample_rate = audio_event["sample_rate"].f64s()[0];
    let resampler = ensomi_generate::resampler_files(sample_rate).map(|names| names.map(|n| std::fs::read(assets.join(n)).unwrap()));
    let audio = Audio { samples: Samples::F64(audio_event["pcm"].f64s()), channels, sample_rate, mel: Some(audio_event["mel_pcm"].f32s()), beatthis: None, resampler };
    let request_json = json!({"star": receipt["star"], "title": receipt["metadata"]["title"], "artist": receipt["metadata"]["artist"], "audio": {"asset_id": song}});
    let request = Request::from_json(&request_json.to_string(), receipt["seed"].as_u64().ok_or("seed")?, false)?;
    Ok((audio, request, receipt["setting"].as_str().unwrap_or("r2").to_string(), expected_osu, audio_name))
}

fn run_song(ctx: &mut Context, model_dir: &Path, reference_dir: &Path, assets: &Path, song: &str, report: &mut Map<String, Value>) -> Result<(), String> {
    let recording = Recording::open(&reference_dir.join("bulk").join(song))?;
    let listener = recording.event("listener").ok_or("no listener event")?;
    let audio_event = recording.event("audio").ok_or("no audio event")?;
    let grid_event = recording.event("rows/grid").ok_or("no rows/grid event")?;
    let expected_rows = recording.event("system/rows").ok_or("no system/rows event")?;
    let expected_prior = recording.event("system/prior").ok_or("no system/prior event")?;
    let positions = ctx.models.positions.clone();
    let row: Rc<RefCell<Map<String, Value>>> = Rc::new(RefCell::new(Map::new()));
    let decisions = Rc::new(RefCell::new(0usize));
    let recording = Rc::new(recording);
    let (audio, request, setting, expected_osu, audio_name) = song_inputs(model_dir, reference_dir, assets, &recording, song)?;
    println!("{song}: stages 2-5, replaying draws");
    let expected_listener = (audio_event["spectrogram"].f64s(), listener["log_mel"].f64s(), listener["beat_logits"].f64s());
    let grid_positions = grid_event["position_ms"].f64s();
    let head_ms_expected = expected_rows["head_ms"].f64s();
    let heads = head_ms_expected.len();
    let prior_expected: Vec<(String, f64)> = ["held", "heads", "busy_free", "quad", "log_r"].iter().map(|k| (k.to_string(), expected_prior[*k].f64s()[0])).collect();
    let replay = ReplayRandom::new(recording.draws());
    let recorded_draws = replay.recorded();
    let consumed = replay.position();
    let trace_row = row.clone();
    let trace_decisions = decisions.clone();
    let trace_recording = recording.clone();
    let trace: TraceFn = Box::new(move |event| {
        match event {
            Trace::Listener { spectrogram, log_mel, beat_logits, segments, grid, song_ms, .. } => {
                let grid: &Grid = &grid.whole_beats();
                let b0 = grid.beat(0.0).floor();
                let nb = f64::max((grid.beat(song_ms)).ceil() - b0, 1.0) as usize;
                let (mut max_grid_ms, mut whole_ms, mut frames) = (0.0f64, 0usize, 0usize);
                for b in 0..nb {
                    for p in 0..24 {
                        let actual = grid.time(b0 + b as f64 + positions[p] / 48.0);
                        let expected = grid_positions[b * 24 + p];
                        max_grid_ms = max_grid_ms.max((actual - expected).abs());
                        whole_ms += (ensomi_round(actual) != ensomi_round(expected)) as usize;
                        frames += ((actual / 10.0).trunc() != (expected / 10.0).trunc()) as usize;
                    }
                }
                let passed = max_grid_ms <= GRID_TOLERANCE_MS && whole_ms == 0 && frames == 0;
                let as_f64 = |v: &[f32]| v.iter().map(|&x| x as f64).collect::<Vec<_>>();
                let stage2 = json!({
                    "passed": passed, "max_grid_error_ms": max_grid_ms, "whole_ms_mismatches": whole_ms, "frame_mismatches": frames,
                    "spectrogram": difference(&as_f64(spectrogram), &expected_listener.0).json(),
                    "log_mel": difference(&as_f64(log_mel), &expected_listener.1).json(),
                    "beat_logits": difference(&as_f64(beat_logits), &expected_listener.2).json(),
                    "segments": segments.iter().map(|s| json!([s.offset_ms, s.beat_length_ms])).collect::<Vec<_>>(),
                });
                trace_row.borrow_mut().insert("stage2".into(), stage2.clone());
                if !passed {
                    return Err(format!("stage 2 failed: {stage2}"));
                }
            }
            Trace::Rows { head_ms } => {
                let diff = difference(head_ms, &head_ms_expected);
                let passed = diff.max_abs == 0.0 && diff.same_length;
                trace_row.borrow_mut().insert("stage3".into(), json!({"passed": passed, "heads": head_ms.len(), "head_times": diff.json()}));
                if !passed {
                    return Err(format!("stage 3 failed: {}", diff.json()));
                }
            }
            Trace::Prior { prediction } => {
                let values = [prediction.held, prediction.heads, prediction.busy_free, prediction.quad, prediction.log_r];
                let error = values.iter().zip(&prior_expected).fold(0.0f64, |e, (v, (_, x))| e.max((v - x).abs()));
                trace_row.borrow_mut().insert("stage4".into(), json!({"passed": false, "prior_max_error": error}));
                if error != 0.0 {
                    return Err(format!("stage 4 prior differs by {error}"));
                }
            }
            Trace::Chosen { name, codes, releases } => {
                let expected = trace_recording.event(&name).ok_or_else(|| format!("no recorded event {name}"))?;
                *trace_decisions.borrow_mut() += 1;
                let actual_codes: Vec<f64> = codes.iter().map(|&c| c as f64).collect();
                for (key, actual) in [("codes", actual_codes), ("releases", releases.to_vec())] {
                    let diff = difference(&actual, &expected[key].f64s());
                    if diff.max_abs != 0.0 || !diff.nonfinite_equal {
                        return Err(format!("stage 4 {name} {key}: {}", diff.json()));
                    }
                }
            }
        }
        Ok(())
    });
    // With `--stream`, the audio stages and the rest are two generators, which share the trace.
    let trace = Rc::new(RefCell::new(trace));
    let traced = || -> Option<TraceFn> {
        let trace = trace.clone();
        Some(Box::new(move |event: Trace<'_>| (trace.borrow_mut())(event)))
    };
    let options = Options { random: Some(Box::new(replay)), setting: Some(setting.clone()), audio_name: Some(audio_name.clone()), trace: traced(), clock: clock(), commit: Commit::AtEnd };
    let mut audio_stages = None;
    let result = if ctx.stream {
        let (prepared, _) = drive(&mut prepare(ctx.models.clone(), audio, Options { trace: traced(), ..Options::new(clock()) }), &mut ctx.sessions)?;
        let prepared = Arc::new(prepared);
        audio_stages = Some(prepared.clone());
        let (result, pages) = drive_pages(&mut generate_prepared(ctx.models.clone(), prepared, request, Options { commit: Commit::HeadRow, ..options }), &mut ctx.sessions)?;
        row.borrow_mut().insert("pages".into(), pages_make_chart(&pages, &result, heads));
        result
    } else {
        drive(&mut generate(ctx.models.clone(), audio, request, options), &mut ctx.sessions)?.0
    };
    // Where the release ran pass 2, its draws are left.
    let pass_2 = recording.index["draws"].as_array().ok_or("no draws")?.iter().filter(|d| d["name"].as_str().is_some_and(|n| n.starts_with("r2_pass_2/"))).count();
    let expected_draws = recorded_draws - pass_2;
    if consumed.get() != expected_draws {
        return Err(format!("Unconsumed draws: {}/{expected_draws}", consumed.get()));
    }
    let mut row = row.borrow().clone();
    let detail: Value = serde_json::from_str(&result.receipt.receipt_json).map_err(|e| e.to_string())?;
    let mut stage4 = row.get("stage4").cloned().unwrap_or_else(|| json!({}));
    stage4["passed"] = json!(true);
    stage4["decisions"] = json!(*decisions.borrow());
    stage4["draws_consumed"] = json!(consumed.get());
    stage4["draws_recorded"] = json!(recorded_draws);
    stage4["passes"] = detail["controls"]["difficulty"]["passes"].clone();
    stage4["release_passes"] = json!(if pass_2 > 0 { 2 } else { 1 });
    if detail["controls"]["difficulty"]["passes"].as_array().map(Vec::len) != Some(1) {
        return Err(format!("stage 4 {song}: passes {}", detail["controls"]["difficulty"]["passes"]));
    }
    row.insert("stage4".into(), stage4);
    row.insert("replay_seconds".into(), detail["seconds"].clone());
    let expected_objects = parse_osu(&expected_osu);
    if pass_2 > 0 {
        // The release's chart is pass 2's; pass 1's decisions were checked one by one above.
        row.insert("stage5".into(), json!({"passed": true, "compared": false, "reason": "the release ran pass 2", "objects": result.chart.objects.len(), "same_as_release": result.chart.objects == expected_objects}));
    } else if result.chart.objects != expected_objects {
        let index = result.chart.objects.iter().zip(&expected_objects).position(|(a, b)| a != b).unwrap_or(result.chart.objects.len().min(expected_objects.len()));
        return Err(format!("stage 5 {song}: hit objects differ at {index}: {:?} vs {:?}", result.chart.objects.get(index), expected_objects.get(index)));
    } else {
        let text_difference = first_text_difference(&strip_generator(&result.osu), &strip_generator(&expected_osu));
        let (rows_a, rows_b) = (timing_rows(&result.osu), timing_rows(&expected_osu));
        let same_rows = rows_a.len() == rows_b.len() && rows_a.iter().zip(&rows_b).all(|(a, b)| a.len() == b.len());
        let mut timing_max = 0.0f64;
        for (a, b) in rows_a.iter().zip(&rows_b) {
            for (x, y) in a.iter().zip(b) {
                timing_max = timing_max.max((x - y).abs());
            }
        }
        let rest = first_text_difference(&strip_generator(&without_timing_points(&result.osu)), &strip_generator(&without_timing_points(&expected_osu)));
        row.insert(
            "stage5".into(),
            json!({
                "passed": same_rows && timing_max <= TIMING_TOLERANCE_MS && rest.is_none(),
                "hitobjects_identical": true, "objects": result.chart.objects.len(), "timing_points": rows_b.len(), "timing_max_abs_ms": timing_max,
                "rest_identical": rest.is_none(), "osu_identical_except_generator": text_difference.is_none(), "first_text_difference": text_difference,
            }),
        );
    }
    println!("{song} replay {}", row["stage5"]);
    // The generator's own PRNG, as it runs in production; with `--stream`, from the same audio stages.
    println!("{song}: own PRNG run");
    let (audio, request, _, _, _) = song_inputs(model_dir, reference_dir, assets, &recording, song)?;
    let options = Options { setting: Some(setting), audio_name: Some(audio_name), ..Options::new(clock()) };
    let started = Instant::now();
    let (own, own_timings, own_pages) = match audio_stages {
        Some(prepared) => {
            let (own, pages) = drive_pages(&mut generate_prepared(ctx.models.clone(), prepared, request.clone_for_check(), Options { commit: Commit::HeadRow, ..options }), &mut ctx.sessions)?;
            let heads = serde_json::from_str::<Value>(&own.receipt.receipt_json).map_err(|e| e.to_string())?["rows"]["K"].as_u64().unwrap_or(0) as usize;
            let check = pages_make_chart(&pages, &own, heads);
            (own, None, Some(check))
        }
        None => {
            let (own, timings) = drive(&mut generate(ctx.models.clone(), audio, request.clone_for_check(), options), &mut ctx.sessions)?;
            (own, Some(timings), None)
        }
    };
    let wall = started.elapsed().as_secs_f64();
    let own_detail: Value = serde_json::from_str(&own.receipt.receipt_json).map_err(|e| e.to_string())?;
    let finalized = own_detail["export"]["replay_finalized"].as_bool().unwrap_or(false);
    if !finalized {
        return Err(format!("Own PRNG left an open hold: {song}"));
    }
    let pages_passed = own_pages.as_ref().is_none_or(|p| p["passed"] == true);
    row.insert(
        "prng".into(),
        json!({
            "passed": pages_passed, "algorithm": own_detail["runtime"]["random_algorithm"], "objects": own.chart.objects.len(), "heads": own_detail["rows"]["K"],
            "fallbacks": own_detail["fallback_count"], "seconds": own_detail["seconds"], "wall_seconds": wall, "timings": own_timings.as_ref().map(timings_json),
            "pages": own_pages,
        }),
    );
    report.insert(song.to_string(), Value::Object(row));
    Ok(())
}

fn ensomi_round(x: f64) -> f64 {
    let lo = x.floor();
    let d = x - lo;
    if d == 0.5 { lo + if lo % 2.0 == 0.0 { 0.0 } else { 1.0 } } else if d > 0.5 { lo + 1.0 } else { lo }
}

fn timings_json(t: &Timings) -> Value {
    let phases: Map<String, Value> = t.phases.iter().enumerate().filter(|(_, p)| p.0 + p.1 > 0.0).map(|(i, p)| (phase_name(i).to_string(), json!({"core_seconds": p.0, "graph_seconds": p.1}))).collect();
    let graphs: Map<String, Value> = t.graphs.iter().map(|(g, (calls, seconds))| (g.clone(), json!({"calls": calls, "seconds": seconds}))).collect();
    json!({"by_phase": phases, "by_graph": graphs})
}

trait CloneForCheck {
    fn clone_for_check(&self) -> Self;
}

impl CloneForCheck for Request {
    fn clone_for_check(&self) -> Self {
        Request { audio: self.audio.clone(), star: self.star, seed: self.seed, seed_is_bigint: self.seed_is_bigint, controls: Default::default(), title: self.title.clone(), artist: self.artist.clone() }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut model_dir, mut reference, mut assets, mut out) = (None, None, None, None);
    let mut stream = false;
    let mut songs: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model-dir" => model_dir = args.next().map(PathBuf::from),
            "--reference" => reference = args.next().map(PathBuf::from),
            "--assets" => assets = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--stream" => stream = true,
            _ => songs.push(arg),
        }
    }
    let (model_dir, reference, assets, out) = (model_dir.expect("--model-dir"), reference.expect("--reference"), assets.expect("--assets"), out.expect("--out"));
    if songs.is_empty() {
        songs = SONGS.iter().map(|s| s.to_string()).collect();
    }
    let mut report = Map::new();
    report.insert("ort".into(), json!("onnxruntime 1.23.2 (ort 2.0.0-rc.11)"));
    report.insert("provider".into(), json!("cpu"));
    report.insert("threads".into(), json!(1));
    report.insert("graph_tolerance".into(), json!({"default": GRAPH_TOLERANCE, "beatthis": BEATTHIS_TOLERANCE}));
    report.insert("grid_tolerance_ms".into(), json!(GRID_TOLERANCE_MS));
    report.insert("timing_tolerance_ms".into(), json!(TIMING_TOLERANCE_MS));
    report.insert("scope".into(), json!("recorded graph inputs, PCM frontends, full Rust decode with carried state and replayed random draws, exact exports, own PRNG"));
    report.insert("commit".into(), json!(if stream { "a page per head row from a Prepared; graphs not replayed again" } else { "the whole chart at the end" }));
    let save = |report: &Map<String, Value>| {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&out, serde_json::to_string_pretty(report).unwrap() + "\n").unwrap();
    };
    let outcome = (|| -> Result<(), String> {
        let onnx = model_dir.join("onnx");
        let manifest: Value = serde_json::from_slice(&std::fs::read(onnx.join("runtime.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let files = read_files(&Models::required_files(&manifest), &onnx)?;
        let models = Arc::new(Models::new(manifest, files)?);
        let sessions = Sessions::load(&onnx, &models, &Config::default())?;
        let mut ctx = Context { models, sessions, stream };
        let mut songs_report = Map::new();
        for song in &songs {
            if stream {
                songs_report.insert(song.clone(), json!({}));
                continue;
            }
            println!("{song}: stage 1 graph replay");
            let recording = Recording::open(&reference.join("bulk").join(song))?;
            let networks = network_replay(&recording, &mut ctx.sessions)?;
            println!("{song} stage 1 {}", networks.as_object().unwrap().iter().map(|(k, v)| format!("{k} {:.3e}", v["max_abs"].as_f64().unwrap())).collect::<Vec<_>>().join(", "));
            let mut row = Map::new();
            row.insert("stage1".into(), json!({"passed": true, "networks": networks}));
            songs_report.insert(song.clone(), Value::Object(row));
            report.insert("songs".into(), Value::Object(songs_report.clone()));
            save(&report);
        }
        for song in &songs {
            let mut row_report = Map::new();
            let result = run_song(&mut ctx, &model_dir, &reference, &assets, song, &mut row_report);
            let mut row = songs_report[song].as_object().cloned().unwrap();
            if let Some(Value::Object(extra)) = row_report.remove(song) {
                row.extend(extra);
            }
            songs_report.insert(song.clone(), Value::Object(row));
            report.insert("songs".into(), Value::Object(songs_report.clone()));
            save(&report);
            result?;
        }
        let first = if stream { 2 } else { 1 };
        let passed = songs.iter().all(|s| (first..=5).all(|n| songs_report[s][format!("stage{n}")]["passed"] == true) && songs_report[s]["prng"]["passed"] == true && (!stream || songs_report[s]["pages"]["passed"] == true));
        report.insert("passed".into(), json!(passed));
        if !passed {
            return Err("some stage did not pass".to_string());
        }
        Ok(())
    })();
    if let Err(e) = &outcome {
        report.insert("passed".into(), json!(false));
        report.insert("failure".into(), json!(e));
        eprintln!("{e}");
    }
    save(&report);
    if outcome.is_err() {
        std::process::exit(1);
    }
}
