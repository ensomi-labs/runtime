//! Per-frame cost of a session over a long chart, played at 60 Hz. Ignored: it measures time, so run it
//! on its own, in release mode:
//! `cargo test --release -p ensomi-gameplay --test frame_cost -- --ignored --nocapture`

use ensomi_gameplay::{Chart, ChartObject, InputEvent, Judge, Phase, Ruleset, Session};
use std::time::Instant;

const FRAME_MS: f64 = 1000.0 / 60.0;

/// `count` objects over `count * 60` ms in 4 lanes, one every 240 ms per lane: every third a hold of
/// 100 ms, the others taps.
fn chart(count: u32) -> Chart {
    let objects: Vec<ChartObject> = (0..count)
        .map(|i| {
            let start_ms = 1000 + (i / 4) * 240 + (i % 4) * 60;
            ChartObject {
                lane: i % 4,
                start_ms,
                end_ms: (i % 3 == 0).then_some(start_ms + 100),
            }
        })
        .collect();
    let duration_ms = objects
        .iter()
        .map(|o| o.end_ms.unwrap_or(o.start_ms))
        .max()
        .unwrap();
    Chart {
        duration_ms,
        objects,
    }
}

/// A press at every note's start and a release at every hold's end, in time order.
fn inputs(chart: &Chart) -> Vec<InputEvent> {
    let mut inputs: Vec<InputEvent> = chart
        .objects
        .iter()
        .flat_map(|o| {
            let press = InputEvent {
                lane: o.lane,
                phase: Phase::Press,
                time_ms: f64::from(o.start_ms),
            };
            let release = o.end_ms.map(|end_ms| InputEvent {
                lane: o.lane,
                phase: Phase::Release,
                time_ms: f64::from(end_ms),
            });
            [Some(press), release].into_iter().flatten()
        })
        .collect();
    inputs.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    inputs
}

fn percentile(sorted_us: &[f64], p: f64) -> f64 {
    sorted_us[((sorted_us.len() - 1) as f64 * p) as usize]
}

fn play(count: u32) {
    let chart = chart(count);
    let inputs = inputs(&chart);
    let mut session = Session::new(&chart, Ruleset { judge: Judge::C }).unwrap();
    let mut next_input = 0;
    let mut frame_us = Vec::new();
    let mut render_ms = 0.0;
    while render_ms < f64::from(chart.duration_ms) + 1000.0 {
        while next_input < inputs.len() && inputs[next_input].time_ms <= render_ms {
            session.input(inputs[next_input]);
            next_input += 1;
        }
        let start = Instant::now();
        let events = session.advance(render_ms);
        let visible = session.visible_objects(render_ms - 700.0, render_ms + 1000.0);
        let lanes = session.lane_states();
        let score = session.score();
        let resolved = session.is_resolved();
        frame_us.push(start.elapsed().as_secs_f64() * 1e6);
        std::hint::black_box((events, visible, lanes, score, resolved));
        render_ms += FRAME_MS;
    }
    assert!(session.is_resolved());
    assert_eq!(session.score().tier_counts.judged(), count);
    frame_us.sort_by(f64::total_cmp);
    println!(
        "{count:>6} objects, {} frames: p50 {:.1} us, p99 {:.1} us, max {:.1} us",
        frame_us.len(),
        percentile(&frame_us, 0.5),
        percentile(&frame_us, 0.99),
        frame_us[frame_us.len() - 1],
    );
}

#[test]
#[ignore = "measures time; run alone in release mode"]
fn frame_cost() {
    for count in [4_000, 16_000] {
        play(count);
    }
}
