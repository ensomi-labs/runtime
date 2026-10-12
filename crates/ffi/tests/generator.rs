//! The chart generator through the bindings, with the released model read from beside the repository where
//! bings-mac keeps it (conformance/generate/README.md); without it the test passes without running.

use ensomi_gameplay::{Judge, Ruleset};
use ensomi_runtime_ffi::generator::{
    ChartControls, ChartGenerator, ExecutionProvider, GenerateChartRequest, GenerationListener, GenerationPhase,
    GeneratorError, GeneratorOptions, Prepared,
};
use ensomi_runtime_ffi::scheduler::{ChartScheduler, LeadPolicy, ScheduleMode, SchedulerAction};
use ensomi_runtime_ffi::GameplaySession;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// A listener that panics when the generation reaches the rows, as a bug in the core would.
struct PanicsAtRows;

impl GenerationListener for PanicsAtRows {
    fn progress(&self, phase: GenerationPhase) {
        assert_ne!(phase, GenerationPhase::Rows, "a listener that panics");
    }
}

/// Reproducible noise at 22050 Hz, the rate BeatThis reads without a resampler table.
fn noise(seconds: usize) -> Vec<f32> {
    let mut state = 1u32;
    (0..seconds * 22_050)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / (1 << 24) as f32 - 0.5
        })
        .collect()
}

/// The released model where bings-mac keeps it, or `None` when it is absent.
fn model() -> Option<PathBuf> {
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2/onnx");
    if !model.exists() {
        eprintln!("skipped: {} is absent", model.display());
    }
    model.exists().then_some(model)
}

fn open(model: &Path) -> Arc<ChartGenerator> {
    let options = GeneratorOptions { provider: ExecutionProvider::Cpu, resampler_directory: None, low_memory: None };
    ChartGenerator::open(model.to_string_lossy().into_owned(), options).unwrap()
}

fn request() -> GenerateChartRequest {
    GenerateChartRequest { star: 3.0, seed: 0, controls: ChartControls { long_notes: None, sections: Vec::new() }, title: None, artist: None, audio_name: None }
}

#[test]
fn a_panic_ends_its_generation_with_an_error_and_the_next_generation_works() {
    let Some(model) = model() else { return };
    let generator = open(&model);
    let request = request();

    match generator.generate(noise(20), 22_050.0, request.clone(), Some(Arc::new(PanicsAtRows)), None) {
        Err(GeneratorError::Generation { code, message }) => {
            assert_eq!(code, "generation_failed");
            assert!(message.contains("a listener that panics"), "{message}");
        }
        other => panic!("the generation did not fail with the panic: {other:?}"),
    }
    let result = generator.generate(noise(20), 22_050.0, request, None, None).unwrap();
    assert!(!result.chart.objects.is_empty());
}

#[test]
fn the_scheduler_puts_the_chart_into_the_session_page_by_page() {
    let Some(model) = model() else { return };
    let generator = open(&model);
    let prepared = generator.prepare(noise(30), 22_050.0, None).unwrap();
    let stored = Prepared::new(prepared.bytes()).unwrap();
    let lead = LeadPolicy { ruleset: Ruleset { judge: Judge::C }, render_lead_ms: 500.0, margin_ms: 0.0 };

    let whole = ChartScheduler::from_prepared(generator.clone(), stored.clone(), request(), ScheduleMode::WholeChart, lead).unwrap();
    whole.start(0.0, GameplaySession::streaming(Ruleset { judge: Judge::C }));
    assert_eq!(whole.work().unwrap(), SchedulerAction::Ready { join_ms: 0.0 });
    assert_eq!(whole.work().unwrap(), SchedulerAction::Done);
    let whole = whole.result().unwrap();
    assert_eq!(whole.osu, generator.generate(noise(30), 22_050.0, request(), None, None).unwrap().osu);

    let streamed = ChartScheduler::from_prepared(generator.clone(), stored, request(), ScheduleMode::AheadOfPlay, lead).unwrap();
    let session = GameplaySession::streaming(Ruleset { judge: Judge::C });
    streamed.start(0.0, session.clone());
    assert_eq!(streamed.work().unwrap(), SchedulerAction::Ready { join_ms: 0.0 });
    assert!(session.coverage_ms() >= 500.0 && session.coverage_ms() < whole.chart.duration_ms as f64);
    assert!(streamed.rate().unwrap() > 0.0);
    assert_eq!(streamed.work().unwrap(), SchedulerAction::Done);
    let result = streamed.result().unwrap();
    assert!(session.coverage_ms().is_infinite());
    let objects: Vec<_> = session.visible_objects(f64::NEG_INFINITY, f64::INFINITY).iter().map(|o| (o.lane, o.start_time_ms as u32, o.end_time_ms.map(|e| e as u32))).collect();
    assert_eq!(objects, result.chart.objects.iter().map(|o| (o.lane, o.start_ms, o.end_ms)).collect::<Vec<_>>());
    // One arrangement in either mode: the streamed chart is the whole one.
    let receipt: serde_json::Value = serde_json::from_str(&result.receipt.receipt_json).unwrap();
    assert_eq!(receipt["controls"]["difficulty"]["passes"].as_array().unwrap().len(), 1);
    assert_eq!(result.osu, whole.osu);
}

