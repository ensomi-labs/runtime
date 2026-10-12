//! Measures ONNX Runtime configurations on the four release songs: per song, the time to open the model,
//! the total and per-stage generation time, physical footprint (after opening the model, at each phase, peak before the
//! first generation, which holds the recording, and peak overall), and whether the chart equals the CPU
//! chart for the same PCM and seed. Each configuration and song runs in its own process, so peak memory is its own;
//! each process generates twice, to show the cost of the first run.
//!
//! Input: the recorded `audio` PCM of each song with its channels averaged in float64 and rounded to
//! float32, as a client passes mono PCM, at the recorded rate, without frontend streams. Request: star,
//! seed, title and artist of the release example's receipt, no controls.
//!
//! Usage: providers --model-dir <ensomi-r2> --reference <runtime-ref> --assets <resampler tables>
//!        --out <report.json> [--charts <dir>] [--configs cpu,coreml,...] [song...]
//! `--charts` receives each song's CPU `.osu` and its request. A run adds its configurations to an existing
//! report and compares with the CPU charts already in `--charts`.
use ensomi_generate::{generate, Audio, Models, Options, Request, Samples};
use ensomi_generate_ort::recording::Recording;
use ensomi_generate_ort::{drive_observed, package, phase_name, Config, Sessions, Timings};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

const SONGS: [&str; 4] = ["hesitation-snow", "saigetsu", "catch-the-moment", "adventure"];
const CONFIGS: [&str; 5] = ["cpu", "cpu-threads", "cpu-lean", "coreml-beatthis", "coreml"];

fn config(name: &str, models: &Models) -> Config {
    let all: Vec<String> = models.graphs.iter().map(|g| g.name.clone()).collect();
    let beatthis = vec!["beatthis".to_string()];
    let coreml = |graphs: Vec<String>, units: &str, format: &str| Config { coreml_graphs: graphs, coreml_units: units.to_string(), coreml_format: format.to_string(), ..Config::default() };
    match name {
        "cpu" => Config::default(),
        "cpu-threads" => Config { threads: 0, ..Config::default() },
        "cpu-lean" => Config { reuse_memory: false, ..Config::default() },
        "coreml" => coreml(all, "ALL", "MLProgram"),
        "coreml-beatthis" => coreml(beatthis, "ALL", "MLProgram"),
        "coreml-beatthis-gpu" => coreml(beatthis, "CPUAndGPU", "MLProgram"),
        "coreml-beatthis-ane" => coreml(beatthis, "CPUAndNeuralEngine", "MLProgram"),
        "coreml-beatthis-cpu" => coreml(beatthis, "CPUOnly", "MLProgram"),
        "coreml-beatthis-nn" => coreml(beatthis, "ALL", "NeuralNetwork"),
        other => panic!("unknown configuration {other}"),
    }
}

struct Paths {
    model_dir: PathBuf,
    reference: PathBuf,
    assets: PathBuf,
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
}

/// The recorded PCM averaged to mono float32, and its sample rate.
fn mono_pcm(recording: &Recording) -> (Vec<f32>, f64) {
    let audio = recording.event("audio").expect("no audio event");
    let channels = *audio["pcm"].shape.get(1).unwrap_or(&1);
    let samples = audio["pcm"].f64s();
    let pcm = samples.chunks_exact(channels).map(|frame| (frame.iter().sum::<f64>() / channels as f64) as f32).collect();
    (pcm, audio["sample_rate"].f64s()[0])
}

/// `{star, title, artist}` and the seed of the release example's receipt.
fn request(paths: &Paths, song: &str) -> (Value, u64) {
    let receipt = read_json(&paths.model_dir.join("examples").join(song).join("receipt.json"));
    (json!({"star": receipt["star"], "title": receipt["metadata"]["title"], "artist": receipt["metadata"]["artist"]}), receipt["seed"].as_u64().unwrap())
}

fn timings_json(t: &Timings) -> Value {
    let phases: Map<String, Value> = t.phases.iter().enumerate().filter(|(_, p)| p.0 + p.1 > 0.0).map(|(i, p)| (phase_name(i).to_string(), json!({"core_seconds": p.0, "graph_seconds": p.1}))).collect();
    let graphs: Map<String, Value> = t.graphs.iter().map(|(g, (calls, seconds))| (g.clone(), json!({"calls": calls, "seconds": seconds}))).collect();
    json!({"by_phase": phases, "by_graph": graphs})
}

/// The process's physical footprint (what iOS limits) now and its lifetime peak, in bytes.
#[cfg(target_vendor = "apple")]
fn footprint() -> (u64, u64) {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    unsafe { libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V4, &mut info as *mut _ as *mut libc::rusage_info_t) };
    (info.ri_phys_footprint, info.ri_lifetime_max_phys_footprint)
}

#[cfg(not(target_vendor = "apple"))]
fn footprint() -> (u64, u64) {
    (0, 0)
}

