//! Runs every `conformance/gameplay/*.json` fixture; the schema is in that directory's README.

mod common;

use common::{Delivery, Page, Step, all_at_once, complete, recut, run};
use ensomi_gameplay::{
    Chart, ChartObject, ChartToken, InputEvent, Judge, JudgementEvent, Part, Phase, Ruleset,
    Session, Tier, TokenKind,
};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    #[allow(dead_code)]
    description: String,
    /// Absent for a streamed chart, which arrives in `page` steps.
    chart: Option<ChartJson>,
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
    Page(PageJson),
    DecidedThroughMs(f64),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageJson {
    objects: Option<Vec<ObjectJson>>,
    tokens: Option<Vec<TokenJson>>,
    coverage_ms: f64,
    end_of_stream: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenJson {
    lane: u32,
    time_ms: f64,
    kind: String,
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
    step: Option<usize>,
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

fn object(o: &ObjectJson) -> ChartObject {
    ChartObject {
        lane: o.lane,
        start_ms: o.start_ms,
        end_ms: o.end_ms,
    }
}

/// The ruleset, the steps, and the chart unless it is streamed.
fn setup(fixture: &Fixture) -> (Option<Chart>, Ruleset, Vec<Step>) {
    let chart = fixture.chart.as_ref().map(|chart| Chart {
        duration_ms: chart.duration_ms,
        objects: chart.objects.iter().map(object).collect(),
    });
    let judge = match fixture.ruleset.judge.as_str() {
        "A" => Judge::A,
        "B" => Judge::B,
        "C" => Judge::C,
        "D" => Judge::D,
        "E" => Judge::E,
        other => panic!("unknown judge {other}"),
    };
    let steps: Vec<Step> = fixture
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
            StepJson::Page(page) => Step::Page(Page {
                delivery: match (&page.objects, &page.tokens) {
                    (Some(objects), None) => {
                        Delivery::Objects(objects.iter().map(object).collect())
                    }
                    (None, Some(tokens)) => Delivery::Tokens(
                        tokens
                            .iter()
                            .map(|token| ChartToken {
                                lane: token.lane,
                                time_ms: token.time_ms,
                                kind: match token.kind.as_str() {
                                    "tap" => TokenKind::Tap,
                                    "hold_start" => TokenKind::HoldStart,
                                    "hold_end" => TokenKind::HoldEnd,
                                    other => panic!("unknown token kind {other}"),
                                },
                            })
                            .collect(),
                    ),
                    _ => panic!("a page has objects or tokens"),
                },
                coverage_ms: page.coverage_ms,
                end_of_stream: page.end_of_stream,
            }),
            StepJson::DecidedThroughMs(_) => Step::DecidedThrough,
        })
        .collect();
    let streamed = steps.iter().any(|step| matches!(step, Step::Page(_)));
    assert_ne!(chart.is_some(), streamed, "a fixture has a chart or pages");
    (chart, Ruleset { judge }, steps)
}

fn session(chart: &Option<Chart>, ruleset: Ruleset) -> Session {
    match chart {
        Some(chart) => complete(chart, ruleset),
        None => Session::streaming(ruleset),
    }
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
        let outcome = run(session(&chart, ruleset), &steps);
        check_events(
            name,
            "as written",
            &outcome.events,
            &fixture.expected.events,
        );
        for (index, (step, expected)) in outcome
            .event_steps
            .iter()
            .zip(&fixture.expected.events)
            .enumerate()
        {
            if let Some(expected_step) = expected.step {
                assert_eq!(*step, expected_step, "{name}: step of event {index}");
            }
        }
        let expected_decided: Vec<f64> = fixture
            .steps
            .iter()
            .filter_map(|step| match step {
                StepJson::DecidedThroughMs(ms) => Some(*ms),
                _ => None,
            })
            .collect();
        assert_eq!(
            outcome.decided_through, expected_decided,
            "{name}: decided through"
        );
        check_score(name, "as written", &outcome.score, &fixture.expected.score);
        assert_eq!(
            outcome.late_inputs, fixture.expected.late_inputs,
            "{name}: late inputs"
        );
        assert!(outcome.resolved, "{name}: resolved");
    }
}

/// A streamed fixture has the outcome of its objects delivered before play.
#[test]
fn streamed_fixtures_match_the_chart_delivered_at_once() {
    for (name, fixture) in &fixtures() {
        let (chart, ruleset, steps) = setup(fixture);
        if chart.is_some() {
            continue;
        }
        let label = "delivered at once";
        let outcome = run(Session::streaming(ruleset), &all_at_once(&steps));
        check_events(name, label, &outcome.events, &fixture.expected.events);
        check_score(name, label, &outcome.score, &fixture.expected.score);
        assert_eq!(
            outcome.late_inputs, fixture.expected.late_inputs,
            "{name} [{label}]: late inputs"
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
            let outcome = run(session(&chart, ruleset), &recut(&steps, period_ms));
            let label = format!("advance every {period_ms} ms");
            check_events(name, &label, &outcome.events, &fixture.expected.events);
            check_score(name, &label, &outcome.score, &fixture.expected.score);
            assert_eq!(outcome.late_inputs, 0, "{name} [{label}]: late inputs");
        }
    }
}