#[test]
fn stop_ends_a_scheduler_that_waits_for_play() {
    let Some(model) = model() else { return };
    let generator = open(&model);
    let lead = LeadPolicy { ruleset: Ruleset { judge: Judge::C }, render_lead_ms: 500.0, margin_ms: 0.0 };
    let scheduler = ChartScheduler::from_pcm(generator, noise(30), 22_050.0, request(), ScheduleMode::AheadOfPlay, lead).unwrap();
    scheduler.start(0.0, GameplaySession::streaming(Ruleset { judge: Judge::C }));
    assert_eq!(scheduler.work().unwrap(), SchedulerAction::Ready { join_ms: 0.0 });
    // Heard at the start of play, with the chart well ahead: work waits for play to move on.
    scheduler.heard(0.0, 0.0);
    scheduler.heard(1.0, 1.0);
    let worker = {
        let scheduler = scheduler.clone();
        std::thread::spawn(move || scheduler.work())
    };
    std::thread::sleep(Duration::from_millis(200));
    scheduler.stop();
    assert_eq!(worker.join().unwrap().unwrap(), SchedulerAction::Done);
}

#[test]
fn a_start_at_a_position_joins_where_no_hold_is_open() {
    let Some(model) = model() else { return };
    let generator = open(&model);
    let lead = LeadPolicy { ruleset: Ruleset { judge: Judge::C }, render_lead_ms: 500.0, margin_ms: 0.0 };
    let scheduler = ChartScheduler::from_pcm(generator, noise(30), 22_050.0, request(), ScheduleMode::AheadOfPlay, lead).unwrap();
    let session = GameplaySession::streaming(Ruleset { judge: Judge::C });
    scheduler.start(10_000.0, session.clone());
    let SchedulerAction::Ready { join_ms } = scheduler.work().unwrap() else { panic!("not ready") };
    assert!(join_ms >= 10_500.0, "{join_ms}");
    assert_eq!(scheduler.work().unwrap(), SchedulerAction::Done);
    let chart = scheduler.result().unwrap().chart;
    // No hold of the chart is open at the join point, and the session holds the chart from there on.
    assert!(chart.objects.iter().all(|o| o.end_ms.is_none_or(|end| !(f64::from(o.start_ms) < join_ms && join_ms < f64::from(end)))));
    let expected: Vec<_> = chart.objects.iter().filter(|o| f64::from(o.start_ms) >= join_ms).map(|o| (o.lane, o.start_ms, o.end_ms)).collect();
    let objects: Vec<_> = session.visible_objects(f64::NEG_INFINITY, f64::INFINITY).iter().map(|o| (o.lane, o.start_time_ms as u32, o.end_time_ms.map(|e| e as u32))).collect();
    assert!(!expected.is_empty());
    assert_eq!(objects, expected);
}

#[test]
fn a_request_changes_the_chart_only_while_the_audio_stages_run() {
    let Some(model) = model() else { return };
    let generator = open(&model);
    let lead = LeadPolicy { ruleset: Ruleset { judge: Judge::C }, render_lead_ms: 500.0, margin_ms: 0.0 };
    let scheduler = ChartScheduler::from_pcm(generator.clone(), noise(30), 22_050.0, request(), ScheduleMode::AheadOfPlay, lead).unwrap();
    let refused = |result: Result<(), GeneratorError>| matches!(result, Err(GeneratorError::Generation { code, .. }) if code == "unsupported_control");
    let changed = GenerateChartRequest { star: 4.5, ..request() };
    // The audio name goes into the options; it stays, as the seed does.
    assert!(refused(scheduler.request(GenerateChartRequest { audio_name: Some("song.ogg".to_string()), ..changed.clone() })));
    assert!(refused(scheduler.request(GenerateChartRequest { seed: 1, ..changed.clone() })));
    scheduler.request(changed.clone()).unwrap();
    scheduler.start(0.0, GameplaySession::streaming(Ruleset { judge: Judge::C }));
    assert_eq!(scheduler.work().unwrap(), SchedulerAction::Ready { join_ms: 0.0 });
    // Pages are committed: refused, while `work()` generates the rest on another thread.
    let worker = {
        let scheduler = scheduler.clone();
        std::thread::spawn(move || scheduler.work())
    };
    assert!(refused(scheduler.request(request())));
    assert_eq!(worker.join().unwrap().unwrap(), SchedulerAction::Done);
    assert_eq!(scheduler.result().unwrap().osu, generator.generate(noise(30), 22_050.0, changed, None, None).unwrap().osu);
}
