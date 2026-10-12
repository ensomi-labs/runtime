//! UniFFI bindings of the runtime core: the gameplay session, feedback state, note layout, the
//! `.osu` loader, the chart generator and its `.osz` export ([`generator`]), and the scheduler that generates
//! for play ([`scheduler`]).
//!
//! The data types are the core's own, declared here again for the binding generator
//! (`uniffi::remote`), so a change to a core type that is not mirrored fails to compile. The two stateful
//! parts are objects that lock their core value.
//!
//! No panic reaches the foreign side. UniFFI turns a panic into an error, and the Swift bindings call a method
//! that does not throw with `try!`, which aborts the app on that error; so the per-frame methods, none of which
//! throw, catch a panic themselves (`frame`). The gameplay core does not panic on any input a session or
//! feedback state accepts (`crates/gameplay/tests/no_panic.rs`); the catch is a backstop.

use ensomi_gameplay as gameplay;
use ensomi_gameplay::{
    Chart, ChartError, ChartObject, ChartToken, Geometry, InputEvent, Judge, Judgement,
    JudgementEvent, JudgementEventBatch, JudgementPresentation, LaneBrightness,
    LaneInputTransition, LaneState, OsuBeatmap, OsuError, Part, Phase, Ruleset, Score, Tier,
    TierCounts, TokenKind, VisibleObject, VisibleState,
};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub mod generator;
pub mod scheduler;

uniffi::setup_scaffolding!();

#[uniffi::remote(Record)]
pub struct ChartObject {
    pub lane: u32,
    pub start_ms: u32,
    pub end_ms: Option<u32>,
}

#[uniffi::remote(Record)]
pub struct Chart {
    pub duration_ms: u32,
    pub objects: Vec<ChartObject>,
}

#[uniffi::remote(Enum)]
pub enum TokenKind {
    Tap,
    HoldStart,
    HoldEnd,
}

#[uniffi::remote(Record)]
pub struct ChartToken {
    pub lane: u32,
    pub time_ms: f64,
    pub kind: TokenKind,
}

#[uniffi::remote(Error)]
pub enum ChartError {
    InvalidLane { object: u32 },
    InvalidHold { object: u32 },
    OutsideDuration { object: u32 },
    NotSorted { object: u32 },
    SameLaneOverlap { object: u32 },
    InvalidTime { object: u32 },
    BeforeCoverage { object: u32 },
    HoldEndWithoutStart { lane: u32 },
    UnclosedHold { object: u32 },
}

#[uniffi::remote(Record)]
pub struct OsuBeatmap {
    pub chart: Chart,
    pub title: Option<String>,
    pub artist: Option<String>,
}

#[uniffi::remote(Error)]
pub enum OsuError {
    UnsupportedMode,
    UnsupportedKeyCount { keys: i64 },
    UnsupportedHitObject { line: String },
    InvalidChart { error: ChartError },
}

#[uniffi::remote(Enum)]
pub enum Judge {
    A,
    B,
    C,
    D,
    E,
}

#[uniffi::remote(Record)]
pub struct Ruleset {
    pub judge: Judge,
}

#[uniffi::remote(Enum)]
pub enum Phase {
    Press,
    Release,
}

#[uniffi::remote(Record)]
pub struct InputEvent {
    pub lane: u32,
    pub phase: Phase,
    pub time_ms: f64,
}

#[uniffi::remote(Enum)]
pub enum Part {
    Head,
    Tail,
}

#[uniffi::remote(Enum)]
pub enum Tier {
    BigP,
    P1,
    P2,
    P3,
    G,
    M,
}

#[uniffi::remote(Enum)]
pub enum Judgement {
    Perfect,
    Good,
    Miss,
}

#[uniffi::remote(Record)]
pub struct JudgementEvent {
    pub id: u64,
    pub object: u32,
    pub lane: u32,
    pub part: Part,
    pub tier: Tier,
    pub offset_ms: Option<f64>,
    pub time_ms: f64,
    pub object_time_ms: f64,
}

#[uniffi::remote(Record)]
pub struct TierCounts {
    pub big_p: u32,
    pub p1: u32,
    pub p2: u32,
    pub p3: u32,
    pub g: u32,
    pub m: u32,
}

