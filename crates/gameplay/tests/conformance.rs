//! Runs every `conformance/gameplay/*.json` fixture; the schema is in that directory's README.

mod common;

use common::{Step, recut, run};
use ensomi_gameplay::{
    Chart, ChartObject, InputEvent, Judge, JudgementEvent, Part, Phase, Ruleset, Tier,
};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    #[allow(dead_code)]
    description: String,
    chart: ChartJson,
    ruleset: RulesetJson,
    steps: Vec<StepJson>,
    expected: Expected,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChartJson {
    duration_ms: u32,
    objects: Vec<ObjectJson>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectJson {
    lane: u32,
    start_ms: u32,
    end_ms: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesetJson {
    judge: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum StepJson {
    Input(InputJson),
    Advance(f64),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputJson {
    lane: u32,
    phase: String,
    time_ms: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    events: Vec<EventJson>,
    score: ScoreJson,
    late_inputs: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventJson {
    object: u32,
    lane: u32,
    part: String,
    tier: String,
    offset_ms: Option<f64>,
    time_ms: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoreJson {
    tier_counts: TierCountsJson,
    accuracy: f64,
    combo: u32,
    max_combo: u32,
    suggested_audio_offset_ms: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TierCountsJson {
    #[serde(rename = "bigP")]
    big_p: u32,
    p1: u32,
    p2: u32,
    p3: u32,
    g: u32,
    m: u32,
}

fn tier(name: &str) -> Tier {
    match name {
        "bigP" => Tier::BigP,
        "p1" => Tier::P1,
        "p2" => Tier::P2,
        "p3" => Tier::P3,
        "g" => Tier::G,
        "m" => Tier::M,
        _ => panic!("unknown tier {name}"),
    }
}

fn fixtures() -> Vec<(String, Fixture)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/gameplay");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixture directory")
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).unwrap();
            let fixture =
                serde_json::from_str(&text).unwrap_or_else(|error| panic!("{name}: {error}"));
            (name, fixture)
        })
        .collect()
}

fn setup(fixture: &Fixture) -> (Chart, Ruleset, Vec<Step>) {
    let chart = Chart {
        duration_ms: fixture.chart.duration_ms,
        objects: fixture
            .chart
            .objects
            .iter()
            .map(|o| ChartObject {
                lane: o.lane,
                start_ms: o.start_ms,
                end_ms: o.end_ms,
            })
            .collect(),
    };
    let judge = match fixture.ruleset.judge.as_str() {
        "A" => Judge::A,
        "B" => Judge::B,
        "C" => Judge::C,
        "D" => Judge::D,
        "E" => Judge::E,
        other => panic!("unknown judge {other}"),
    };
    let steps = fixture
        .steps
        .iter()
        .map(|step| match step {
            StepJson::Input(input) => Step::Input(InputEvent {
                lane: input.lane,
                phase: match input.phase.as_str() {
                    "press" => Phase::Press,
                    "release" => Phase::Release,
                    other => panic!("unknown phase {other}"),
                },
                time_ms: input.time_ms,
            }),
            StepJson::Advance(watermark_ms) => Step::Advance(*watermark_ms),
        })
        .collect();
    (chart, Ruleset { judge }, steps)
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

fn close_option(a: Option<f64>, b: Option<f64>, tolerance: f64) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => close(a, b, tolerance),
        (None, None) => true,
        _ => false,
    }
}

fn check_events(name: &str, label: &str, actual: &[JudgementEvent], expected: &[EventJson]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{name} [{label}]: event count\nactual: {actual:#?}"
    );
    for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
        let part = if e.part == "head" {
            Part::Head
        } else {
            Part::Tail
        };
        let ok = a.object == e.object
            && a.lane == e.lane
            && a.part == part
            && a.tier == tier(&e.tier)
            && close_option(a.offset_ms, e.offset_ms, 1e-6)
            && close(a.time_ms, e.time_ms, 1e-6);
        assert!(
            ok,
            "{name} [{label}]: event {index} is {a:?}, expected object {} lane {} part {} tier {} offset {:?} time {}",
            e.object, e.lane, e.part, e.tier, e.offset_ms, e.time_ms
        );
    }
}

fn check_score(name: &str, label: &str, actual: &ensomi_gameplay::Score, expected: &ScoreJson) {
    let c = &actual.tier_counts;
    let e = &expected.tier_counts;
    assert_eq!(
        (c.big_p, c.p1, c.p2, c.p3, c.g, c.m),
        (e.big_p, e.p1, e.p2, e.p3, e.g, e.m),
        "{name} [{label}]: tier counts"
    );
    assert!(
        close(actual.accuracy, expected.accuracy, 1e-9),
        "{name} [{label}]: accuracy {} vs {}",
        actual.accuracy,
        expected.accuracy
    );
    assert_eq!(
        (actual.combo, actual.max_combo),
        (expected.combo, expected.max_combo),
        "{name} [{label}]: combo"
    );
    assert!(
        close_option(
            actual.suggested_audio_offset_ms,
            expected.suggested_audio_offset_ms,
            1e-9
        ),
        "{name} [{label}]: suggested offset {:?} vs {:?}",
        actual.suggested_audio_offset_ms,
        expected.suggested_audio_offset_ms
    );
}

#[test]
fn fixtures_reproduce_expected_output() {
    let fixtures = fixtures();
    assert!(!fixtures.is_empty());
    for (name, fixture) in &fixtures {
        let (chart, ruleset, steps) = setup(fixture);
        let outcome = run(&chart, ruleset, &steps);
        check_events(
            name,
            "as written",
            &outcome.events,
            &fixture.expected.events,
        );
        check_score(name, "as written", &outcome.score, &fixture.expected.score);
        assert_eq!(
            outcome.late_inputs, fixture.expected.late_inputs,
            "{name}: late inputs"
        );
    }
}

/// A fixture without late deliveries keeps its output under any advance cadence.
#[test]
fn fixtures_do_not_depend_on_the_advance_schedule() {
    for (name, fixture) in &fixtures() {
        if fixture.expected.late_inputs > 0 {
            continue;
        }
        let (chart, ruleset, steps) = setup(fixture);
        for period_ms in [1e9, 997.0, 100.0, 13.0, 0.5] {
            let outcome = run(&chart, ruleset, &recut(&steps, period_ms));
            let label = format!("advance every {period_ms} ms");
            check_events(name, &label, &outcome.events, &fixture.expected.events);
            check_score(name, &label, &outcome.score, &fixture.expected.score);
            assert_eq!(outcome.late_inputs, 0, "{name} [{label}]: late inputs");
        }
    }
}
