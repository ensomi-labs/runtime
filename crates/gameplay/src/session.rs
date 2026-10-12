use crate::{
    Chart, ChartError, ChartObject, ChartToken, Ruleset, TAIL_LENIENCE, Tier, TierCounts,
    TokenKind, Windows,
};
use std::cmp::Ordering;
use std::collections::VecDeque;

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
    /// A streamed hold whose end has not arrived, not pressed yet.
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
    hold: bool,
    /// `None` for a tap, and for a streamed hold whose end has not arrived.
    end: Option<f64>,
    state: State,
}

impl Object {
    fn is_resolved(&self) -> bool {
        matches!(self.state, State::Resolved(_))
    }

    /// The end of a hold, else the start. In a lane it rises with the position, as the start does.
    fn finish(&self) -> f64 {
        self.end.unwrap_or(self.start)
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
///
/// A streamed chart ([`Session::streaming`]) arrives in pages, each with a coverage. The session decides
/// nothing the delivered objects cannot settle: it judges up to the coverage and keeps later inputs until
/// a page covers them, so the outcome is the one of the complete chart, whatever the page and advance
/// schedule. [`Session::decided_through_ms`] tells the caller how far it got.
///
/// What a frame calls costs in proportion to the objects near the current time, not to the chart length:
/// a scan of a lane starts at its cursor or is bounded by a search on the times, which rise along a lane.
/// A note leaves the waiting state only once every note before it in its lane has, so a lane holds
/// held-or-resolved objects, then waiting ones.
pub struct Session {
    windows: Windows,
    objects: Vec<Object>,
    /// Object indexes per lane, in chart order.
    lanes: [Vec<usize>; LANES],
    /// Per lane, position in `lanes` before which every object is resolved.
    cursor: [usize; LANES],
    /// Objects with an outcome.
    resolved: usize,
    pressed: [bool; LANES],
    /// The session clock: the largest watermark or input time seen. Chart time can be negative: the audio
    /// offset puts the start of the audio before 0.
    now: f64,
    /// Every object starting at or before this has been delivered, and for a token stream every hold end
    /// at or before it. Infinite once the stream has ended.
    coverage: f64,
    /// Inputs delivered that the coverage does not settle yet, in order, each stamped with the time it is
    /// handled at.
    pending: VecDeque<InputEvent>,
    late_inputs: u32,
    next_event_id: u64,
    counts: TierCounts,
    combo: u32,
    max_combo: u32,
    offset_sum: f64,
    offset_count: u32,
}

impl Session {
    /// A session for a complete chart. Rejects what the protocol forbids of it.
    pub fn new(chart: &Chart, ruleset: Ruleset) -> Result<Self, ChartError> {
        let mut session = Self::streaming(ruleset);
        for (index, object) in chart.objects.iter().enumerate() {
            session.push_object(object)?;
            if object.end_ms.unwrap_or(object.start_ms) > chart.duration_ms {
                return Err(ChartError::OutsideDuration {
                    object: index as u32,
                });
            }
        }
        session.coverage = f64::INFINITY;
        Ok(session)
    }

    /// A session for a chart that arrives in pages, through [`Session::append`] or
    /// [`Session::append_tokens`]. Until the first page it covers nothing.
    pub fn streaming(ruleset: Ruleset) -> Self {
        Self {
            windows: ruleset.windows(),
            objects: Vec::new(),
            lanes: Default::default(),
            cursor: [0; LANES],
            resolved: 0,
            pressed: [false; LANES],
            now: f64::NEG_INFINITY,
            coverage: f64::NEG_INFINITY,
            pending: VecDeque::new(),
            late_inputs: 0,
            next_event_id: 0,
            counts: TierCounts::default(),
            combo: 0,
            max_combo: 0,
            offset_sum: 0.0,
            offset_count: 0,
        }
    }

    /// Adds a page of whole objects, as EnsomiProtocol's `ChartPage` carries them: in chart order, each
    /// starting after the coverage of the previous page, a hold with its final end. `coverage_ms`: every
    /// object starting at or before it has now been delivered. `end_of_stream`: every object has.
    /// Returns the events the new coverage decides.
    pub fn append(
        &mut self,
        objects: &[ChartObject],
        coverage_ms: f64,
        end_of_stream: bool,
    ) -> Result<Vec<JudgementEvent>, ChartError> {
        for object in objects {
            self.push_object(object)?;
        }
        self.cover(coverage_ms, end_of_stream)
    }

    /// Adds a page of tokens, as the macOS app's object streams deliver them: taps and hold starts in chart
    /// order, each after the previous coverage; a hold's end may come in a later page, but before any
    /// later note of its lane. `coverage_ms`: every token at or before it has now been delivered.
    /// `end_of_stream`: every token has. Returns the events the new coverage decides.
    pub fn append_tokens(
        &mut self,
        tokens: &[ChartToken],
        coverage_ms: f64,
        end_of_stream: bool,
    ) -> Result<Vec<JudgementEvent>, ChartError> {
        for token in tokens {
            match token.kind {
                TokenKind::Tap => self.push(token.lane, token.time_ms, false, None)?,
                TokenKind::HoldStart => self.push(token.lane, token.time_ms, true, None)?,
                TokenKind::HoldEnd => self.close(token.lane, token.time_ms)?,
            }
        }
        self.cover(coverage_ms, end_of_stream)
    }

    /// How far the stream is complete: infinite for a complete chart and once a stream has ended.
    pub fn coverage_ms(&self) -> f64 {
        self.coverage
    }

    /// Chart time up to which the session has judged: the clock, held back to the coverage and to the
    /// first input that waits for more of the stream. When it is below the last watermark passed to
    /// [`Session::advance`], the stream has fallen behind play; the caller waits for pages or stops.
    pub fn decided_through_ms(&self) -> f64 {
        self.coverage
            .min(self.pending.front().map_or(self.now, |input| input.time_ms))
    }

    /// The caller declares that every input stamped at or before `watermark_ms` has been delivered.
    /// Resolves the misses due by then. Returns the events decided.
    pub fn advance(&mut self, watermark_ms: f64) -> Vec<JudgementEvent> {
        self.now = self.now.max(watermark_ms);
        let mut events = Vec::new();
        self.settle(&mut events);
        events
    }

    /// Handles a key transition. One stamped before the session clock was delivered late and is
    /// handled at the clock instead; see [`Session::late_inputs`]. Lanes outside 0 to 3 are ignored.
    /// In a stream, an input waits until the coverage settles it (see [`Session::decided_through_ms`]).
    pub fn input(&mut self, input: InputEvent) -> Vec<JudgementEvent> {
        if input.time_ms < self.now {
            self.late_inputs += 1;
        }
        self.now = self.now.max(input.time_ms);
        self.pending.push_back(InputEvent {
            time_ms: self.now,
            ..input
        });
        let mut events = Vec::new();
        self.settle(&mut events);
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
        let mut shown = Vec::new();
        for (lane, objects) in self.lanes.iter().enumerate() {
            // From the first object not over before the range to the last one started in it, or further
            // for the held and resolved ones, which are shown whatever their start. The last object of a
            // lane may be a hold without its end, drawn up to the range, so it is always looked at.
            let from = objects
                .partition_point(|&i| self.objects[i].finish() < lower_ms)
                .min(objects.len().saturating_sub(1));
            let to = objects
                .partition_point(|&i| self.objects[i].start <= upper_ms)
                .max(self.first_waiting(lane));
            shown.extend(
                objects
                    .get(from..to)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|&i| self.visible(i, lower_ms, upper_ms)),
            );
        }
        shown.sort_unstable_by_key(|object| object.id);
        shown
    }

    /// Whether the stream has ended and every object has an outcome.
    pub fn is_resolved(&self) -> bool {
        self.coverage == f64::INFINITY && self.resolved == self.objects.len()
    }

    /// The object as drawn for a range, if it shows in it.
    fn visible(&self, index: usize, lower_ms: f64, upper_ms: f64) -> Option<VisibleObject> {
        let object = &self.objects[index];
        let object_end = object
            .end
            .unwrap_or(if object.hold && !object.is_resolved() {
                upper_ms
            } else {
                object.start
            });
        let visible = object.start <= upper_ms && object_end >= lower_ms;
        let keep_resolved =
            object.is_resolved() && self.now - object_end <= 300.0 && object_end >= lower_ms;
        (visible || keep_resolved).then_some(VisibleObject {
            id: index as u32,
            lane: object.lane as u32,
            start_time_ms: object.start,
            end_time_ms: object.end,
            state: match object.state {
                State::Waiting if object.hold && object.end.is_none() => VisibleState::OpenEnded,
                State::Waiting => VisibleState::Waiting,
                State::Holding { .. } => VisibleState::Holding,
                State::Resolved(Tier::M) => VisibleState::MissedButVisible,
                State::Resolved(_) => VisibleState::Resolved,
            },
        })
    }

    fn push_object(&mut self, object: &ChartObject) -> Result<(), ChartError> {
        self.push(
            object.lane,
            f64::from(object.start_ms),
            object.end_ms.is_some(),
            object.end_ms.map(f64::from),
        )
    }

    /// Adds an object after checking its place in the chart. A hold's `end` is `None` until its end token.
    fn push(
        &mut self,
        lane: u32,
        start: f64,
        hold: bool,
        end: Option<f64>,
    ) -> Result<(), ChartError> {
        let object = self.objects.len() as u32;
        let Some(lane_objects) = self.lanes.get(lane as usize) else {
            return Err(ChartError::InvalidLane { object });
        };
        if !(0.0..f64::INFINITY).contains(&start) {
            return Err(ChartError::InvalidTime { object });
        }
        if end.is_some_and(|end| end < start) {
            return Err(ChartError::InvalidHold { object });
        }
        if start <= self.coverage {
            return Err(ChartError::BeforeCoverage { object });
        }
        if self
            .objects
            .last()
            .is_some_and(|last| (last.start, last.lane as u32) > (start, lane))
        {
            return Err(ChartError::NotSorted { object });
        }
        if lane_objects.last().is_some_and(|&last| {
            let last = &self.objects[last];
            let last_end = if last.hold {
                last.end
            } else {
                Some(last.start)
            };
            start <= last.start || last_end.is_none_or(|end| start < end)
        }) {
            return Err(ChartError::SameLaneOverlap { object });
        }
        self.lanes[lane as usize].push(self.objects.len());
        self.objects.push(Object {
            lane: lane as usize,
            start,
            hold,
            end,
            state: State::Waiting,
        });
        Ok(())
    }

    /// Sets the end of the lane's hold that is waiting for it.
    fn close(&mut self, lane: u32, end: f64) -> Result<(), ChartError> {
        let Some(index) = self.open_hold(lane as usize) else {
            return Err(ChartError::HoldEndWithoutStart { lane });
        };
        let object = index as u32;
        if !(0.0..f64::INFINITY).contains(&end) {
            return Err(ChartError::InvalidTime { object });
        }
        if end < self.objects[index].start {
            return Err(ChartError::InvalidHold { object });
        }
        if end <= self.coverage {
            return Err(ChartError::BeforeCoverage { object });
        }
        self.objects[index].end = Some(end);
        Ok(())
    }

    /// The last object of a lane, if it is a hold without its end.
    fn open_hold(&self, lane: usize) -> Option<usize> {
        let &last = self.lanes.get(lane)?.last()?;
        let object = &self.objects[last];
        (object.hold && object.end.is_none()).then_some(last)
    }

    fn cover(
        &mut self,
        coverage_ms: f64,
        end_of_stream: bool,
    ) -> Result<Vec<JudgementEvent>, ChartError> {
        if end_of_stream {
            if let Some(index) = (0..LANES).find_map(|lane| self.open_hold(lane)) {
                return Err(ChartError::UnclosedHold {
                    object: index as u32,
                });
            }
            self.coverage = f64::INFINITY;
        } else {
            self.coverage = self.coverage.max(coverage_ms);
        }
        let mut events = Vec::new();
        self.settle(&mut events);
        Ok(events)
    }

    /// Resolves what is due up to [`Session::decided_through_ms`] and handles the waiting inputs the
    /// coverage settles, in time order.
    fn settle(&mut self, events: &mut Vec<JudgementEvent>) {
        loop {
            self.resolve_due(self.decided_through_ms(), events);
            let Some(&input) = self.pending.front() else {
                return;
            };
            if !self.is_settled(&input) {
                return;
            }
            self.pending.pop_front();
            let lane = input.lane as usize;
            if lane < LANES {
                match input.phase {
                    Phase::Press => {
                        self.pressed[lane] = true;
                        self.press(lane, input.time_ms, events);
                    }
                    Phase::Release => {
                        self.pressed[lane] = false;
                        self.release(lane, input.time_ms, events);
                    }
                }
            }
        }
    }

    /// Whether the delivered objects decide an input, once everything due before it is resolved. A press
    /// with no waiting note in its lane could hit one that starts up to `g` later; a release of a hold
    /// whose end has not arrived is too early only if that end is known to lie beyond the tail window.
    fn is_settled(&self, input: &InputEvent) -> bool {
        let time = input.time_ms;
        if time > self.coverage {
            return false;
        }
        let lane = input.lane as usize;
        if lane >= LANES {
            return true;
        }
        match input.phase {
            Phase::Press => self.waiting(lane).is_some() || time + self.windows.g <= self.coverage,
            Phase::Release => self.holding(lane).is_none_or(|(index, ..)| {
                self.objects[index].end.is_some()
                    || time + self.windows.g * TAIL_LENIENCE <= self.coverage
            }),
        }
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
            State::Holding { .. } => first
                .end
                .map_or(f64::INFINITY, |end| end + self.windows.g * TAIL_LENIENCE),
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

    fn resolve_due(&mut self, now: f64, events: &mut Vec<JudgementEvent>) {
        while let Some(expiry) = (0..LANES)
            .filter_map(|lane| self.expiry(lane))
            .filter(|expiry| expiry.is_due(now))
            .min_by(Expiry::order)
        {
            let part = match self.objects[expiry.object].state {
                State::Holding { .. } => Part::Tail,
                _ => Part::Head,
            };
            events.push(self.resolve(expiry.object, Tier::M, part, None, expiry.time));
        }
    }

    fn press(&mut self, lane: usize, now: f64, events: &mut Vec<JudgementEvent>) {
        let Some(index) = self.waiting(lane) else {
            return;
        };
        let error_ms = now - self.objects[index].start;
        if error_ms < -self.windows.g {
            return;
        }
        let tier = self.windows.classify(error_ms, 1.0);
        if !self.objects[index].hold {
            events.push(self.resolve(index, tier, Part::Head, Some(error_ms), now));
        } else {
            self.objects[index].state = State::Holding {
                head_tier: tier,
                head_offset_ms: error_ms,
            };
        }
    }

    /// A release the coverage settles: an unknown end lies beyond the tail window.
    fn release(&mut self, lane: usize, now: f64, events: &mut Vec<JudgementEvent>) {
        let Some((index, head_tier, head_offset_ms)) = self.holding(lane) else {
            return;
        };
        let end = self.objects[index].end.unwrap_or(f64::INFINITY);
        if now < end - self.windows.g * TAIL_LENIENCE {
            events.push(self.resolve(index, Tier::M, Part::Tail, None, now));
            return;
        }
        let tail_tier = self.windows.classify(now - end, TAIL_LENIENCE);
        events.push(self.resolve(
            index,
            head_tier.max(tail_tier),
            Part::Tail,
            Some(head_offset_ms),
            now,
        ));
    }

    /// The first note of a lane waiting for its press.
    fn waiting(&self, lane: usize) -> Option<usize> {
        self.lanes[lane].get(self.first_waiting(lane)).copied()
    }

    /// The first hold being held in a lane, with its head tier and press error.
    fn holding(&self, lane: usize) -> Option<(usize, Tier, f64)> {
        self.lanes[lane][self.cursor[lane]..self.first_waiting(lane)]
            .iter()
            .find_map(|&i| match self.objects[i].state {
                State::Holding {
                    head_tier,
                    head_offset_ms,
                } => Some((i, head_tier, head_offset_ms)),
                _ => None,
            })
    }

    /// Position in `lanes[lane]` of the first waiting note. A note is held or resolved only once every
    /// note before it is, so none after this one is, and a held hold lies between the cursor and it.
    fn first_waiting(&self, lane: usize) -> usize {
        let cursor = self.cursor[lane];
        cursor
            + self.lanes[lane][cursor..]
                .iter()
                .take_while(|&&i| !matches!(self.objects[i].state, State::Waiting))
                .count()
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
        self.resolved += 1;
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
