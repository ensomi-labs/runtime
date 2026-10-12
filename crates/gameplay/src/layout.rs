//! Where a playfield draws a note. Platforms draw natively from these numbers; y grows downward and
//! the receptor line is at `receptor_y`.

use crate::{VisibleObject, VisibleState};

pub const MINIMUM_HOLD_BODY_HEIGHT: f64 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Geometry {
    Tap {
        y: f64,
    },
    /// `tail_y` is `None` for a hold without a known end. `body_bottom_y - body_top_y` is the body height
    /// and their mean its centre.
    Hold {
        body_top_y: f64,
        body_bottom_y: f64,
        head_y: f64,
        tail_y: Option<f64>,
    },
}

/// Vertical position of a time: a note at `render_time_ms` is on the receptor line, and one a full
/// `scroll_time_ms` ahead of it is 18 below the top of the lane.
pub fn y_position(
    object_time_ms: f64,
    render_time_ms: f64,
    scroll_time_ms: f64,
    receptor_y: f64,
) -> f64 {
    let travel_height = (receptor_y - 18.0).max(1.0);
    let progress = (object_time_ms - render_time_ms) / scroll_time_ms.max(1.0);
    receptor_y - progress * travel_height
}

pub fn geometry(
    object: &VisibleObject,
    render_time_ms: f64,
    scroll_time_ms: f64,
    receptor_y: f64,
) -> Geometry {
    let displayed_y = |time_ms: f64| {
        let raw_y = y_position(time_ms, render_time_ms, scroll_time_ms, receptor_y);
        if object.state == VisibleState::Holding {
            raw_y.min(receptor_y)
        } else {
            raw_y
        }
    };
    let head_y = displayed_y(object.start_time_ms);
    let is_hold = object.end_time_ms.is_some()
        || matches!(
            object.state,
            VisibleState::Holding | VisibleState::OpenEnded
        );
    if !is_hold {
        return Geometry::Tap { y: head_y };
    }
    let tail_y = object.end_time_ms.map(displayed_y);
    let raw_top_y = tail_y.map_or(0.0_f64.min(head_y), |tail| head_y.min(tail));
    let raw_bottom_y = tail_y.map_or(0.0_f64.max(head_y), |tail| head_y.max(tail));
    let body_height = (raw_bottom_y - raw_top_y).max(MINIMUM_HOLD_BODY_HEIGHT);
    Geometry::Hold {
        body_top_y: raw_bottom_y - body_height,
        body_bottom_y: raw_bottom_y,
        head_y,
        tail_y,
    }
}
