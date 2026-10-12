//! Reads every `conformance/gameplay/osu/*.osu` file and compares with the `.json` beside it; the schema
//! is in `conformance/gameplay/README.md`.

use ensomi_gameplay::{ChartError, OsuError, parse_osu};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    #[allow(dead_code)]
    description: String,
    expected: Expected,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    chart: Option<ChartJson>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    artist: Option<String>,
    error: Option<ErrorJson>,
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
struct ErrorJson {
    kind: String,
    keys: Option<i64>,
    line: Option<String>,
    chart_error: Option<String>,
    object: Option<u32>,
}

fn chart_error_name(error: ChartError) -> (&'static str, u32) {
    match error {
        ChartError::InvalidLane { object } => ("invalid_lane", object),
        ChartError::InvalidHold { object } => ("invalid_hold", object),
        ChartError::OutsideDuration { object } => ("outside_duration", object),
        ChartError::NotSorted { object } => ("not_sorted", object),
        ChartError::SameLaneOverlap { object } => ("same_lane_overlap", object),
        other => panic!("{other:?} cannot come from a .osu file"),
    }
}

#[test]
fn osu_files_parse_as_expected() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/gameplay/osu");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixture directory")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "osu"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty());
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        let fixture: Fixture =
            serde_json::from_str(&std::fs::read_to_string(path.with_extension("json")).unwrap())
                .unwrap_or_else(|error| panic!("{name}: {error}"));
        let expected = fixture.expected;
        match (parse_osu(&text), expected.error) {
            (Ok(beatmap), None) => {
                let chart = expected.chart.expect("chart or error");
                assert_eq!(beatmap.chart.duration_ms, chart.duration_ms, "{name}");
                let objects: Vec<_> = beatmap
                    .chart
                    .objects
                    .iter()
                    .map(|o| (o.lane, o.start_ms, o.end_ms))
                    .collect();
                let expected_objects: Vec<_> = chart
                    .objects
                    .iter()
                    .map(|o| (o.lane, o.start_ms, o.end_ms))
                    .collect();
                assert_eq!(objects, expected_objects, "{name}");
                assert_eq!(beatmap.title, expected.title, "{name}");
                assert_eq!(beatmap.artist, expected.artist, "{name}");
            }
            (Err(error), Some(expected)) => {
                let ok = match (&error, expected.kind.as_str()) {
                    (OsuError::UnsupportedMode, "unsupported_mode") => true,
                    (OsuError::UnsupportedKeyCount { keys }, "unsupported_key_count") => {
                        expected.keys == Some(*keys)
                    }
                    (OsuError::UnsupportedHitObject { line }, "unsupported_hit_object") => {
                        expected.line.as_ref() == Some(line)
                    }
                    (OsuError::InvalidChart { error }, "invalid_chart") => {
                        let (kind, object) = chart_error_name(*error);
                        expected.chart_error.as_deref() == Some(kind)
                            && expected.object == Some(object)
                    }
                    _ => false,
                };
                assert!(ok, "{name}: {error:?}");
            }
            (actual, _) => panic!("{name}: unexpected {actual:?}"),
        }
    }
}
