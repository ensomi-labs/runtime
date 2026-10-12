//! Time to first playable through the bindings, as the Swift API runs it: per song, the seconds from the start of a
//! generation to the scheduler's first `Ready` at position 0, playing while generating (`AheadOfPlay`) against the
//! whole chart, from PCM and from a stored `Prepared`; then to `Done`, with the throughput r (committed chart
//! milliseconds per wall millisecond of work) at `Ready` and at the end.
//!
//! Input as for `providers`: the recorded PCM averaged to mono float32 at its recorded rate, and the star, seed,
//! title and artist of the release example. The lead: judge C's settle lead, the render lead given (the app's
//! default scroll time is 11485 / 25 = 459.4 ms), no margin. "From a stored Prepared" starts from its bytes in
//! memory, reading them included. CPU provider, one thread. One `prepare` per song first warms the graphs and gives
//! the bytes.
//!
//! Usage: stream --model-dir <ensomi-r2> --reference <runtime-ref> --out <report.json> [--runs 3]
//!        [--render-lead-ms 459.4] [song...]
use ensomi_gameplay::{Judge, Ruleset};
use ensomi_generate_ort::recording::Recording;
use ensomi_runtime_ffi::generator::{ChartControls, ChartGenerator, ExecutionProvider, GenerateChartRequest, GeneratorOptions, Prepared};
use ensomi_runtime_ffi::scheduler::{ChartScheduler, LeadPolicy, ScheduleMode, SchedulerAction};
use ensomi_runtime_ffi::GameplaySession;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

const SONGS: [&str; 4] = ["hesitation-snow", "saigetsu", "catch-the-moment", "adventure"];

fn median(values: &mut [f64]) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}