/// One configuration on one song, in this process.
fn child(paths: &Paths, name: &str, song: &str) -> Value {
    let onnx = paths.model_dir.join("onnx");
    let started = Instant::now();
    let (models, _) = package::open(&onnx).unwrap();
    let models = Arc::new(models);
    let config = config(name, &models);
    let mut sessions = Sessions::load(&onnx, &models, &config).unwrap();
    let load_seconds = started.elapsed().as_secs_f64();
    let after_load = footprint().0;
    let recording = Recording::open(&paths.reference.join("bulk").join(song)).unwrap();
    let (pcm, sample_rate) = mono_pcm(&recording);
    drop(recording);
    let (request_json, seed) = request(paths, song);
    let resampler = ensomi_generate::resampler_files(sample_rate).map(|names| names.map(|n| std::fs::read(paths.assets.join(n)).unwrap()));
    let before_generation = footprint().1;
    let mut runs = Vec::new();
    let mut osu = String::new();
    let mut objects = 0;
    for _ in 0..2 {
        let audio = Audio { samples: Samples::F32(pcm.clone()), channels: 1, sample_rate, mel: None, beatthis: None, resampler: resampler.clone() };
        let request = Request::from_json(&request_json.to_string(), seed, false).unwrap();
        let clock_start = Instant::now();
        let options = Options::new(Rc::new(move || clock_start.elapsed().as_secs_f64() * 1000.0));
        let started = Instant::now();
        let mut generator = generate(models.clone(), audio, request, options);
        // The footprint as each phase starts and at the end, in MiB.
        let mut footprints = Map::new();
        let (result, timings) = drive_observed(&mut generator, &mut sessions, |phase| {
            if let Some(phase) = phase {
                footprints.insert(phase_name(phase as usize).to_string(), json!(footprint().0 >> 20));
            }
            Ok(())
        })
        .unwrap();
        footprints.insert("done".to_string(), json!(footprint().0 >> 20));
        let receipt: Value = serde_json::from_str(&result.receipt.receipt_json).unwrap();
        runs.push(json!({"wall_seconds": started.elapsed().as_secs_f64(), "stages": receipt["seconds"], "timings": timings_json(&timings), "footprint_mib_at_phase": footprints}));
        objects = result.chart.objects.len();
        osu = result.osu;
    }
    json!({
        "config": format!("{config:?}"), "load_seconds": load_seconds, "runs": runs, "objects": objects,
        "footprint_after_load_bytes": after_load, "peak_footprint_before_generation_bytes": before_generation, "peak_footprint_bytes": footprint().1,
        "sample_rate": sample_rate, "request": request_json, "seed": seed, "osu": osu,
    })
}

fn hit_objects(osu: &str) -> &str {
    osu.split("[HitObjects]\n").nth(1).unwrap_or_default()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut model_dir, mut reference, mut assets, mut out, mut charts, mut child_of) = (None, None, None, None, None, None);
    let mut configs: Vec<String> = CONFIGS.iter().map(|c| c.to_string()).collect();
    let mut songs: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model-dir" => model_dir = args.next().map(PathBuf::from),
            "--reference" => reference = args.next().map(PathBuf::from),
            "--assets" => assets = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--charts" => charts = args.next().map(PathBuf::from),
            "--configs" => configs = args.next().unwrap().split(',').map(str::to_string).collect(),
            "--child" => child_of = args.next(),
            _ => songs.push(arg),
        }
    }
    let paths = Paths { model_dir: model_dir.expect("--model-dir"), reference: reference.expect("--reference"), assets: assets.expect("--assets") };
    if songs.is_empty() {
        songs = SONGS.iter().map(|s| s.to_string()).collect();
    }
    if let Some(name) = child_of {
        println!("{}", child(&paths, &name, &songs[0]));
        return;
    }
    let out = out.expect("--out");
    // Runs add to an existing report, and compare with the CPU charts already in `--charts`.
    let mut report: Map<String, Value> = std::fs::read(&out).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    report.insert("ort".into(), json!("onnxruntime 1.23.2 (ort 2.0.0-rc.11)"));
    report.insert("input".into(), json!("recorded audio PCM, channels averaged in float64 and rounded to float32, at the recorded rate; no frontend streams"));
    report.insert("request".into(), json!("star, seed, title and artist of the release example receipt; no controls"));
    let mut cpu_osu: Map<String, Value> = Map::new();
    for song in &songs {
        if let Some(osu) = charts.as_ref().and_then(|dir| std::fs::read_to_string(dir.join(format!("{song}.osu"))).ok()) {
            cpu_osu.insert(song.clone(), json!(osu));
        }
    }
    let mut results = report.get("configs").and_then(Value::as_object).cloned().unwrap_or_default();
    for name in &configs {
        let mut rows = Map::new();
        for song in &songs {
            eprintln!("{name} {song}");
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--model-dir", paths.model_dir.to_str().unwrap(), "--reference", paths.reference.to_str().unwrap(), "--assets", paths.assets.to_str().unwrap(), "--child", name, song])
                .output()
                .unwrap();
            let mut row = if output.status.success() { serde_json::from_slice(&output.stdout).unwrap() } else { json!({"failed": String::from_utf8_lossy(&output.stderr), "osu": ""}) };
            let osu = row["osu"].as_str().unwrap().to_string();
            row.as_object_mut().unwrap().remove("osu");
            if name == "cpu" && output.status.success() {
                cpu_osu.insert(song.clone(), json!(osu));
                if let Some(dir) = &charts {
                    std::fs::create_dir_all(dir).unwrap();
                    std::fs::write(dir.join(format!("{song}.osu")), &osu).unwrap();
                    let request = json!({"input": report["input"], "sample_rate": row["sample_rate"], "request": row["request"], "seed": row["seed"], "objects": row["objects"]});
                    std::fs::write(dir.join(format!("{song}.json")), serde_json::to_string_pretty(&request).unwrap() + "\n").unwrap();
                }
            }
            if let (Some(cpu), true) = (cpu_osu.get(song).and_then(Value::as_str), output.status.success()) {
                row["same_hit_objects_as_cpu"] = json!(hit_objects(cpu) == hit_objects(&osu));
                row["same_osu_as_cpu"] = json!(cpu == osu);
            }
            rows.insert(song.clone(), row);
            results.insert(name.clone(), Value::Object(rows.clone()));
            report.insert("configs".into(), Value::Object(results.clone()));
            std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap() + "\n").unwrap();
        }
    }
}
