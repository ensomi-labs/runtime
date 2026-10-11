//! Ensomi 4K mania gameplay rules.
//!
//! No IO and no clock: the caller passes chart objects, a ruleset, timestamped inputs and a
//! watermark. Judgement depends on those only, never on how often `advance` is called.
//! Rules and the differences from the Swift engine are in `README.md`.

mod chart;
mod feedback;
mod layout;
mod rules;
mod session;

pub use chart::{Chart, ChartError, ChartObject};
pub use feedback::{
    GameplayFeedback, JudgementEventBatch, JudgementPresentation, LaneBrightness,
    LaneInputTransition,
};
pub use layout::{Geometry, MINIMUM_HOLD_BODY_HEIGHT, geometry, y_position};
pub use rules::{Judge, Judgement, Ruleset, TAIL_LENIENCE, Tier, TierCounts, Windows};
pub use session::{
    InputEvent, JudgementEvent, LANES, LaneState, Part, Phase, Score, Session, VisibleObject,
    VisibleState,
};
