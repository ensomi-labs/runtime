use crate::{Chart, ChartError, Ruleset, TAIL_LENIENCE, Tier, TierCounts, Windows};
use std::cmp::Ordering;

pub const LANES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Press,
    Release,
}

/// A key transition. `time_ms` is chart time (audio time plus the audio offset).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputEvent {
    pub lane: u32,
    pub phase: Phase,
    pub time_ms: f64,
}

/// Which end of the object the outcome was decided at: `Head` for a tap and for a hold whose head was
/// not held yet, `Tail` once the head was held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Head,
    Tail,
}

/// The outcome of one object. A hold gets one event, with the worse of its head and tail tiers.
#[derive(Clone, Debug, PartialEq)]
pub struct JudgementEvent {
    /// Position in the session's event sequence, from 0.
    pub id: u64,
    /// Index into `Chart::objects`.
    pub object: u32,
    pub lane: u32,
    pub part: Part,
    pub tier: Tier,
    /// Signed press error (press time minus note time) of a tap or hold head. `None` for a miss without
    /// a hit.
    pub offset_ms: Option<f64>,
    /// Chart time at which the outcome was decided.
    pub time_ms: f64,
    /// Start time of the object.
    pub object_time_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Score {
    pub tier_counts: TierCounts,
    /// Mean tier weight over judged objects (1, .9, .85, .8, .4, 0), 1 before any.
    pub accuracy: f64,
    pub combo: u32,
    pub max_combo: u32,
    /// Negative mean of the errors of successfully hit taps and hold heads. `None` before the first.
    pub suggested_audio_offset_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneState {
    pub lane: u32,
    pub is_pressed: bool,
    /// Index of the hold being held in this lane.
    pub holding_object: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibleState {
    Waiting,
    Holding,
    /// A hold without a known end. A session never reports it; the note layout accepts it.
    OpenEnded,
    MissedButVisible,
    Resolved,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibleObject {
    pub id: u32,
    pub lane: u32,
    pub start_time_ms: f64,
    pub end_time_ms: Option<f64>,
    pub state: VisibleState,
}

#[derive(Clone, Copy)]
enum State {
    Waiting,
    Holding {
        head_tier: Tier,
        head_offset_ms: f64,
    },
    Resolved(Tier),
}

struct Object {
    lane: usize,
    start: f64,
    end: Option<f64>,
    state: State,
}

impl Object {
    fn is_resolved(&self) -> bool {
        matches!(self.state, State::Resolved(_))
    }
}

/// A miss that is due: the first unresolved object of a lane ran out of time, either because the next
/// object of the lane started (note lock) or because its window closed.
#[derive(Clone, Copy)]
struct Expiry {
    time: f64,
    lock: bool,
    object: usize,
}

impl Expiry {
    /// A lock is due at its time; a window closes just after it, so an input stamped on the closing
    /// time still hits.
    fn is_due(&self, now: f64) -> bool {
        if self.lock {
            self.time <= now
        } else {
            self.time < now
        }
    }

    /// Time order; at equal times locks first, then chart order.
    fn order(&self, other: &Self) -> Ordering {
        self.time
            .total_cmp(&other.time)
            .then(other.lock.cmp(&self.lock))
            .then(self.object.cmp(&other.object))
    }
}

/// Judgement of one chart under one ruleset.
///
/// Time only moves through [`Session::input`] and [`Session::advance`], and both first resolve every
/// miss that is due, in time order, each at its own chart time. The events and the final score are
/// therefore a function of the inputs alone.
pub struct Session {
    windows: Windows,
    objects: Vec<Object>,
    /// Object indexes per lane, in chart order.
    lanes: [Vec<usize>; LANES],
    /// Per lane, position in `lanes` before which every object is resolved.
    cursor: [usize; LANES],
    pressed: [bool; LANES],
    /// The session clock: the largest watermark or input time seen. Chart time can be negative: the audio
    /// offset puts the start of the audio before 0.
    now: f64,
    late_inputs: u32,
    next_event_id: u64,
    counts: TierCounts,
    combo: u32,
    max_combo: u32,
    offset_sum: f64,
    offset_count: u32,
}

impl Session {
    pub fn new(chart: &Chart, ruleset: Ruleset) -> Result<Self, ChartError> {
        chart.validate()?;
        let mut lanes: [Vec<usize>; LANES] = Default::default();
        let objects = chart
            .objects
            .iter()
            .enumerate()
            .map(|(index, object)| {
                lanes[object.lane as usize].push(index);
                Object {
                    lane: object.lane as usize,
                    start: f64::from(object.start_ms),
                    end: object.end_ms.map(f64::from),
                    state: State::Waiting,
                }
            })
            .collect();
        Ok(Self {
            windows: ruleset.windows(),
            objects,
            lanes,
            cursor: [0; LANES],
            pressed: [false; LANES],
            now: f64::NEG_INFINITY,
            late_inputs: 0,
            next_event_id: 0,
            counts: TierCounts::default(),
            combo: 0,
            max_combo: 0,
            offset_sum: 0.0,
            offset_count: 0,
        })
    }

    /// The caller declares that every input stamped at or before `watermark_ms` has been delivered.
    /// Resolves the misses due by then. Returns the events decided.
    pub fn advance(&mut self, watermark_ms: f64) -> Vec<JudgementEvent> {
        self.now = self.now.max(watermark_ms);
        let mut events = Vec::new();
        self.resolve_due(&mut events);
        events
    }

    /// Handles a key transition. One stamped before the session clock was delivered late and is
    /// handled at the clock instead; see [`Session::late_inputs`]. Lanes outside 0 to 3 are ignored.
    pub fn input(&mut self, input: InputEvent) -> Vec<JudgementEvent> {
        if input.time_ms < self.now {
            self.late_inputs += 1;
        }
        self.now = self.now.max(input.time_ms);
        let mut events = Vec::new();
        self.resolve_due(&mut events);
        let lane = input.lane as usize;
        if lane < LANES {
            match input.phase {
                Phase::Press => {
                    self.pressed[lane] = true;
                    self.press(lane, &mut events);
                }
                Phase::Release => {
                    self.pressed[lane] = false;
                    self.release(lane, &mut events);
                }
            }
        }
        events
    }

    /// Inputs that arrived stamped before the session clock.
    pub fn late_inputs(&self) -> u32 {
        self.late_inputs
    }

    pub fn score(&self) -> Score {
        let c = &self.counts;
        let judged = c.judged();
        let weighted = f64::from(c.big_p)
            + f64::from(c.p1) * 0.90
            + f64::from(c.p2) * 0.85
            + f64::from(c.p3) * 0.80
            + f64::from(c.g) * 0.40;
        Score {
            tier_counts: self.counts,
            accuracy: if judged == 0 {
                1.0
            } else {
                weighted / f64::from(judged)
            },
            combo: self.combo,
            max_combo: self.max_combo,
            suggested_audio_offset_ms: (self.offset_count > 0)
                .then(|| -(self.offset_sum / f64::from(self.offset_count))),
        }
    }

    pub fn lane_states(&self) -> Vec<LaneState> {
        (0..LANES)
            .map(|lane| LaneState {
                lane: lane as u32,
                is_pressed: self.pressed[lane],
                holding_object: self.holding(lane).map(|(index, ..)| index as u32),
            })
            .collect()
    }

    /// Objects to draw for a visible chart-time range: those overlapping it, and resolved ones for 300 ms
    /// after they pass its lower bound.
    pub fn visible_objects(&self, lower_ms: f64, upper_ms: f64) -> Vec<VisibleObject> {
        self.objects
            .iter()
            .enumerate()
            .filter_map(|(index, object)| {
                let object_end = object.end.unwrap_or(object.start);
                let visible = object.start <= upper_ms && object_end >= lower_ms;
                let keep_resolved = object.is_resolved()
                    && self.now - object_end <= 300.0
                    && object_end >= lower_ms;
                (visible || keep_resolved).then_some(VisibleObject {
                    id: index as u32,
                    lane: object.lane as u32,
                    start_time_ms: object.start,
                    end_time_ms: object.end,
                    state: match object.state {
                        State::Waiting => VisibleState::Waiting,
                        State::Holding { .. } => VisibleState::Holding,
                        State::Resolved(Tier::M) => VisibleState::MissedButVisible,
                        State::Resolved(_) => VisibleState::Resolved,
                    },
                })
            })
            .collect()
    }

    /// Whether every object has an outcome.
    pub fn is_resolved(&self) -> bool {
        self.objects.iter().all(Object::is_resolved)
    }

    /// The miss due soonest in a lane. Only the first unresolved object of a lane can have one: the
    /// lock on it precedes the window of any later object.
    fn expiry(&self, lane: usize) -> Option<Expiry> {
        let mut unresolved = self.lanes[lane][self.cursor[lane]..]
            .iter()
            .copied()
            .filter(|&i| !self.objects[i].is_resolved());
        let object = unresolved.next()?;
        let first = &self.objects[object];
        let window = match first.state {
            State::Holding { .. } => {
                first.end.unwrap_or(first.start) + self.windows.g * TAIL_LENIENCE
            }
            _ => first.start + self.windows.g,
        };
        Some(
            match unresolved.next().map(|next| self.objects[next].start) {
                Some(lock) if lock <= window => Expiry {
                    time: lock,
                    lock: true,
                    object,
                },
                _ => Expiry {
                    time: window,
                    lock: false,
                    object,
                },
            },
        )
    }

    fn resolve_due(&mut self, events: &mut Vec<JudgementEvent>) {
        while let Some(expiry) = (0..LANES)
            .filter_map(|lane| self.expiry(lane))
            .filter(|expiry| expiry.is_due(self.now))
            .min_by(Expiry::order)
        {
            let part = match self.objects[expiry.object].state {
                State::Holding { .. } => Part::Tail,
                _ => Part::Head,
            };
            events.push(self.resolve(expiry.object, Tier::M, part, None, expiry.time));
        }
    }

    fn press(&mut self, lane: usize, events: &mut Vec<JudgementEvent>) {
        let Some(&index) = self.lanes[lane][self.cursor[lane]..]
            .iter()
            .find(|&&i| matches!(self.objects[i].state, State::Waiting))
        else {
            return;
        };
        let error_ms = self.now - self.objects[index].start;
        if error_ms < -self.windows.g {
            return;
        }
        let tier = self.windows.classify(error_ms, 1.0);
        if self.objects[index].end.is_none() {
            events.push(self.resolve(index, tier, Part::Head, Some(error_ms), self.now));
        } else {
            self.objects[index].state = State::Holding {
                head_tier: tier,
                head_offset_ms: error_ms,
            };
        }
    }

    fn release(&mut self, lane: usize, events: &mut Vec<JudgementEvent>) {
        let Some((index, head_tier, head_offset_ms)) = self.holding(lane) else {
            return;
        };
        let end = self.objects[index].end.unwrap_or(self.objects[index].start);
        if self.now < end - self.windows.g * TAIL_LENIENCE {
            events.push(self.resolve(index, Tier::M, Part::Tail, None, self.now));
            return;
        }
        let tail_tier = self.windows.classify(self.now - end, TAIL_LENIENCE);
        events.push(self.resolve(
            index,
            head_tier.max(tail_tier),
            Part::Tail,
            Some(head_offset_ms),
            self.now,
        ));
    }

    /// The first hold being held in a lane, with its head tier and press error.
    fn holding(&self, lane: usize) -> Option<(usize, Tier, f64)> {
        self.lanes[lane][self.cursor[lane]..]
            .iter()
            .find_map(|&i| match self.objects[i].state {
                State::Holding {
                    head_tier,
                    head_offset_ms,
                } => Some((i, head_tier, head_offset_ms)),
                _ => None,
            })
    }

    fn resolve(
        &mut self,
        index: usize,
        tier: Tier,
        part: Part,
        offset_ms: Option<f64>,
        time_ms: f64,
    ) -> JudgementEvent {
        self.objects[index].state = State::Resolved(tier);
        let lane = self.objects[index].lane;
        while let Some(&first) = self.lanes[lane].get(self.cursor[lane])
            && self.objects[first].is_resolved()
        {
            self.cursor[lane] += 1;
        }
        self.counts.increment(tier);
        if tier == Tier::M {
            self.combo = 0;
        } else {
            self.combo += 1;
            self.max_combo = self.max_combo.max(self.combo);
            if let Some(offset) = offset_ms {
                self.offset_sum += offset;
                self.offset_count += 1;
            }
        }
        let id = self.next_event_id;
        self.next_event_id += 1;
        JudgementEvent {
            id,
            object: index as u32,
            lane: lane as u32,
            part,
            tier,
            offset_ms,
            time_ms,
            object_time_ms: self.objects[index].start,
        }
    }
}
