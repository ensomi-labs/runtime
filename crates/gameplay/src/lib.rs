//! Ensomi 4K mania gameplay rules.
//!
//! No IO and no clock: the caller passes chart objects (whole or as a stream of pages), a ruleset,
//! timestamped inputs and a watermark. Judgement depends on those only, never on how often `advance` is
//! called. `parse_osu` reads a `.osu` file's text into a chart.
//! Rules and the differences from the Swift engine are in `README.md`.

mod chart;
mod feedback;
mod layout;
mod osu;
mod rules;
mod session;

pub use chart::{Chart, ChartError, ChartObject, ChartToken, TokenKind};
pub use feedback::{
    GameplayFeedback, JudgementEventBatch, JudgementPresentation, LaneBrightness,
    LaneInputTransition,
};
pub use layout::{Geometry, MINIMUM_HOLD_BODY_HEIGHT, geometry, y_position};
pub use osu::{OsuBeatmap, OsuError, parse_osu};
pub use rules::{Judge, Judgement, Ruleset, TAIL_LENIENCE, Tier, TierCounts, Windows};
pub use session::{
    InputEvent, JudgementEvent, LANES, LaneState, Part, Phase, Score, Session, VisibleObject,
    VisibleState,
};
