//! UniFFI bindings of the runtime core: the gameplay session, feedback state and note layout.
//!
//! The data types are the core's own, declared here again for the binding generator
//! (`uniffi::remote`), so a change to a core type that is not mirrored fails to compile. The two stateful
//! parts are objects that lock their core value.

use ensomi_gameplay as gameplay;
use ensomi_gameplay::{
    Chart, ChartError, ChartObject, Geometry, InputEvent, Judge, Judgement, JudgementEvent,
    JudgementEventBatch, JudgementPresentation, LaneBrightness, LaneInputTransition, LaneState,
    Part, Phase, Ruleset, Score, Tier, TierCounts, VisibleObject, VisibleState,
};
use std::sync::{Arc, Mutex};

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

#[uniffi::remote(Error)]
pub enum ChartError {
    InvalidLane { object: u32 },
    InvalidHold { object: u32 },
    OutsideDuration { object: u32 },
    NotSorted { object: u32 },
    SameLaneOverlap { object: u32 },
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

/// Judgement of one chart under one ruleset; see `ensomi_gameplay::Session`.
#[derive(uniffi::Object)]
pub struct GameplaySession(Mutex<gameplay::Session>);

#[uniffi::export]
impl GameplaySession {
    #[uniffi::constructor]
    pub fn new(chart: Chart, ruleset: Ruleset) -> Result<Arc<Self>, ChartError> {
        Ok(Arc::new(Self(Mutex::new(gameplay::Session::new(
            &chart, ruleset,
        )?))))
    }

    pub fn input(&self, input: InputEvent) -> Vec<JudgementEvent> {
        self.0.lock().unwrap().input(input)
    }

    pub fn advance(&self, watermark_ms: f64) -> Vec<JudgementEvent> {
        self.0.lock().unwrap().advance(watermark_ms)
    }

    pub fn score(&self) -> Score {
        self.0.lock().unwrap().score()
    }

    pub fn lane_states(&self) -> Vec<LaneState> {
        self.0.lock().unwrap().lane_states()
    }

    pub fn visible_objects(&self, lower_ms: f64, upper_ms: f64) -> Vec<VisibleObject> {
        self.0.lock().unwrap().visible_objects(lower_ms, upper_ms)
    }

    pub fn is_resolved(&self) -> bool {
        self.0.lock().unwrap().is_resolved()
    }

    pub fn late_inputs(&self) -> u32 {
        self.0.lock().unwrap().late_inputs()
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
        self.0
            .lock()
            .unwrap()
            .record_judgement_events(&events, chart_time_ms);
    }

    pub fn advance_judgement_time(&self, chart_time_ms: f64) {
        self.0.lock().unwrap().advance_judgement_time(chart_time_ms);
    }

    pub fn record_input(&self, lane: u32, phase: Phase, sequence_number: u64, ui_time_ms: f64) {
        self.0
            .lock()
            .unwrap()
            .record_input(lane, phase, sequence_number, ui_time_ms);
    }

    pub fn judgement_presentation(&self, chart_time_ms: f64) -> Option<JudgementPresentation> {
        self.0.lock().unwrap().judgement_presentation(chart_time_ms)
    }

    pub fn lane_brightness(&self, lane: u32, ui_time_ms: f64) -> LaneBrightness {
        self.0.lock().unwrap().lane_brightness(lane, ui_time_ms)
    }

    pub fn judgement_event_batches(&self) -> Vec<JudgementEventBatch> {
        self.0.lock().unwrap().judgement_event_batches().to_vec()
    }

    pub fn latest_lane_input_transition(&self, lane: u32) -> Option<LaneInputTransition> {
        self.0.lock().unwrap().latest_lane_input_transition(lane)
    }
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
