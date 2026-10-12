//! Requests to the scheduler during generation, with the released model read from beside the repository where
//! bings-mac keeps it (conformance/generate/README.md); without it the tests pass without running.
use ensomi_generate::{Action, Audio, Event, Lead, Mode, Models, Options, Request, Samples, Scheduler, Source};
use ensomi_generate_ort::{package, Config, Sessions};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

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

/// The released model and its sessions, or `None` when it is absent.
fn model() -> Option<(Arc<Models>, Sessions)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2/onnx");
    if !dir.exists() {
        eprintln!("skipped: {} is absent", dir.display());
        return None;
    }
    let models = package::open(&dir).unwrap().0;
    let sessions = Sessions::load(&dir, &models, &Config::default()).unwrap();
    Some((Arc::new(models), sessions))
}

fn request(json: &str, seed: u64) -> Request {
    Request::from_json(json, seed, false).unwrap()
}

/// Ahead of play from 30 s of noise, with play asked to start at 0 so that pages are committed.
fn scheduler(models: &Arc<Models>, request: Request) -> Scheduler {
    let audio = Audio { samples: Samples::F32(noise(30)), channels: 1, sample_rate: 22_050.0, mel: None, beatthis: None, resampler: None };
    let start = Instant::now();
    let options = Options::new(Rc::new(move || start.elapsed().as_secs_f64() * 1000.0));
    let lead = Lead { settle_ms: 200.0, render_ms: 500.0, margin_ms: 0.0 };
    let mut scheduler = Scheduler::new(models.clone(), Source::Audio(audio), request, options, Mode::AheadOfPlay, lead);
    scheduler.handle(Event::Start { at_ms: 0.0 }).unwrap();
    scheduler
}

/// Runs the scheduler to `Done`, calling `each` with every action before acting on it, and returns the `.osu`.
fn drive(scheduler: &mut Scheduler, sessions: &mut Sessions, mut each: impl FnMut(&mut Scheduler, &Action)) -> String {
    loop {
        let action = scheduler.next().unwrap();
        each(&mut *scheduler, &action);
        match action {
            Action::Run { graph, inputs } => scheduler.handle(Event::Outputs(sessions.run(&graph, inputs).unwrap())).unwrap(),
            Action::Done => return scheduler.result().unwrap().osu.clone(),
            Action::Commit(_) | Action::Ready { .. } | Action::Wait => {}
        }
    }
}

const FIRST: &str = r#"{"star": 3}"#;
const SECOND: &str = r#"{"star": 4.5, "controls": {"long_notes": 1}}"#;

#[test]
fn a_request_while_the_audio_stages_run_gives_the_chart_of_a_fresh_generation() {
    let Some((models, mut sessions)) = model() else { return };
    let mut changed = scheduler(&models, request(FIRST, 0));
    let mut sent = false;
    // The first graph run is BeatThis's, an audio stage.
    let osu = drive(&mut changed, &mut sessions, |scheduler, action| {
        if !sent && matches!(action, Action::Run { .. }) {
            scheduler.handle(Event::Request(request(SECOND, 0))).unwrap();
            sent = true;
        }
    });
    assert_eq!(changed.result().unwrap().receipt.star, 4.5);
    assert_eq!(osu, drive(&mut scheduler(&models, request(SECOND, 0)), &mut sessions, |_, _| {}));
}

#[test]
fn a_request_once_pages_are_committed_is_refused_and_the_chart_stays() {
    let Some((models, mut sessions)) = model() else { return };
    let mut refused = None;
    let osu = drive(&mut scheduler(&models, request(FIRST, 0)), &mut sessions, |scheduler, action| {
        if refused.is_none() && matches!(action, Action::Commit(_)) {
            refused = Some(scheduler.handle(Event::Request(request(SECOND, 0))).unwrap_err());
        }
    });
    let error = refused.expect("no page was committed");
    assert!(error.starts_with("unsupported_control:"), "{error}");
    assert_eq!(osu, drive(&mut scheduler(&models, request(FIRST, 0)), &mut sessions, |_, _| {}));
}

#[test]
fn a_request_that_changes_the_seed_audio_or_metadata_is_refused() {
    let Some((models, mut sessions)) = model() else { return };
    let mut running = scheduler(&models, request(FIRST, 0));
    for (json, seed) in [(SECOND, 1), (r#"{"star": 4.5, "audio": {"uri": "other.ogg"}}"#, 0), (r#"{"star": 4.5, "title": "Other"}"#, 0)] {
        let error = running.handle(Event::Request(request(json, seed))).unwrap_err();
        assert!(error.starts_with("unsupported_control:"), "{error}");
    }
    // The running request stays.
    drive(&mut running, &mut sessions, |_, _| {});
    let receipt = &running.result().unwrap().receipt;
    assert_eq!((receipt.star, receipt.seed), (3.0, 0));
}