#[uniffi::remote(Record)]
pub struct Score {
    pub tier_counts: TierCounts,
    pub accuracy: f64,
    pub combo: u32,
    pub max_combo: u32,
    pub suggested_audio_offset_ms: Option<f64>,
}

#[uniffi::remote(Record)]
pub struct LaneState {
    pub lane: u32,
    pub is_pressed: bool,
    pub holding_object: Option<u32>,
}

#[uniffi::remote(Enum)]
pub enum VisibleState {
    Waiting,
    Holding,
    OpenEnded,
    MissedButVisible,
    Resolved,
}

#[uniffi::remote(Record)]
pub struct VisibleObject {
    pub id: u32,
    pub lane: u32,
    pub start_time_ms: f64,
    pub end_time_ms: Option<f64>,
    pub state: VisibleState,
}

#[uniffi::remote(Record)]
pub struct JudgementEventBatch {
    pub id: u64,
    pub chart_time_ms: f64,
    pub events: Vec<JudgementEvent>,
}

#[uniffi::remote(Record)]
pub struct JudgementPresentation {
    pub event: JudgementEvent,
    pub opacity: f64,
    pub scale: f64,
    pub vertical_offset: f64,
}

#[uniffi::remote(Record)]
pub struct LaneInputTransition {
    pub lane: u32,
    pub phase: Phase,
    pub sequence_number: u64,
    pub ui_time_ms: f64,
}

#[uniffi::remote(Record)]
pub struct LaneBrightness {
    pub receptor: f64,
    pub lane: f64,
}

#[uniffi::remote(Enum)]
pub enum Geometry {
    Tap {
        y: f64,
    },
    Hold {
        body_top_y: f64,
        body_bottom_y: f64,
        head_y: f64,
        tail_y: Option<f64>,
    },
}

/// Locks a core value. After a panic in another call the value is used as that call left it, rather than
/// panicking again.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs a call that does not throw on the locked value; a panic returns the default value instead.
fn frame<T, R: Default>(mutex: &Mutex<T>, call: impl FnOnce(&mut T) -> R) -> R {
    catch_unwind(AssertUnwindSafe(|| call(&mut lock(mutex)))).unwrap_or_default()
}

/// Judgement of one chart, complete or streamed, under one ruleset; see `ensomi_gameplay::Session`. The events that
/// a page appended by a `ChartScheduler` decides come out of the next `input` or `advance`, before their own.
#[derive(uniffi::Object)]
pub struct GameplaySession(Mutex<gameplay::Session>, Mutex<Vec<JudgementEvent>>);

impl GameplaySession {
    /// Appends a scheduler's page and keeps the events it decides for the next `input` or `advance`.
    pub(crate) fn append_page(
        &self,
        tokens: &[ChartToken],
        coverage_ms: f64,
        end_of_stream: bool,
    ) -> Result<(), ChartError> {
        let mut session = lock(&self.0);
        let events = session.append_tokens(tokens, coverage_ms, end_of_stream)?;
        lock(&self.1).extend(events);
        Ok(())
    }

    /// The events kept from scheduler pages, then those of `call`, which runs on the locked session.
    fn decide(
        &self,
        call: impl FnOnce(&mut gameplay::Session) -> Vec<JudgementEvent>,
    ) -> Vec<JudgementEvent> {
        frame(&self.0, |s| {
            let mut events = std::mem::take(&mut *lock(&self.1));
            events.extend(call(s));
            events
        })
    }
}

#[uniffi::export]
impl GameplaySession {
    #[uniffi::constructor]
    pub fn new(chart: Chart, ruleset: Ruleset) -> Result<Arc<Self>, ChartError> {
        Ok(Arc::new(Self(
            Mutex::new(gameplay::Session::new(&chart, ruleset)?),
            Mutex::default(),
        )))
    }

    #[uniffi::constructor]
    pub fn streaming(ruleset: Ruleset) -> Arc<Self> {
        Arc::new(Self(
            Mutex::new(gameplay::Session::streaming(ruleset)),
            Mutex::default(),
        ))
    }

    pub fn append(
        &self,
        objects: Vec<ChartObject>,
        coverage_ms: f64,
        end_of_stream: bool,
    ) -> Result<Vec<JudgementEvent>, ChartError> {
        lock(&self.0).append(&objects, coverage_ms, end_of_stream)
    }

