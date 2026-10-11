//! For a given chart, ruleset and input log, events and score do not depend on the advance cadence, nor,
//! for a streamed chart, on when its pages arrive. Also the per-frame queries, which are indexed, against
//! a linear reference.

mod common;

use common::{Delivery, Page, Step, complete, run};
use ensomi_gameplay::{
    Chart, ChartObject, ChartToken, InputEvent, Judge, Phase, Ruleset, Session, TokenKind,
    VisibleObject, VisibleState,
};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

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
                time_ms,
            }
        });
    prop::collection::vec(input, 0..40).prop_map(|mut inputs| {
        inputs.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
        inputs
    })
}

/// The inputs with advances at `watermarks` and `pages` (each with the chart time it arrives at) placed
/// before the first input stamped later than each, then a flush.
fn schedule(inputs: &[InputEvent], watermarks: &[f64], pages: Vec<(f64, Page)>) -> Vec<Step> {
    let mut timed: Vec<(f64, u8, Step)> = inputs
        .iter()
        .map(|input| (input.time_ms, 0, Step::Input(*input)))
        .collect();
    timed.extend(watermarks.iter().map(|&w| (w, 1, Step::Advance(w))));
    timed.extend(
        pages
            .into_iter()
            .map(|(at, page)| (at, 1, Step::Page(page))),
    );
    timed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut steps: Vec<Step> = timed.into_iter().map(|(.., step)| step).collect();
    steps.push(Step::Advance(f64::INFINITY));
    steps
}

fn with_advances(inputs: &[InputEvent], watermarks: &[f64]) -> Vec<Step> {
    schedule(inputs, watermarks, Vec::new())
}

/// The chart as tokens: hold ends apart from their starts, in time order, and in a lane a hold's start
/// before its end before the next note's start.
fn tokens(chart: &Chart) -> Vec<ChartToken> {
    let mut lane_count = [0u32; 4];
    let mut keyed = Vec::new();
    for object in &chart.objects {
        let index = lane_count[object.lane as usize];
        lane_count[object.lane as usize] += 1;
        let start_kind = if object.end_ms.is_some() {
            TokenKind::HoldStart
        } else {
            TokenKind::Tap
        };
        keyed.push(((object.start_ms, object.lane, 2 * index), start_kind));
        if let Some(end_ms) = object.end_ms {
            keyed.push(((end_ms, object.lane, 2 * index + 1), TokenKind::HoldEnd));
        }
    }
    keyed.sort_by_key(|&(key, _)| key);
    keyed
        .into_iter()
        .map(|((time_ms, lane, _), kind)| ChartToken {
            lane,
            time_ms: f64::from(time_ms),
            kind,
        })
        .collect()
}

