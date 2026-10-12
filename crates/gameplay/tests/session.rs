use ensomi_gameplay::*;

fn tap(lane: u32, start_ms: u32) -> ChartObject {
    ChartObject {
        lane,
        start_ms,
        end_ms: None,
    }
}

fn hold(lane: u32, start_ms: u32, end_ms: u32) -> ChartObject {
    ChartObject {
        lane,
        start_ms,
        end_ms: Some(end_ms),
    }
}

fn chart(objects: Vec<ChartObject>) -> Chart {
    Chart {
        duration_ms: 10_000,
        objects,
    }
}

fn session(objects: Vec<ChartObject>) -> Session {
    Session::new(&chart(objects), Ruleset { judge: Judge::C }).unwrap()
}

fn press(lane: u32, time_ms: f64) -> InputEvent {
    InputEvent {
        lane,
        phase: Phase::Press,
        time_ms,
    }
}

fn release(lane: u32, time_ms: f64) -> InputEvent {
    InputEvent {
        lane,
        phase: Phase::Release,
        time_ms,
    }
}

fn error_of(objects: Vec<ChartObject>) -> ChartError {
    chart(objects).validate().unwrap_err()
}

#[test]
fn chart_validation_reports_the_offending_object() {
    assert_eq!(
        error_of(vec![tap(4, 100)]),
        ChartError::InvalidLane { object: 0 }
    );
    assert_eq!(
        error_of(vec![tap(0, 100), hold(1, 200, 199)]),
        ChartError::InvalidHold { object: 1 }
    );
    assert_eq!(
        error_of(vec![hold(0, 200, 100)]),
        ChartError::InvalidHold { object: 0 }
    );
    assert_eq!(
        error_of(vec![tap(0, 10_001)]),
        ChartError::OutsideDuration { object: 0 }
    );
    assert_eq!(
        error_of(vec![hold(0, 9_000, 10_001)]),
        ChartError::OutsideDuration { object: 0 }
    );
    assert_eq!(
        error_of(vec![tap(0, 200), tap(1, 100)]),
        ChartError::NotSorted { object: 1 }
    );
    assert_eq!(
        error_of(vec![tap(1, 100), tap(0, 100)]),
        ChartError::NotSorted { object: 1 }
    );
    assert_eq!(
        error_of(vec![tap(0, 100), tap(0, 100)]),
        ChartError::SameLaneOverlap { object: 1 }
    );
    assert_eq!(
        error_of(vec![hold(0, 100, 300), tap(0, 200)]),
        ChartError::SameLaneOverlap { object: 1 }
    );
    assert_eq!(
        error_of(vec![hold(0, 100, 300), hold(0, 200, 400)]),
        ChartError::SameLaneOverlap { object: 1 }
    );
}

#[test]
fn chart_validation_accepts_a_note_starting_where_a_hold_ends() {
    assert_eq!(
        chart(vec![hold(0, 100, 300), tap(0, 300), tap(1, 300)]).validate(),
        Ok(())
    );
    assert!(chart(vec![]).validate().is_ok());
}

#[test]
fn a_late_input_is_handled_at_the_session_clock_and_counted() {
    let mut session = session(vec![tap(0, 1_000)]);
    assert!(session.advance(1_100.0).is_empty());

    let events = session.input(press(0, 1_050.0));

    assert_eq!(session.late_inputs(), 1);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].tier, Tier::P3);
    assert_eq!(events[0].offset_ms, Some(100.0));
    assert_eq!(events[0].time_ms, 1_100.0);
}

#[test]
fn chart_time_may_be_negative() {
    let mut session = session(vec![tap(0, 50)]);
    let events = session.input(press(0, -100.0));
    assert_eq!(
        (events[0].tier, events[0].offset_ms, events[0].time_ms),
        (Tier::G, Some(-150.0), -100.0)
    );
    assert_eq!(session.late_inputs(), 0);
}

#[test]
fn an_input_stamped_on_the_clock_is_not_late() {
    let mut session = session(vec![tap(0, 1_000)]);
    session.advance(1_100.0);
    session.input(press(0, 1_100.0));
    assert_eq!(session.late_inputs(), 0);
}