/// One generation: seconds to the first `Ready` and to `Done`, r at both, and what the receipt says of the passes.
fn run(scheduler: Arc<ChartScheduler>, started: Instant) -> Value {
    let session = GameplaySession::streaming(Ruleset { judge: Judge::C });
    scheduler.start(0.0, session.clone());
    let ready = scheduler.work().unwrap();
    let ready_seconds = started.elapsed().as_secs_f64();
    let (rate_at_ready, coverage_at_ready) = (scheduler.rate(), session.coverage_ms());
    assert_eq!(ready, SchedulerAction::Ready { join_ms: 0.0 });
    assert_eq!(scheduler.work().unwrap(), SchedulerAction::Done);
    let done_seconds = started.elapsed().as_secs_f64();
    let result = scheduler.result().unwrap();
    assert!(session.coverage_ms().is_infinite());
    assert_eq!(session.visible_objects(f64::NEG_INFINITY, f64::INFINITY).len(), result.chart.objects.len());
    let receipt: Value = serde_json::from_str(&result.receipt.receipt_json).unwrap();
    let difficulty = &receipt["controls"]["difficulty"];
    json!({
        "ready_seconds": ready_seconds, "done_seconds": done_seconds, "coverage_at_ready_ms": coverage_at_ready,
        "rate_at_ready": rate_at_ready, "rate_at_done": scheduler.rate(), "objects": result.chart.objects.len(),
        "passes": difficulty["passes"].as_array().map(Vec::len), "would_repass": difficulty["would_repass"], "seconds": receipt["seconds"],
        "osu_fnv": format!("{:x}", result.osu.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))),
    })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut model_dir, mut reference, mut out, mut runs, mut render_lead_ms) = (None, None, None, 3, 459.4);
    let mut songs: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model-dir" => model_dir = args.next().map(PathBuf::from),
            "--reference" => reference = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--runs" => runs = args.next().unwrap().parse().unwrap(),
            "--render-lead-ms" => render_lead_ms = args.next().unwrap().parse().unwrap(),
            _ => songs.push(arg),
        }
    }
    let (model_dir, reference, out) = (model_dir.expect("--model-dir"), reference.expect("--reference"), out.expect("--out"));
    if songs.is_empty() {
        songs = SONGS.iter().map(|s| s.to_string()).collect();
    }
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/resampler");
    let options = GeneratorOptions { provider: ExecutionProvider::Cpu, resampler_directory: Some(assets.to_string_lossy().into_owned()), low_memory: None };
    let generator = ChartGenerator::open(model_dir.join("onnx").to_string_lossy().into_owned(), options).unwrap();
    let lead = LeadPolicy { ruleset: Ruleset { judge: Judge::C }, render_lead_ms, margin_ms: 0.0 };
    let mut report = Map::new();
    report.insert("what".into(), json!("seconds from the start of a generation to the first Ready at 0 and to Done, through the UniFFI binding, native CPU, one ONNX Runtime thread"));
    report.insert("lead".into(), json!({"settle_ms": lead.ruleset.settle_lead_ms(), "render_ms": render_lead_ms, "margin_ms": 0.0}));
    let mut songs_report = Map::new();
    for song in &songs {
        let recording = Recording::open(&reference.join("bulk").join(song)).unwrap();
        let audio = recording.event("audio").unwrap();
        let channels = *audio["pcm"].shape.get(1).unwrap_or(&1);
        let pcm: Vec<f32> = audio["pcm"].f64s().chunks_exact(channels).map(|f| (f.iter().sum::<f64>() / channels as f64) as f32).collect();
        let sample_rate = audio["sample_rate"].f64s()[0];
        let receipt: Value = serde_json::from_slice(&std::fs::read(model_dir.join("examples").join(song).join("receipt.json")).unwrap()).unwrap();
        let request = GenerateChartRequest {
            star: receipt["star"].as_f64().unwrap(),
            seed: receipt["seed"].as_u64().unwrap(),
            controls: ChartControls { long_notes: None, sections: Vec::new() },
            title: receipt["metadata"]["title"].as_str().map(str::to_string),
            artist: receipt["metadata"]["artist"].as_str().map(str::to_string),
            audio_name: None,
        };
        let started = Instant::now();
        let bytes = generator.prepare(pcm.clone(), sample_rate, None).unwrap().bytes();
        let prepare_seconds = started.elapsed().as_secs_f64();
        let mut variants: Map<String, Value> = Map::new();
        for _ in 0..runs {
            for (name, mode, stored) in [("stream_pcm", ScheduleMode::AheadOfPlay, false), ("whole_pcm", ScheduleMode::WholeChart, false), ("stream_prepared", ScheduleMode::AheadOfPlay, true), ("whole_prepared", ScheduleMode::WholeChart, true)] {
                let (pcm, bytes) = (pcm.clone(), bytes.clone());
                let started = Instant::now();
                let scheduler = if stored {
                    ChartScheduler::from_prepared(generator.clone(), Prepared::new(bytes).unwrap(), request.clone(), mode, lead).unwrap()
                } else {
                    ChartScheduler::from_pcm(generator.clone(), pcm, sample_rate, request.clone(), mode, lead).unwrap()
                };
                let row = run(scheduler, started);
                println!("{song} {name}: ready {:.3} s, done {:.3} s, r at ready {}", row["ready_seconds"], row["done_seconds"], row["rate_at_ready"]);
                variants.entry(name).or_insert_with(|| json!([])).as_array_mut().unwrap().push(row);
            }
        }
        let summary: Map<String, Value> = variants
            .iter()
            .map(|(name, rows)| {
                let field = |key: &str| median(&mut rows.as_array().unwrap().iter().filter_map(|r| r[key].as_f64()).collect::<Vec<_>>());
                (name.clone(), json!({"ready_seconds": field("ready_seconds"), "done_seconds": field("done_seconds"), "rate_at_ready": field("rate_at_ready"), "rate_at_done": field("rate_at_done")}))
            })
            .collect();
        songs_report.insert(song.clone(), json!({"sample_rate": sample_rate, "song_ms": pcm.len() as f64 / sample_rate * 1000.0, "prepare_seconds": prepare_seconds, "prepared_bytes": bytes.len(), "median": summary, "runs": variants}));
        report.insert("songs".into(), Value::Object(songs_report.clone()));
        std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap() + "\n").unwrap();
    }
}