/// The chart cut into pages at the `cuts` coverages, the last one ending the stream. Each page arrives at
/// its cut's arrival time, but not before the previous page.
fn pages(chart: &Chart, as_tokens: bool, cuts: &[(f64, f64)], end_at: f64) -> Vec<(f64, Page)> {
    let mut cuts = cuts.to_vec();
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    cuts.push((f64::INFINITY, end_at));
    let all_tokens = tokens(chart);
    let mut pages = Vec::new();
    let (mut after, mut arrival) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (coverage_ms, at) in cuts {
        let inside = |time_ms: f64| after < time_ms && time_ms <= coverage_ms;
        let delivery = if as_tokens {
            Delivery::Tokens(
                all_tokens
                    .iter()
                    .filter(|token| inside(token.time_ms))
                    .copied()
                    .collect(),
            )
        } else {
            Delivery::Objects(
                chart
                    .objects
                    .iter()
                    .filter(|object| inside(f64::from(object.start_ms)))
                    .copied()
                    .collect(),
            )
        };
        arrival = arrival.max(at);
        pages.push((
            arrival,
            Page {
                delivery,
                coverage_ms,
                end_of_stream: coverage_ms == f64::INFINITY,
            },
        ));
        after = coverage_ms;
    }
    pages
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

/// Page cuts: a coverage, often exactly a note's start or end, and an arrival time from well before it
/// (the stream ahead of play) to well after it (behind play). Also the arrival of the last page.
fn cuts(chart: &Chart) -> impl Strategy<Value = (Vec<(f64, f64)>, f64)> + use<> {
    let times: Vec<f64> = chart
        .objects
        .iter()
        .flat_map(|o| [o.start_ms, o.end_ms.unwrap_or(o.start_ms)].map(f64::from))
        .collect();
    let cut = (
        any::<prop::sample::Index>(),
        any::<bool>(),
        0i32..8000,
        -800i32..1600,
    )
        .prop_map(move |(at, on_note, half_ms, lag_half_ms)| {
            let coverage_ms = if on_note && !times.is_empty() {
                times[at.index(times.len())]
            } else {
                f64::from(half_ms) / 2.0
            };
            (coverage_ms, coverage_ms + f64::from(lag_half_ms) / 2.0)
        });
    (
        prop::collection::vec(cut, 0..8),
        (0i32..8000).prop_map(|half_ms| f64::from(half_ms) / 2.0),
    )
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
        let reference = run(complete(&chart, ruleset), &with_advances(&inputs, &[]));
        let cadenced = run(complete(&chart, ruleset), &with_advances(&inputs, &watermarks));
        prop_assert_eq!(&cadenced.events, &reference.events);
        prop_assert_eq!(&cadenced.score, &reference.score);
        prop_assert_eq!(cadenced.late_inputs, 0);

        // Every object gets exactly one outcome, and outcomes come in decision-time order.
        prop_assert!(reference.resolved);
        prop_assert_eq!(reference.score.tier_counts.judged() as usize, chart.objects.len());
        prop_assert!(reference.events.windows(2).all(|pair| pair[0].time_ms <= pair[1].time_ms));
    }
}

type StreamCase = (
    Chart,
    Judge,
    Vec<InputEvent>,
    Vec<f64>,
    bool,
    (Vec<(f64, f64)>, f64),
);