#[test]
fn an_input_after_a_missed_note_cannot_hit_it() {
    let mut session = session(vec![tap(0, 1_000)]);
    let missed = session.advance(1_300.0);
    assert_eq!(
        (missed.len(), missed[0].tier, missed[0].time_ms),
        (1, Tier::M, 1_200.0)
    );

    assert!(session.input(press(0, 1_100.0)).is_empty());
    assert_eq!(session.late_inputs(), 1);
}

#[test]
fn inputs_for_lanes_outside_the_chart_are_ignored() {
    let mut session = session(vec![tap(0, 1_000)]);
    assert!(session.input(press(4, 1_000.0)).is_empty());
    assert_eq!(session.score().tier_counts.judged(), 0);
}

#[test]
fn the_miss_at_the_end_of_a_window_waits_for_the_input_stamped_on_it() {
    let mut session = session(vec![tap(0, 1_000)]);
    assert!(session.advance(1_200.0).is_empty());
    let events = session.input(press(0, 1_200.0));
    assert_eq!(
        (events[0].tier, events[0].offset_ms),
        (Tier::G, Some(200.0))
    );
}

#[test]
fn a_note_lock_is_decided_at_the_start_of_the_next_note() {
    let mut session = session(vec![tap(0, 1_000), tap(0, 1_100), tap(1, 1_100)]);
    let events = session.advance(1_100.0);
    assert_eq!(events.len(), 1);
    assert_eq!(
        (
            events[0].object,
            events[0].tier,
            events[0].time_ms,
            events[0].part
        ),
        (0, Tier::M, 1_100.0, Part::Head)
    );
}

#[test]
fn simultaneous_misses_come_in_chart_order() {
    let mut session = session(vec![tap(2, 1_000), tap(3, 1_000), tap(0, 1_001)]);
    let events = session.advance(f64::INFINITY);
    assert_eq!(
        events.iter().map(|e| e.object).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(
        events.iter().map(|e| e.time_ms).collect::<Vec<_>>(),
        [1_200.0, 1_200.0, 1_201.0]
    );
}

#[test]
fn lane_states_follow_presses_and_holds() {
    let mut session = session(vec![hold(1, 1_000, 1_500)]);
    session.input(press(1, 1_000.0));
    session.input(press(3, 1_000.0));

    let states = session.lane_states();
    assert_eq!(states.len(), 4);
    assert_eq!(
        (states[1].is_pressed, states[1].holding_object),
        (true, Some(0))
    );
    assert_eq!(
        (states[3].is_pressed, states[3].holding_object),
        (true, None)
    );
    assert_eq!(
        (states[0].is_pressed, states[0].holding_object),
        (false, None)
    );

    session.input(release(1, 1_500.0));
    assert_eq!(
        session.lane_states()[1],
        LaneState {
            lane: 1,
            is_pressed: false,
            holding_object: None
        }
    );
}

#[test]
fn visible_objects_report_states() {
    let mut session = session(vec![tap(0, 1_000), hold(1, 1_000, 2_000), tap(2, 5_000)]);
    session.input(press(0, 1_000.0));
    session.input(press(1, 1_000.0));

    let visible = session.visible_objects(900.0, 3_000.0);
    assert_eq!(
        visible.iter().map(|o| (o.id, o.state)).collect::<Vec<_>>(),
        [(0, VisibleState::Resolved), (1, VisibleState::Holding)]
    );
    assert_eq!(visible[1].end_time_ms, Some(2_000.0));

    session.advance(3_000.0);
    assert_eq!(
        session.visible_objects(0.0, 10_000.0)[1].state,
        VisibleState::MissedButVisible
    );
}

#[test]
fn resolved_objects_ahead_of_the_range_stay_for_300_ms() {
    let mut session = session(vec![tap(0, 1_000)]);
    session.input(press(0, 1_000.0));

    session.advance(1_300.0);
    assert_eq!(session.visible_objects(0.0, 500.0).len(), 1);

    session.advance(1_301.0);
    assert!(session.visible_objects(0.0, 500.0).is_empty());
}

#[test]
fn a_session_without_objects_is_resolved_with_full_accuracy() {
    let session = session(vec![]);
    assert!(session.is_resolved());
    let score = session.score();
    assert_eq!(
        (
            score.accuracy,
            score.combo,
            score.max_combo,
            score.suggested_audio_offset_ms
        ),
        (1.0, 0, 0, None)
    );
}
