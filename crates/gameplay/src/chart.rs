use std::fmt;

/// One note, as in EnsomiProtocol's `ChartObject`: audio time in whole milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChartObject {
    /// Column, 0 to 3 from left to right.
    pub lane: u32,
    pub start_ms: u32,
    /// Release time of a hold, greater than `start_ms`. `None` for a tap.
    pub end_ms: Option<u32>,
}

/// A complete chart, as in EnsomiProtocol's `Chart`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chart {
    pub duration_ms: u32,
    /// Sorted by `start_ms`, then lane.
    pub objects: Vec<ChartObject>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartError {
    InvalidLane {
        object: u32,
    },
    /// `end_ms` is not greater than `start_ms`.
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
}

impl fmt::Display for ChartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLane { object } => write!(f, "object {object}: lane is not 0 to 3"),
            Self::InvalidHold { object } => {
                write!(f, "object {object}: hold ends at or before its start")
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
        }
    }
}

impl std::error::Error for ChartError {}

impl Chart {
    /// Checks what the protocol promises of a chart. A hold may end exactly where the next note of its
    /// lane starts.
    pub fn validate(&self) -> Result<(), ChartError> {
        let mut previous: Option<(u32, u32)> = None;
        let mut lane_last: [Option<(u32, u32)>; 4] = [None; 4];
        for (index, object) in self.objects.iter().enumerate() {
            let id = index as u32;
            let Some(last) = lane_last.get_mut(object.lane as usize) else {
                return Err(ChartError::InvalidLane { object: id });
            };
            let end = object.end_ms.unwrap_or(object.start_ms);
            if object.end_ms.is_some_and(|end| end <= object.start_ms) {
                return Err(ChartError::InvalidHold { object: id });
            }
            if end > self.duration_ms {
                return Err(ChartError::OutsideDuration { object: id });
            }
            if previous.is_some_and(|key| key > (object.start_ms, object.lane)) {
                return Err(ChartError::NotSorted { object: id });
            }
            if last.is_some_and(|(start, last_end)| {
                object.start_ms <= start || object.start_ms < last_end
            }) {
                return Err(ChartError::SameLaneOverlap { object: id });
            }
            previous = Some((object.start_ms, object.lane));
            *last = Some((object.start_ms, end));
        }
        Ok(())
    }
}
