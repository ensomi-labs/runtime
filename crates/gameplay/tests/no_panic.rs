//! No input a session or feedback state accepts makes it panic: charts complete or streamed in pages, arbitrary
//! pages, inputs, watermarks and queries, with times anywhere in f64, NaN and the infinities included. The Swift
//! bindings call the per-frame methods without `throws` on the strength of this (`crates/ffi`).

use ensomi_gameplay::{
    Chart, ChartObject, ChartToken, GameplayFeedback, InputEvent, Judge, JudgementEvent, Phase,
    Ruleset, Session, TokenKind, VisibleObject, VisibleState, geometry, y_position,
};
use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;

fn any_time() -> BoxedStrategy<f64> {
    prop_oneof![
        -500.0f64..3000.0,
        any::<f64>(),
        Just(f64::NAN),
        Just(f64::INFINITY),
        Just(f64::NEG_INFINITY),
        Just(-0.0),
    ]
    .boxed()
}

fn lane() -> impl Strategy<Value = u32> + Clone {
    prop_oneof![0u32..4, any::<u32>()]
}

fn phase() -> impl Strategy<Value = Phase> + Clone {
    prop_oneof![Just(Phase::Press), Just(Phase::Release)]
}

fn judge() -> impl Strategy<Value = Judge> {
    prop_oneof![Just(Judge::A), Just(Judge::B), Just(Judge::C), Just(Judge::D), Just(Judge::E)]
}

/// A chart in order, with gaps and holds that the windows and locks make interesting.
fn chart_objects() -> impl Strategy<Value = Vec<ChartObject>> {
    prop::collection::vec((0u32..4, 0u32..300, prop::option::of(1u32..600)), 0..16).prop_map(|notes| {
        let mut earliest = [0u32; 4];
        let mut objects: Vec<ChartObject> = notes
            .into_iter()
            .map(|(lane, gap, hold)| {
                let start_ms = earliest[lane as usize] + gap;
                let end_ms = hold.map(|h| start_ms + h);
                earliest[lane as usize] = end_ms.unwrap_or(start_ms) + 1;
                ChartObject { lane, start_ms, end_ms }
            })
            .collect();
        objects.sort_by_key(|o| (o.start_ms, o.lane));
        objects
    })
}

fn junk_objects() -> impl Strategy<Value = Vec<ChartObject>> {
    let ms = prop_oneof![0u32..3000, any::<u32>()];
    prop::collection::vec((lane(), ms.clone(), prop::option::of(ms)), 0..6)
        .prop_map(|o| o.into_iter().map(|(lane, start_ms, end_ms)| ChartObject { lane, start_ms, end_ms }).collect())
}

fn junk_tokens(time: BoxedStrategy<f64>) -> impl Strategy<Value = Vec<ChartToken>> {
    let kind = prop_oneof![Just(TokenKind::Tap), Just(TokenKind::HoldStart), Just(TokenKind::HoldEnd)];
    prop::collection::vec((lane(), time, kind), 0..6)
        .prop_map(|t| t.into_iter().map(|(lane, time_ms, kind)| ChartToken { lane, time_ms, kind }).collect())
}

#[derive(Clone, Debug)]
enum Op {
    Input(InputEvent),
    Advance(f64),
    /// The next objects of the chart, with the coverage the following one leaves.
    Page(usize),
    Objects(Vec<ChartObject>, f64, bool),
    Tokens(Vec<ChartToken>, f64, bool),
    Visible(f64, f64),
    Feedback(f64),
    Key(u32, Phase, u64, f64),
    Look(u32, f64),
}

/// Operations whose times are mostly near the chart's note starts and ends, in the note's lane, so that hits,
/// holds, tail windows and locks are reached.
fn ops(objects: &[ChartObject]) -> BoxedStrategy<Vec<Op>> {
    let anchors: Vec<(u32, f64)> = objects
        .iter()
        .flat_map(|o| [(o.lane, o.start_ms), (o.lane, o.end_ms.unwrap_or(o.start_ms))])
        .map(|(lane, ms)| (lane, f64::from(ms)))
        .collect();
    let near = if anchors.is_empty() {
        any_time()
    } else {
        let anchors = anchors.clone();
        (any::<prop::sample::Index>(), -350.0f64..350.0).prop_map(move |(i, offset)| anchors[i.index(anchors.len())].1 + offset).boxed()
    };
    let time = prop_oneof![3 => near, 1 => any_time()].boxed();
    let input = if anchors.is_empty() {
        (lane(), phase(), time.clone()).prop_map(|(lane, phase, time_ms)| InputEvent { lane, phase, time_ms }).boxed()
    } else {
        (any::<prop::sample::Index>(), -350.0f64..350.0, phase(), prop::option::weighted(0.1, lane()))
            .prop_map(move |(i, offset, phase, other)| {
                let (lane, at) = anchors[i.index(anchors.len())];
                InputEvent { lane: other.unwrap_or(lane), phase, time_ms: at + offset }
            })
            .boxed()
    };
    let op = prop_oneof![
        6 => input.prop_map(Op::Input),
        3 => time.clone().prop_map(Op::Advance),
        2 => (1usize..5).prop_map(Op::Page),
        1 => (junk_objects(), time.clone(), any::<bool>()).prop_map(|(o, c, e)| Op::Objects(o, c, e)),
        1 => (junk_tokens(time.clone()), time.clone(), any::<bool>()).prop_map(|(t, c, e)| Op::Tokens(t, c, e)),
        2 => (time.clone(), time.clone()).prop_map(|(a, b)| Op::Visible(a, b)),
        1 => time.clone().prop_map(Op::Feedback),
        2 => (lane(), phase(), prop_oneof![0u64..20, any::<u64>()], time.clone()).prop_map(|(l, p, s, t)| Op::Key(l, p, s, t)),
        1 => (lane(), time).prop_map(|(l, t)| Op::Look(l, t)),
    ];
    prop::collection::vec(op, 0..80).boxed()
}

