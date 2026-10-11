#![allow(dead_code)]

use ensomi_gameplay::{Chart, InputEvent, JudgementEvent, Ruleset, Score, Session};

#[derive(Clone, Copy, Debug)]
pub enum Step {
    Input(InputEvent),
    Advance(f64),
}

pub struct Outcome {
    pub events: Vec<JudgementEvent>,
    pub score: Score,
    pub late_inputs: u32,
    pub resolved: bool,
}

pub fn run(chart: &Chart, ruleset: Ruleset, steps: &[Step]) -> Outcome {
    let mut session = Session::new(chart, ruleset).expect("valid chart");
    let mut events = Vec::new();
    for step in steps {
        events.extend(match *step {
            Step::Input(input) => session.input(input),
            Step::Advance(watermark_ms) => session.advance(watermark_ms),
        });
    }
    Outcome {
        events,
        score: session.score(),
        late_inputs: session.late_inputs(),
        resolved: session.is_resolved(),
    }
}

/// The same inputs with their advances replaced by ones every `period_ms`, each before the next input
/// that is later than it, then a flush that resolves everything.
pub fn recut(steps: &[Step], period_ms: f64) -> Vec<Step> {
    let mut out = Vec::new();
    let mut last_ms = 0.0_f64;
    for step in steps {
        if let Step::Input(input) = step {
            let mut watermark_ms = (last_ms / period_ms).floor() * period_ms + period_ms;
            while watermark_ms < input.time_ms {
                out.push(Step::Advance(watermark_ms));
                watermark_ms += period_ms;
            }
            out.push(*step);
            last_ms = last_ms.max(input.time_ms);
        }
    }
    out.push(Step::Advance(f64::INFINITY));
    out
}
