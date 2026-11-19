//! Streamed charts: page validation, open-ended holds and how far a session has judged.

use ensomi_gameplay::*;

fn tap(lane: u32, start_ms: u32) -> ChartObject {
    ChartObject {
        lane,
        start_ms,
        end_ms: None,
    }
}

fn token(lane: u32, time_ms: f64, kind: TokenKind) -> ChartToken {
    ChartToken {
        lane,
        time_ms,
        kind,
    }
}

fn press(lane: u32, time_ms: f64) -> InputEvent {
    InputEvent {
        lane,
        phase: Phase::Press,
        time_ms,
    }
}

fn stream() -> Session {
    Session::streaming(Ruleset::new(Judge::C))
}

#[test]
fn pages_are_checked_against_the_coverage_and_the_lane_sequence() {
    let mut session = stream();
    session.append(&[tap(0, 1_000)], 1_200.0, false).unwrap();
    assert_eq!(
        session.append(&[tap(1, 1_200)], 2_000.0, false),
        Err(ChartError::BeforeCoverage { object: 1 })
    );

    let mut session = stream();
    session
        .append_tokens(&[token(0, 1_000.0, TokenKind::HoldStart)], 1_500.0, false)
        .unwrap();
    assert_eq!(
        session.append_tokens(&[token(0, 1_400.0, TokenKind::HoldEnd)], 2_000.0, false),
        Err(ChartError::BeforeCoverage { object: 0 })
    );
    assert_eq!(
        session.append_tokens(&[token(0, 1_600.0, TokenKind::Tap)], 2_000.0, false),
        Err(ChartError::SameLaneOverlap { object: 1 })
    );
    assert_eq!(
        session.append_tokens(&[token(1, 1_600.0, TokenKind::HoldEnd)], 2_000.0, false),
        Err(ChartError::HoldEndWithoutStart { lane: 1 })
    );
    assert_eq!(
        session.append_tokens(&[], 2_000.0, true),
        Err(ChartError::UnclosedHold { object: 0 })
    );
    assert_eq!(
        session.append_tokens(&[token(1, f64::NAN, TokenKind::Tap)], 2_000.0, false),
        Err(ChartError::InvalidTime { object: 1 })
    );
}

#[test]
fn nothing_arrives_after_the_end_of_the_stream() {
    let mut session = stream();
    session.append(&[], 1_000.0, true).unwrap();
    assert_eq!(
        session.append(&[tap(0, 5_000)], 6_000.0, false),
        Err(ChartError::BeforeCoverage { object: 0 })
    );
}

#[test]
fn a_hold_without_its_end_is_drawn_open_ended_until_pressed() {
    let mut session = stream();
    session
        .append_tokens(&[token(0, 1_000.0, TokenKind::HoldStart)], 1_100.0, false)
        .unwrap();
    let visible = session.visible_objects(500.0, 1_500.0);
    assert_eq!(
        (visible[0].state, visible[0].end_time_ms),
        (VisibleState::OpenEnded, None)
    );

    session.input(press(0, 1_000.0));
    assert_eq!(
        session.visible_objects(500.0, 1_500.0)[0].state,
        VisibleState::Holding
    );
    assert_eq!(session.lane_states()[0].holding_object, Some(0));

    session
        .append_tokens(&[token(0, 1_800.0, TokenKind::HoldEnd)], 2_000.0, false)
        .unwrap();
    assert_eq!(
        session.visible_objects(500.0, 1_500.0)[0].end_time_ms,
        Some(1_800.0)
    );
}

#[test]
fn a_stream_is_resolved_only_after_its_end() {
    let mut session = stream();
    session.append(&[tap(0, 1_000)], 3_000.0, false).unwrap();
    assert_eq!(session.advance(3_000.0).len(), 1);
    assert!(!session.is_resolved());
    assert_eq!(session.coverage_ms(), 3_000.0);

    session.append(&[], 3_000.0, true).unwrap();
    assert!(session.is_resolved());
    assert_eq!(session.coverage_ms(), f64::INFINITY);
    assert_eq!(session.decided_through_ms(), 3_000.0);
}

#[test]
fn a_complete_chart_is_judged_up_to_the_clock() {
    let chart = Chart {
        duration_ms: 2_000,
        objects: vec![tap(0, 1_000)],
    };
    let mut session = Session::new(&chart, Ruleset::new(Judge::C)).unwrap();
    assert_eq!(session.decided_through_ms(), f64::NEG_INFINITY);
    session.advance(1_500.0);
    assert_eq!(session.decided_through_ms(), 1_500.0);
}

#[test]
fn open_ended_hold_without_tail_judgement_settles_release_without_waiting_for_end() {
    let ruleset = Ruleset {
        judge: Judge::C,
        tail_judgement: false,
        hold_ticks: true,
    };
    assert_eq!(ruleset.settle_lead_ms(), 200.0);
    let mut session = Session::streaming(ruleset);
    session
        .append_tokens(&[token(0, 1_000.0, TokenKind::HoldStart)], 1_300.0, false)
        .unwrap();
    let hit = session.input(press(0, 1_000.0));
    assert_eq!(hit.len(), 1);
    assert_eq!((hit[0].part, hit[0].tier), (Part::Head, Tier::BigP));

    let released = session.input(InputEvent {
        lane: 0,
        phase: Phase::Release,
        time_ms: 1_250.0,
    });
    assert!(released.is_empty());
    assert_eq!(session.decided_through_ms(), 1_250.0);
    // Head +1, plus 1100 and 1200 ticks = 3 combo, accuracy 1.0, 1 judged object.
    let score = session.score();
    assert_eq!((score.combo, score.max_combo, score.accuracy, score.tier_counts.judged()), (3, 3, 1.0, 1));
}