fn record(feedback: &mut GameplayFeedback, events: Vec<JudgementEvent>, at: f64) {
    feedback.record_judgement_events(&events, at);
    feedback.judgement_presentation(at);
}

fn page(session: &mut Session, feedback: &mut GameplayFeedback, objects: &[ChartObject], coverage: f64, end: bool) {
    if let Ok(events) = session.append(objects, coverage, end) {
        record(feedback, events, coverage);
    }
}

/// Draws every object a range shows, as a playfield does each frame.
fn draw(objects: &[VisibleObject], at: f64) {
    for object in objects {
        geometry(object, at, 1500.0, 600.0);
        y_position(object.start_time_ms, at, 1500.0, 600.0);
    }
}

/// Runs the operations; `chart` is what `Op::Page` delivers, from `delivered` on.
fn run(mut session: Session, chart: &[ChartObject], mut delivered: usize, ops: &[Op]) {
    let mut feedback = GameplayFeedback::new();
    for op in ops {
        match op {
            Op::Input(input) => record(&mut feedback, session.input(*input), input.time_ms),
            Op::Advance(watermark) => record(&mut feedback, session.advance(*watermark), *watermark),
            Op::Page(count) => {
                let to = (delivered + count).min(chart.len());
                let coverage = chart.get(to).map_or(f64::INFINITY, |next| f64::from(next.start_ms) - 1.0);
                page(&mut session, &mut feedback, &chart[delivered..to], coverage, to == chart.len());
                delivered = to;
            }
            Op::Objects(objects, coverage, end) => page(&mut session, &mut feedback, objects, *coverage, *end),
            Op::Tokens(tokens, coverage, end) => {
                if let Ok(events) = session.append_tokens(tokens, *coverage, *end) {
                    record(&mut feedback, events, *coverage);
                }
            }
            Op::Visible(lower, upper) => draw(&session.visible_objects(*lower, *upper), *lower),
            Op::Feedback(at) => {
                feedback.advance_judgement_time(*at);
                feedback.judgement_presentation(*at);
                feedback.judgement_event_batches();
            }
            Op::Key(lane, phase, sequence, at) => {
                feedback.record_input(*lane, *phase, *sequence, *at);
                feedback.lane_brightness(*lane, *at);
            }
            Op::Look(lane, at) => {
                feedback.lane_brightness(*lane, *at);
                feedback.latest_lane_input_transition(*lane);
            }
        }
        session.score();
        session.lane_states();
        session.coverage_ms();
        session.decided_through_ms();
        session.is_resolved();
        session.late_inputs();
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    #[test]
    fn a_complete_chart_never_panics((objects, ops) in chart_objects().prop_flat_map(|o| { let ops = ops(&o); (Just(o), ops) }), judge in judge()) {
        let duration_ms = objects.iter().map(|o| o.end_ms.unwrap_or(o.start_ms)).max().unwrap_or(0);
        let session = Session::new(&Chart { duration_ms, objects: objects.clone() }, Ruleset { judge }).unwrap();
        run(session, &objects, objects.len(), &ops);
    }

    #[test]
    fn a_stream_never_panics((objects, ops) in chart_objects().prop_flat_map(|o| { let ops = ops(&o); (Just(o), ops) }), judge in judge()) {
        run(Session::streaming(Ruleset { judge }), &objects, 0, &ops);
    }

    #[test]
    fn an_arbitrary_chart_never_panics(objects in junk_objects(), duration_ms in any::<u32>(), judge in judge(), ops in ops(&[])) {
        if let Ok(session) = Session::new(&Chart { duration_ms, objects }, Ruleset { judge }) {
            run(session, &[], 0, &ops);
        }
    }

    #[test]
    fn layout_never_panics(id in any::<u32>(), lane in lane(), start in any_time(), end in prop::option::of(any_time()), state in 0usize..5, at in any_time(), scroll in any_time(), receptor in any_time()) {
        let state = [VisibleState::Waiting, VisibleState::Holding, VisibleState::OpenEnded, VisibleState::MissedButVisible, VisibleState::Resolved][state];
        geometry(&VisibleObject { id, lane, start_time_ms: start, end_time_ms: end, state }, at, scroll, receptor);
        y_position(start, at, scroll, receptor);
    }
}