    pub fn append_tokens(
        &self,
        tokens: Vec<ChartToken>,
        coverage_ms: f64,
        end_of_stream: bool,
    ) -> Result<Vec<JudgementEvent>, ChartError> {
        lock(&self.0).append_tokens(&tokens, coverage_ms, end_of_stream)
    }

    pub fn coverage_ms(&self) -> f64 {
        frame(&self.0, |s| s.coverage_ms())
    }

    pub fn decided_through_ms(&self) -> f64 {
        frame(&self.0, |s| s.decided_through_ms())
    }

    pub fn input(&self, input: InputEvent) -> Vec<JudgementEvent> {
        self.decide(|s| s.input(input))
    }

    pub fn advance(&self, watermark_ms: f64) -> Vec<JudgementEvent> {
        self.decide(|s| s.advance(watermark_ms))
    }

    /// Arithmetic on the counts, which cannot panic, so without the backstop of `frame`.
    pub fn score(&self) -> Score {
        lock(&self.0).score()
    }

    pub fn lane_states(&self) -> Vec<LaneState> {
        frame(&self.0, |s| s.lane_states())
    }

    pub fn visible_objects(&self, lower_ms: f64, upper_ms: f64) -> Vec<VisibleObject> {
        frame(&self.0, |s| s.visible_objects(lower_ms, upper_ms))
    }

    pub fn is_resolved(&self) -> bool {
        frame(&self.0, |s| s.is_resolved())
    }

    pub fn late_inputs(&self) -> u32 {
        frame(&self.0, |s| s.late_inputs())
    }
}

/// Judgement popup and lane lighting state; see `ensomi_gameplay::GameplayFeedback`.
#[derive(uniffi::Object)]
pub struct GameplayFeedback(Mutex<gameplay::GameplayFeedback>);

#[uniffi::export]
impl GameplayFeedback {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(gameplay::GameplayFeedback::new())))
    }

    pub fn record_judgement_events(&self, events: Vec<JudgementEvent>, chart_time_ms: f64) {
        frame(&self.0, |f| f.record_judgement_events(&events, chart_time_ms));
    }

    pub fn advance_judgement_time(&self, chart_time_ms: f64) {
        frame(&self.0, |f| f.advance_judgement_time(chart_time_ms));
    }

    pub fn record_input(&self, lane: u32, phase: Phase, sequence_number: u64, ui_time_ms: f64) {
        frame(&self.0, |f| f.record_input(lane, phase, sequence_number, ui_time_ms));
    }

    pub fn judgement_presentation(&self, chart_time_ms: f64) -> Option<JudgementPresentation> {
        frame(&self.0, |f| f.judgement_presentation(chart_time_ms))
    }

    /// Easing of a recorded transition, which cannot panic, so without the backstop of `frame`.
    pub fn lane_brightness(&self, lane: u32, ui_time_ms: f64) -> LaneBrightness {
        lock(&self.0).lane_brightness(lane, ui_time_ms)
    }

    pub fn judgement_event_batches(&self) -> Vec<JudgementEventBatch> {
        frame(&self.0, |f| f.judgement_event_batches().to_vec())
    }

    pub fn latest_lane_input_transition(&self, lane: u32) -> Option<LaneInputTransition> {
        frame(&self.0, |f| f.latest_lane_input_transition(lane))
    }
}

/// Reads an osu!mania 4K beatmap; see `ensomi_gameplay::parse_osu`.
#[uniffi::export]
pub fn parse_osu(text: String) -> Result<OsuBeatmap, OsuError> {
    gameplay::parse_osu(&text)
}

/// What the player is shown for a tier.
#[uniffi::export]
pub fn tier_judgement(tier: Tier) -> Judgement {
    tier.judgement()
}

/// Where to draw an object; see `ensomi_gameplay::geometry`.
#[uniffi::export]
pub fn note_geometry(
    object: VisibleObject,
    render_time_ms: f64,
    scroll_time_ms: f64,
    receptor_y: f64,
) -> Geometry {
    gameplay::geometry(&object, render_time_ms, scroll_time_ms, receptor_y)
}

#[uniffi::export]
pub fn note_y_position(
    object_time_ms: f64,
    render_time_ms: f64,
    scroll_time_ms: f64,
    receptor_y: f64,
) -> f64 {
    gameplay::y_position(object_time_ms, render_time_ms, scroll_time_ms, receptor_y)
}
