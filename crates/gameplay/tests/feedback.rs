//! The cases of the Swift `Mania4KGameplayFeedbackTests`.

use ensomi_gameplay::*;

fn event(id: u64, lane: u32, tier: Tier) -> JudgementEvent {
    JudgementEvent {
        id,
        object: id as u32,
        lane,
        part: Part::Head,
        tier,
        offset_ms: (tier != Tier::M).then_some(0.0),
        time_ms: 1_000.0,
        object_time_ms: 1_000.0,
    }
}

fn shown(feedback: &GameplayFeedback, chart_time_ms: f64) -> Option<(Judgement, u64)> {
    feedback
        .judgement_presentation(chart_time_ms)
        .map(|p| (p.event.tier.judgement(), p.event.id))
}

#[test]
fn batches_keep_complete_events_until_pruned() {
    let mut feedback = GameplayFeedback::new();
    feedback.record_judgement_events(&[], 1_000.0);
    assert!(feedback.judgement_event_batches().is_empty());

    let events = [
        event(1, 0, Tier::BigP),
        event(2, 1, Tier::M),
        event(3, 2, Tier::P1),
    ];
    feedback.record_judgement_events(&events, 1_000.0);
    assert_eq!(feedback.judgement_event_batches().len(), 1);
    assert_eq!(feedback.judgement_event_batches()[0].events, events);

    feedback.advance_judgement_time(1_000.0);
    assert_eq!(feedback.judgement_event_batches().len(), 1);
    feedback.advance_judgement_time(10_000.0);
    assert!(feedback.judgement_event_batches().is_empty());
}

#[test]
fn presentation_chooses_the_most_severe_event_then_the_newest_id() {
    let mut mixed = GameplayFeedback::new();
    mixed.record_judgement_events(
        &[
            event(1, 0, Tier::BigP),
            event(2, 1, Tier::M),
            event(3, 2, Tier::P1),
        ],
        1_000.0,
    );
    assert_eq!(shown(&mixed, 1_000.0), Some((Judgement::Miss, 2)));

    let mut tie = GameplayFeedback::new();
    tie.record_judgement_events(
        &[
            event(10, 0, Tier::P1),
            event(12, 3, Tier::P1),
            event(11, 2, Tier::P1),
        ],
        2_000.0,
    );
    assert_eq!(shown(&tie, 2_000.0), Some((Judgement::Good, 12)));
}

#[test]
fn a_miss_replaces_a_perfect_and_is_protected_from_a_lighter_judgement() {
    let mut protected = GameplayFeedback::new();
    protected.record_judgement_events(&[event(1, 0, Tier::BigP)], 1_000.0);
    protected.record_judgement_events(&[event(2, 0, Tier::M)], 1_000.0);
    assert_eq!(shown(&protected, 1_000.0), Some((Judgement::Miss, 2)));

    protected.record_judgement_events(&[event(3, 0, Tier::P1)], 1_000.0);
    assert_eq!(shown(&protected, 1_000.0), Some((Judgement::Miss, 2)));
    protected.advance_judgement_time(10_000.0);
    assert_eq!(shown(&protected, 10_000.0), None);

    let mut expired = GameplayFeedback::new();
    expired.record_judgement_events(&[event(4, 0, Tier::M)], 2_000.0);
    expired.record_judgement_events(&[event(5, 0, Tier::P1)], 10_000.0);
    assert_eq!(shown(&expired, 10_000.0), Some((Judgement::Good, 5)));
}

#[test]
fn the_same_judgement_retriggers_and_disappears() {
    let mut feedback = GameplayFeedback::new();
    feedback.record_judgement_events(&[event(20, 0, Tier::BigP)], 1_000.0);
    feedback.record_judgement_events(&[event(21, 0, Tier::BigP)], 1_000.0);

    let presentation = feedback.judgement_presentation(1_000.0).unwrap();
    assert_eq!(
        (presentation.event.tier, presentation.event.id),
        (Tier::BigP, 21)
    );
    assert!((presentation.opacity - 1.0).abs() < 1e-4);
    assert_eq!(feedback.judgement_presentation(1_000.0), Some(presentation));

    feedback.advance_judgement_time(10_000.0);
    assert_eq!(feedback.judgement_presentation(10_000.0), None);
}

