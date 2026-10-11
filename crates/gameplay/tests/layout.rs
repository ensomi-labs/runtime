//! The cases of the Swift `Mania4KNoteRenderLayoutTests`.

use ensomi_gameplay::*;

fn object(start_ms: f64, end_ms: Option<f64>, state: VisibleState) -> VisibleObject {
    VisibleObject {
        id: 0,
        lane: 0,
        start_time_ms: start_ms,
        end_time_ms: end_ms,
        state,
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn a_held_hold_is_clamped_to_the_receptor_after_its_head_passes() {
    let held = object(1_000.0, Some(2_000.0), VisibleState::Holding);
    let Geometry::Hold {
        body_top_y,
        body_bottom_y,
        head_y,
        tail_y,
    } = geometry(&held, 1_250.0, 1_000.0, 520.0)
    else {
        panic!("expected hold geometry");
    };
    assert!(near(head_y, 520.0) && near(body_bottom_y, 520.0));
    assert!(body_top_y < body_bottom_y);
    assert!(tail_y.unwrap() < head_y);
}

#[test]
fn an_open_ended_hold_reaches_the_lane_top_without_a_tail() {
    let open = object(500.0, None, VisibleState::OpenEnded);
    let Geometry::Hold {
        body_top_y,
        body_bottom_y,
        head_y,
        tail_y,
    } = geometry(&open, 0.0, 1_000.0, 520.0)
    else {
        panic!("expected hold geometry");
    };
    assert_eq!(tail_y, None);
    assert!(near(body_top_y, 0.0) && near(body_bottom_y, head_y));
}

#[test]
fn render_time_moves_a_note_without_involving_gameplay_time() {
    assert!(
        y_position(1_500.0, 1_100.0, 1_000.0, 520.0) > y_position(1_500.0, 1_000.0, 1_000.0, 520.0)
    );
}

#[test]
fn a_tap_is_one_position_and_a_short_hold_keeps_a_minimum_body() {
    let tap = object(1_000.0, None, VisibleState::Waiting);
    assert_eq!(
        geometry(&tap, 500.0, 1_000.0, 520.0),
        Geometry::Tap {
            y: y_position(1_000.0, 500.0, 1_000.0, 520.0)
        }
    );

    let short = object(1_000.0, Some(1_001.0), VisibleState::Waiting);
    let Geometry::Hold {
        body_top_y,
        body_bottom_y,
        ..
    } = geometry(&short, 500.0, 1_000.0, 520.0)
    else {
        panic!("expected hold geometry");
    };
    assert!(near(body_bottom_y - body_top_y, MINIMUM_HOLD_BODY_HEIGHT));
}

#[test]
fn positions_follow_the_scroll_formula() {
    // travel = 520 - 18 = 502; a quarter of the scroll time ahead is a quarter of the travel above the receptor.
    assert!(near(
        y_position(1_250.0, 1_000.0, 1_000.0, 520.0),
        520.0 - 125.5
    ));
    // Degenerate scroll time and receptor are clamped to 1.
    assert!(near(y_position(1.0, 0.0, 0.0, 10.0), 10.0 - 1.0));
}
