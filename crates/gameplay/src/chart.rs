use crate::{Judge, Ruleset, Session};
use std::fmt;

/// One note, as in EnsomiProtocol's `ChartObject`: audio time in whole milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChartObject {
    /// Column, 0 to 3 from left to right.
    pub lane: u32,
    pub start_ms: u32,
    /// Release time of a hold. `None` for a tap. The protocol promises it is greater than `start_ms`; a
    /// session also accepts equal, which is what the .osu loader makes of a hold that ends early.
    pub end_ms: Option<u32>,
}

/// A complete chart, as in EnsomiProtocol's `Chart`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chart {
    pub duration_ms: u32,
    /// Sorted by `start_ms`, then lane.
    pub objects: Vec<ChartObject>,
}

/// What a token stream delivers: a hold arrives as a start and, possibly in a later page, an end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Tap,
    HoldStart,
    HoldEnd,
}

/// One token of the macOS app's object stream (`Mania4KHitObject`): audio time in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartToken {
    pub lane: u32,
    pub time_ms: f64,
    pub kind: TokenKind,
}

/// Why a chart, or a page of a chart stream, is rejected. `object` is the index the object has, or would
/// have, in chart order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartError {
    InvalidLane {
        object: u32,
    },
    /// `end_ms` is before `start_ms`.
    InvalidHold {
        object: u32,
    },
    /// A time lies beyond `duration_ms`.
    OutsideDuration {
        object: u32,
    },
    NotSorted {
        object: u32,
    },
    /// A note starts at or before the start of the previous note of its lane, or inside its hold.
    SameLaneOverlap {
        object: u32,
    },
    /// A token time is negative or not finite.
    InvalidTime {
        object: u32,
    },
    /// An object starts, or a hold ends, at or before a coverage the stream already declared, or after
    /// the end of the stream.
    BeforeCoverage {
        object: u32,
    },
    /// A hold end token in a lane without a hold waiting for its end.
    HoldEndWithoutStart {
        lane: u32,
    },
    /// The stream ended before this hold's end arrived.
    UnclosedHold {
        object: u32,
    },
}

impl fmt::Display for ChartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLane { object } => write!(f, "object {object}: lane is not 0 to 3"),
            Self::InvalidHold { object } => {
                write!(f, "object {object}: hold ends before its start")
            }
            Self::OutsideDuration { object } => {
                write!(f, "object {object}: time is beyond the chart duration")
            }
            Self::NotSorted { object } => {
                write!(f, "object {object}: not sorted by start time, then lane")
            }
            Self::SameLaneOverlap { object } => write!(
                f,
                "object {object}: overlaps the previous object of its lane"
            ),
            Self::InvalidTime { object } => {
                write!(f, "object {object}: time is negative or not finite")
            }
            Self::BeforeCoverage { object } => write!(
                f,
                "object {object}: arrived after the stream declared its time complete"
            ),
            Self::HoldEndWithoutStart { lane } => {
                write!(f, "lane {lane}: hold end without a hold start")
            }
            Self::UnclosedHold { object } => {
                write!(f, "object {object}: the stream ended before the hold's end")
            }
        }
    }
}

impl std::error::Error for ChartError {}

impl Chart {
    /// Checks what the protocol promises of a chart. A hold may end exactly where the next note of its
    /// lane starts.
    pub fn validate(&self) -> Result<(), ChartError> {
        Session::new(self, Ruleset { judge: Judge::C }).map(drop)
    }
}
