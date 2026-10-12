//! Presentation state for judgement popups and lane lighting, driven by the session's events and by
//! key transitions. Pure functions of what was recorded and the time asked about.

use crate::{Judgement, JudgementEvent, LANES, Phase};

/// The events of one `input` or `advance` call that produced any, kept for 600 ms.
#[derive(Clone, Debug, PartialEq)]
pub struct JudgementEventBatch {
    pub id: u64,
    pub chart_time_ms: f64,
    pub events: Vec<JudgementEvent>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JudgementPresentation {
    pub event: JudgementEvent,
    pub opacity: f64,
    pub scale: f64,
    pub vertical_offset: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneInputTransition {
    pub lane: u32,
    pub phase: Phase,
    pub sequence_number: u64,
    pub ui_time_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneBrightness {
    pub receptor: f64,
    pub lane: f64,
}

const ZERO_BRIGHTNESS: LaneBrightness = LaneBrightness {
    receptor: 0.0,
    lane: 0.0,
};

const BATCH_RETENTION_MS: f64 = 600.0;
const PRESS_PEAK_RECEPTOR: f64 = 0.52;
const PRESS_PEAK_LANE: f64 = 0.13;
const PRESS_HOLD_RECEPTOR: f64 = 0.24;
const PRESS_HOLD_LANE: f64 = 0.05;
const PRESS_FALLOFF_MS: f64 = 85.0;
const RELEASE_FADE_MS: f64 = 110.0;

#[derive(Clone, Debug, PartialEq)]
struct ActiveJudgement {
    event: JudgementEvent,
    start_chart_time_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LaneFeedback {
    Pressed(LaneInputTransition),
    Released(LaneInputTransition, LaneBrightness),
}

#[derive(Clone, Debug, Default)]
pub struct GameplayFeedback {
    batches: Vec<JudgementEventBatch>,
    transitions: [Option<LaneInputTransition>; LANES],
    next_batch_id: u64,
    active: Option<ActiveJudgement>,
    lanes: [Option<LaneFeedback>; LANES],
}

impl GameplayFeedback {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn judgement_event_batches(&self) -> &[JudgementEventBatch] {
        &self.batches
    }

    /// The latest key transition recorded for a lane.
    pub fn latest_lane_input_transition(&self, lane: u32) -> Option<LaneInputTransition> {
        self.transitions.get(lane as usize).copied().flatten()
    }

    /// Records the events one call produced, at the chart time of that call. The popup shows the most
    /// severe event of the batch, the newest among equals.
    pub fn record_judgement_events(&mut self, events: &[JudgementEvent], chart_time_ms: f64) {
        self.advance_judgement_time(chart_time_ms);
        let Some(candidate) = events.iter().reduce(|selected, event| {
            let (event_severity, selected_severity) = (
                severity(event.tier.judgement()),
                severity(selected.tier.judgement()),
            );
            if event_severity > selected_severity
                || (event_severity == selected_severity && event.id >= selected.id)
            {
                event
            } else {
                selected
            }
        }) else {
            return;
        };
        self.batches.push(JudgementEventBatch {
            id: self.next_batch_id,
            chart_time_ms,
            events: events.to_vec(),
        });
        self.next_batch_id += 1;

        let replace = match &self.active {
            Some(current) if current.is_visible(chart_time_ms) => {
                severity(candidate.tier.judgement()) >= severity(current.event.tier.judgement())
                    || chart_time_ms >= current.protection_end_ms()
            }
            _ => true,
        };
        if replace {
            self.active = Some(ActiveJudgement {
                event: candidate.clone(),
                start_chart_time_ms: chart_time_ms,
            });
        }
    }

    pub fn advance_judgement_time(&mut self, chart_time_ms: f64) {
        self.batches
            .retain(|batch| chart_time_ms - batch.chart_time_ms <= BATCH_RETENTION_MS);
        if self
            .active
            .as_ref()
            .is_some_and(|active| !active.is_visible(chart_time_ms))
        {
            self.active = None;
        }
    }

    /// Records a key transition. `sequence_number` orders transitions of a lane: one not above the latest
    /// recorded is ignored.
    pub fn record_input(&mut self, lane: u32, phase: Phase, sequence_number: u64, ui_time_ms: f64) {
        let slot = lane as usize;
        if slot >= LANES {
            return;
        }
        if self.transitions[slot].is_some_and(|latest| sequence_number <= latest.sequence_number) {
            return;
        }
        let transition = LaneInputTransition {
            lane,
            phase,
            sequence_number,
            ui_time_ms,
        };
        let current = self.brightness(slot, ui_time_ms);
        self.transitions[slot] = Some(transition);
        self.lanes[slot] = Some(match phase {
            Phase::Press => LaneFeedback::Pressed(transition),
            Phase::Release => LaneFeedback::Released(transition, current),
        });
    }

    pub fn judgement_presentation(&self, chart_time_ms: f64) -> Option<JudgementPresentation> {
        let active = self
            .active
            .as_ref()
            .filter(|active| active.is_visible(chart_time_ms))?;
        let p = Parameters::new(active.event.tier.judgement());
        let age_ms = (chart_time_ms - active.start_chart_time_ms).max(0.0);
        let fade_progress = progress(age_ms - p.fade_start_ms, p.end_ms - p.fade_start_ms);
        let scale = if age_ms < p.pulse_ms {
            interpolate(
                p.start_scale,
                1.0,
                smooth_step(progress(age_ms, p.pulse_ms)),
            )
        } else if fade_progress > 0.0 {
            interpolate(1.0, p.end_scale, smooth_step(fade_progress))
        } else {
            1.0
        };
        let vertical_offset = if p.end_vertical_offset == 0.0 {
            0.0
        } else {
            interpolate(
                0.0,
                p.end_vertical_offset,
                smooth_step(progress(age_ms, p.end_ms)),
            )
        };
        Some(JudgementPresentation {
            event: active.event.clone(),
            opacity: 1.0 - smooth_step(fade_progress),
            scale,
            vertical_offset,
        })
    }

    pub fn lane_brightness(&self, lane: u32, ui_time_ms: f64) -> LaneBrightness {
        if (lane as usize) < LANES {
            self.brightness(lane as usize, ui_time_ms)
        } else {
            ZERO_BRIGHTNESS
        }
    }

    fn brightness(&self, lane: usize, ui_time_ms: f64) -> LaneBrightness {
        match self.lanes[lane] {
            None => ZERO_BRIGHTNESS,
            Some(LaneFeedback::Pressed(transition)) => {
                let age_ms = (ui_time_ms - transition.ui_time_ms).max(0.0);
                let fall = smooth_step(progress(age_ms, PRESS_FALLOFF_MS));
                LaneBrightness {
                    receptor: interpolate(PRESS_PEAK_RECEPTOR, PRESS_HOLD_RECEPTOR, fall),
                    lane: interpolate(PRESS_PEAK_LANE, PRESS_HOLD_LANE, fall),
                }
            }
            Some(LaneFeedback::Released(transition, start)) => {
                let age_ms = (ui_time_ms - transition.ui_time_ms).max(0.0);
                if age_ms >= RELEASE_FADE_MS {
                    return ZERO_BRIGHTNESS;
                }
                let remaining = 1.0 - smooth_step(progress(age_ms, RELEASE_FADE_MS));
                LaneBrightness {
                    receptor: start.receptor * remaining,
                    lane: start.lane * remaining,
                }
            }
        }
    }
}

impl ActiveJudgement {
    fn is_visible(&self, chart_time_ms: f64) -> bool {
        chart_time_ms - self.start_chart_time_ms
            < Parameters::new(self.event.tier.judgement()).end_ms
    }

    fn protection_end_ms(&self) -> f64 {
        self.start_chart_time_ms + Parameters::new(self.event.tier.judgement()).protection_ms
    }
}

struct Parameters {
    pulse_ms: f64,
    fade_start_ms: f64,
    end_ms: f64,
    protection_ms: f64,
    start_scale: f64,
    end_scale: f64,
    end_vertical_offset: f64,
}

impl Parameters {
    fn new(judgement: Judgement) -> Self {
        match judgement {
            Judgement::Perfect => Self {
                pulse_ms: 60.0,
                fade_start_ms: 180.0,
                end_ms: 280.0,
                protection_ms: 0.0,
                start_scale: 0.90,
                end_scale: 0.98,
                end_vertical_offset: 0.0,
            },
            Judgement::Good => Self {
                pulse_ms: 70.0,
                fade_start_ms: 200.0,
                end_ms: 320.0,
                protection_ms: 80.0,
                start_scale: 0.90,
                end_scale: 0.98,
                end_vertical_offset: 0.0,
            },
            Judgement::Miss => Self {
                pulse_ms: 70.0,
                fade_start_ms: 260.0,
                end_ms: 400.0,
                protection_ms: 180.0,
                start_scale: 1.08,
                end_scale: 0.98,
                end_vertical_offset: 6.0,
            },
        }
    }
}

fn severity(judgement: Judgement) -> u8 {
    match judgement {
        Judgement::Perfect => 1,
        Judgement::Good => 2,
        Judgement::Miss => 3,
    }
}

fn progress(elapsed_ms: f64, duration_ms: f64) -> f64 {
    if duration_ms > 0.0 {
        (elapsed_ms / duration_ms).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn smooth_step(progress: f64) -> f64 {
    let clamped = progress.clamp(0.0, 1.0);
    clamped * clamped * (3.0 - 2.0 * clamped)
}

fn interpolate(from: f64, to: f64, progress: f64) -> f64 {
    from + (to - from) * progress.clamp(0.0, 1.0)
}