fn stream_case() -> impl Strategy<Value = StreamCase> {
    case().prop_flat_map(|(chart, judge, inputs, watermarks)| {
        let cuts = cuts(&chart);
        (
            Just(chart),
            Just(judge),
            Just(inputs),
            Just(watermarks),
            any::<bool>(),
            cuts,
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(5000))]

    /// A streamed chart, as objects or tokens, under any page and advance schedule, is judged as the
    /// complete chart; `run` also checks that nothing is decided beyond the coverage.
    #[test]
    fn streamed_sessions_judge_as_the_complete_chart(
        (chart, judge, inputs, watermarks, as_tokens, (cuts, end_at)) in stream_case()
    ) {
        let ruleset = Ruleset { judge };
        let reference = run(complete(&chart, ruleset), &with_advances(&inputs, &[]));
        for watermarks in [&[][..], &watermarks[..]] {
            let steps = schedule(&inputs, watermarks, pages(&chart, as_tokens, &cuts, end_at));
            let streamed = run(Session::streaming(ruleset), &steps);
            prop_assert_eq!(&streamed.events, &reference.events);
            prop_assert_eq!(&streamed.score, &reference.score);
            prop_assert_eq!(streamed.late_inputs, 0);
            prop_assert!(streamed.resolved);
        }
    }
}

/// `Session::visible_objects`, as it was before it was indexed: the filter over every object. Given every
/// object (`visible_objects(-inf, inf)`) and the session clock.
fn reference_visible(
    all: &[VisibleObject],
    now: f64,
    lower_ms: f64,
    upper_ms: f64,
) -> Vec<VisibleObject> {
    let resolved = |o: &VisibleObject| {
        matches!(
            o.state,
            VisibleState::Resolved | VisibleState::MissedButVisible
        )
    };
    all.iter()
        .copied()
        .filter(|o| {
            // Only an unresolved hold without an end is drawn up to the range.
            let open = o.end_time_ms.is_none()
                && matches!(o.state, VisibleState::OpenEnded | VisibleState::Holding);
            let end = o
                .end_time_ms
                .unwrap_or(if open { upper_ms } else { o.start_time_ms });
            let visible = o.start_time_ms <= upper_ms && end >= lower_ms;
            let keep_resolved = resolved(o) && now - end <= 300.0 && end >= lower_ms;
            visible || keep_resolved
        })
        .collect()
}

/// Compares the frame queries of `session` with the reference. `pressed` is the key state to expect, when
/// the inputs are handled as they arrive.
fn check_queries(
    session: &Session,
    now: f64,
    windows: &[(f64, f64)],
    events: usize,
    pressed: Option<[bool; 4]>,
) -> Result<(), TestCaseError> {
    let all = session.visible_objects(f64::NEG_INFINITY, f64::INFINITY);
    prop_assert!(all.iter().enumerate().all(|(i, o)| o.id as usize == i));
    let resolved = |o: &VisibleObject| {
        matches!(
            o.state,
            VisibleState::Resolved | VisibleState::MissedButVisible
        )
    };
    prop_assert_eq!(all.iter().filter(|o| resolved(o)).count(), events);
    prop_assert_eq!(
        session.is_resolved(),
        session.coverage_ms() == f64::INFINITY && all.iter().all(resolved)
    );

    for (lane, state) in session.lane_states().iter().enumerate() {
        prop_assert_eq!(state.lane as usize, lane);
        let holding = all
            .iter()
            .find(|o| o.lane as usize == lane && o.state == VisibleState::Holding);
        prop_assert_eq!(state.holding_object, holding.map(|o| o.id));
        if let Some(pressed) = pressed {
            prop_assert_eq!(state.is_pressed, pressed[lane]);
        }
    }

    for &(lower_ms, upper_ms) in windows {
        prop_assert_eq!(
            session.visible_objects(lower_ms, upper_ms),
            reference_visible(&all, now, lower_ms, upper_ms),
            "window {} to {}",
            lower_ms,
            upper_ms
        );
    }
    Ok(())
}

/// Windows as (lower, upper) in milliseconds: some absolute, some around the clock, some reversed.
fn query_windows() -> impl Strategy<Value = Vec<(i32, i32)>> {
    prop::collection::vec((-4000i32..4000, 0i32..4000), 1..5)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// At every step of a complete or streamed session, `visible_objects`, `lane_states` and
    /// `is_resolved` equal their linear definitions.
    #[test]
    fn frame_queries_match_a_linear_reference(
        ((chart, judge, inputs, watermarks, as_tokens, (cuts, end_at)), windows) in
            (stream_case(), query_windows())
    ) {
        let ruleset = Ruleset { judge };
        let streamed = schedule(&inputs, &watermarks, pages(&chart, as_tokens, &cuts, end_at));
        for (mut session, steps, is_complete) in [
            (complete(&chart, ruleset), with_advances(&inputs, &watermarks), true),
            (Session::streaming(ruleset), streamed, false),
        ] {
            let (mut now, mut events, mut pressed) = (f64::NEG_INFINITY, 0, [false; 4]);
            for step in std::iter::once(None).chain(steps.iter().map(Some)) {
                match step {
                    None => {}
                    Some(Step::Input(input)) => {
                        events += session.input(*input).len();
                        now = now.max(input.time_ms);
                        pressed[input.lane as usize] = input.phase == Phase::Press;
                    }
                    Some(Step::Advance(watermark_ms)) => {
                        events += session.advance(*watermark_ms).len();
                        now = now.max(*watermark_ms);
                    }
                    Some(Step::Page(page)) => {
                        events += match &page.delivery {
                            Delivery::Objects(objects) => {
                                session.append(objects, page.coverage_ms, page.end_of_stream)
                            }
                            Delivery::Tokens(tokens) => {
                                session.append_tokens(tokens, page.coverage_ms, page.end_of_stream)
                            }
                        }
                        .expect("valid page")
                        .len();
                    }
                    Some(Step::DecidedThrough) => {}
                }
                let around = if now.is_finite() { now } else { 0.0 };
                let windows: Vec<(f64, f64)> = windows
                    .iter()
                    .flat_map(|&(lower, length)| {
                        let (lower, length) = (f64::from(lower) / 2.0, f64::from(length) / 2.0);
                        [
                            (lower, lower + length),
                            (around + lower, around + lower + length),
                            (lower + length, lower),
                        ]
                    })
                    .collect();
                check_queries(&session, now, &windows, events, is_complete.then_some(pressed))?;
            }
        }
    }
}