#[test]
fn presentation_animates_by_age() {
    let mut feedback = GameplayFeedback::new();
    feedback.record_judgement_events(&[event(1, 0, Tier::M)], 1_000.0);

    let start = feedback.judgement_presentation(1_000.0).unwrap();
    assert_eq!(
        (start.opacity, start.scale, start.vertical_offset),
        (1.0, 1.08, 0.0)
    );
    let end = feedback.judgement_presentation(1_399.0).unwrap();
    assert!(end.opacity < 0.01 && end.vertical_offset > 5.9 && (end.scale - 0.98).abs() < 0.01);
    assert_eq!(feedback.judgement_presentation(1_400.0), None);
}

fn press(feedback: &mut GameplayFeedback, lane: u32, sequence_number: u64, ui_time_ms: f64) {
    feedback.record_input(lane, Phase::Press, sequence_number, ui_time_ms);
}

fn release(feedback: &mut GameplayFeedback, lane: u32, sequence_number: u64, ui_time_ms: f64) {
    feedback.record_input(lane, Phase::Release, sequence_number, ui_time_ms);
}

#[test]
fn lane_brightness_presses_holds_releases_and_returns_to_zero() {
    let mut feedback = GameplayFeedback::new();
    press(&mut feedback, 0, 30, 10_000.0);

    let pressed = feedback.lane_brightness(0, 10_000.0);
    assert!(pressed.receptor > 0.0 && pressed.lane > 0.0);
    let held = feedback.lane_brightness(0, 20_000.0);
    assert!(held.receptor > 0.0 && held.lane > 0.0);
    assert!(held.receptor < pressed.receptor && held.lane < pressed.lane);

    release(&mut feedback, 0, 31, 20_000.0);
    let start = feedback.lane_brightness(0, 20_000.0);
    assert!((start.receptor - held.receptor).abs() < 1e-3 && (start.lane - held.lane).abs() < 1e-3);

    let idle = feedback.lane_brightness(0, 100_000.0);
    assert_eq!((idle.receptor, idle.lane), (0.0, 0.0));
}

#[test]
fn a_higher_sequence_press_restores_brightness_and_old_sequences_are_ignored() {
    let mut feedback = GameplayFeedback::new();
    press(&mut feedback, 0, 40, 20_000.0);
    let initial = feedback.lane_brightness(0, 20_000.0);
    release(&mut feedback, 0, 41, 20_001.0);

    let decaying = feedback.lane_brightness(0, 20_002.0);
    assert!(decaying.receptor > 0.0 && decaying.receptor < initial.receptor);

    press(&mut feedback, 0, 42, 20_002.0);
    let restored = feedback.lane_brightness(0, 20_002.0);
    assert!(
        (restored.receptor - initial.receptor).abs() < 1e-3
            && (restored.lane - initial.lane).abs() < 1e-3
    );

    release(&mut feedback, 0, 41, 20_003.0);
    let held = feedback.lane_brightness(0, 100_000.0);
    assert!(held.receptor > 0.0 && held.lane > 0.0);
    let latest = feedback.latest_lane_input_transition(0).unwrap();
    assert_eq!((latest.sequence_number, latest.phase), (42, Phase::Press));
}

#[test]
fn lanes_outside_the_playfield_are_ignored() {
    let mut feedback = GameplayFeedback::new();
    press(&mut feedback, 4, 1, 0.0);
    assert_eq!(feedback.latest_lane_input_transition(4), None);
    assert_eq!(
        feedback.lane_brightness(4, 0.0),
        LaneBrightness {
            receptor: 0.0,
            lane: 0.0
        }
    );
}
