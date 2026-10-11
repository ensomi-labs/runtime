//! For a given chart, ruleset and input log, events and score do not depend on the advance cadence.

mod common;

use common::{Step, run};
use ensomi_gameplay::{Chart, ChartObject, InputEvent, Judge, Phase, Ruleset};
use proptest::prelude::*;

/// Gaps and hold lengths that are often multiples of 100 ms, so that different lanes' misses tie.
fn gap() -> impl Strategy<Value = u32> {
    prop_oneof![0u32..500, (0u32..8).prop_map(|k| k * 100)]
}

fn hold() -> impl Strategy<Value = u32> {
    prop_oneof![Just(0), 1u32..700, (1u32..8).prop_map(|k| k * 100)]
}

fn chart() -> impl Strategy<Value = Chart> {
    // Per lane, notes as (gap after the previous one, hold length); a hold length of 0 is a tap.
    prop::collection::vec(prop::collection::vec((gap(), hold()), 0..8), 4).prop_map(|lanes| {
        let mut objects = Vec::new();
        for (lane, notes) in lanes.iter().enumerate() {
            let mut earliest = 0;
            for &(gap, hold) in notes {
                let start_ms = earliest + gap;
                let end_ms = (hold > 0).then_some(start_ms + hold);
                earliest = end_ms.unwrap_or(start_ms + 1);
                objects.push(ChartObject {
                    lane: lane as u32,
                    start_ms,
                    end_ms,
                });
            }
        }
        objects.sort_by_key(|o| (o.start_ms, o.lane));
        let duration_ms = objects
            .iter()
            .map(|o| o.end_ms.unwrap_or(o.start_ms))
            .max()
            .unwrap_or(0);
        Chart {
            duration_ms,
            objects,
        }
    })
}

/// Inputs near note times, so that window edges, locks and tail windows are exercised, sorted by time.
fn inputs(chart: &Chart) -> impl Strategy<Value = Vec<InputEvent>> + use<> {
    let anchors: Vec<(u32, u32)> = chart
        .objects
        .iter()
        .flat_map(|o| {
            [
                (o.lane, o.start_ms),
                (o.lane, o.end_ms.unwrap_or(o.start_ms)),
            ]
        })
        .collect();
    let input = (
        any::<prop::sample::Index>(),
        -450i32..450,
        0u32..4,
        any::<bool>(),
        0.0f64..3000.0,
    )
        .prop_map(move |(anchor, half_ms, any_lane, press, free_ms)| {
            let (lane, time_ms) = if anchors.is_empty() {
                (any_lane, free_ms)
            } else {
                let (lane, at) = anchors[anchor.index(anchors.len())];
                (
                    if half_ms % 3 == 0 { any_lane } else { lane },
                    f64::from(at) + f64::from(half_ms) / 2.0,
                )
            };
            InputEvent {
                lane,
                phase: if press { Phase::Press } else { Phase::Release },
                time_ms: time_ms.max(0.0),
            }
        });
    prop::collection::vec(input, 0..40).prop_map(|mut inputs| {
        inputs.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
        inputs
    })
}

/// The inputs with advances at `watermarks` placed before the first input stamped later than each.
fn with_advances(inputs: &[InputEvent], watermarks: &[f64]) -> Vec<Step> {
    let mut watermarks = watermarks.to_vec();
    watermarks.sort_by(f64::total_cmp);
    let mut pending = watermarks.into_iter().peekable();
    let mut steps = Vec::new();
    for input in inputs {
        while let Some(watermark_ms) = pending.next_if(|&w| w < input.time_ms) {
            steps.push(Step::Advance(watermark_ms));
        }
        steps.push(Step::Input(*input));
    }
    steps.extend(pending.map(Step::Advance));
    steps.push(Step::Advance(f64::INFINITY));
    steps
}

/// Advance times, many of them exactly where a miss falls due, since that is where a cadence could matter.
fn watermarks(chart: &Chart) -> impl Strategy<Value = Vec<f64>> + use<> {
    let due: Vec<f64> = chart
        .objects
        .iter()
        .flat_map(|o| {
            let (start, end) = (
                f64::from(o.start_ms),
                f64::from(o.end_ms.unwrap_or(o.start_ms)),
            );
            [start, start + 200.0, end, end + 300.0]
        })
        .collect();
    let watermark = (any::<prop::sample::Index>(), any::<bool>(), 0i32..8000).prop_map(
        move |(due_at, at_due, half_ms)| {
            if at_due && !due.is_empty() {
                due[due_at.index(due.len())]
            } else {
                f64::from(half_ms) / 2.0
            }
        },
    );
    prop::collection::vec(watermark, 0..60)
}

fn case() -> impl Strategy<Value = (Chart, Judge, Vec<InputEvent>, Vec<f64>)> {
    (
        chart(),
        prop::sample::select(vec![Judge::A, Judge::B, Judge::C, Judge::D, Judge::E]),
    )
        .prop_flat_map(|(chart, judge)| {
            let (inputs, watermarks) = (inputs(&chart), watermarks(&chart));
            (Just(chart), Just(judge), inputs, watermarks)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(5000))]

    #[test]
    fn advance_cadence_does_not_change_the_outcome((chart, judge, inputs, watermarks) in case()) {
        let ruleset = Ruleset { judge };
        let reference = run(&chart, ruleset, &with_advances(&inputs, &[]));
        let cadenced = run(&chart, ruleset, &with_advances(&inputs, &watermarks));
        prop_assert_eq!(&cadenced.events, &reference.events);
        prop_assert_eq!(&cadenced.score, &reference.score);
        prop_assert_eq!(cadenced.late_inputs, 0);

        // Every object gets exactly one outcome, and outcomes come in decision-time order.
        prop_assert!(reference.resolved);
        prop_assert_eq!(reference.score.tier_counts.judged() as usize, chart.objects.len());
        prop_assert!(reference.events.windows(2).all(|pair| pair[0].time_ms <= pair[1].time_ms));
    }
}
