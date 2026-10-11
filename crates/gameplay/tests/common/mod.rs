#![allow(dead_code)]

use ensomi_gameplay::{
    Chart, ChartObject, ChartToken, InputEvent, JudgementEvent, Ruleset, Score, Session,
};

/// What a page delivers: whole objects (EnsomiProtocol) or tokens (the macOS app's streams).
#[derive(Clone, Debug)]
pub enum Delivery {
    Objects(Vec<ChartObject>),
    Tokens(Vec<ChartToken>),
}

#[derive(Clone, Debug)]
pub struct Page {
    pub delivery: Delivery,
    pub coverage_ms: f64,
    pub end_of_stream: bool,
}

#[derive(Clone, Debug)]
pub enum Step {
    Input(InputEvent),
    Advance(f64),
    Page(Page),
    /// Reads `Session::decided_through_ms` into `Outcome::decided_through`.
    DecidedThrough,
}

pub struct Outcome {
    pub events: Vec<JudgementEvent>,
    /// Per event, the index of the step that returned it.
    pub event_steps: Vec<usize>,
    /// Per `DecidedThrough` step, what the session reported.
    pub decided_through: Vec<f64>,
    pub score: Score,
    pub late_inputs: u32,
    pub resolved: bool,
}

pub fn complete(chart: &Chart, ruleset: Ruleset) -> Session {
    Session::new(chart, ruleset).expect("valid chart")
}

/// Runs the steps, checking that no event is decided beyond the coverage.
pub fn run(mut session: Session, steps: &[Step]) -> Outcome {
    let mut outcome = Outcome {
        events: Vec::new(),
        event_steps: Vec::new(),
        decided_through: Vec::new(),
        score: session.score(),
        late_inputs: 0,
        resolved: false,
    };
    for (index, step) in steps.iter().enumerate() {
        let events = match step {
            Step::Input(input) => session.input(*input),
            Step::Advance(watermark_ms) => session.advance(*watermark_ms),
            Step::Page(page) => match &page.delivery {
                Delivery::Objects(objects) => {
                    session.append(objects, page.coverage_ms, page.end_of_stream)
                }
                Delivery::Tokens(tokens) => {
                    session.append_tokens(tokens, page.coverage_ms, page.end_of_stream)
                }
            }
            .expect("valid page"),
            Step::DecidedThrough => {
                outcome.decided_through.push(session.decided_through_ms());
                Vec::new()
            }
        };
        for event in &events {
            assert!(
                event.time_ms <= session.coverage_ms(),
                "step {index}: {event:?} decided beyond coverage {}",
                session.coverage_ms()
            );
        }
        outcome.event_steps.extend(events.iter().map(|_| index));
        outcome.events.extend(events);
    }
    outcome.score = session.score();
    outcome.late_inputs = session.late_inputs();
    outcome.resolved = session.is_resolved();
    outcome
}

/// The same inputs and pages with the advances replaced by ones every `period_ms`, each before the next
/// input that is later than it, then a flush that resolves everything.
pub fn recut(steps: &[Step], period_ms: f64) -> Vec<Step> {
    let mut out = Vec::new();
    let mut last_ms = 0.0_f64;
    for step in steps {
        match step {
            Step::Input(input) => {
                let mut watermark_ms = (last_ms / period_ms).floor() * period_ms + period_ms;
                while watermark_ms < input.time_ms {
                    out.push(Step::Advance(watermark_ms));
                    watermark_ms += period_ms;
                }
                out.push(step.clone());
                last_ms = last_ms.max(input.time_ms);
            }
            Step::Page(_) => out.push(step.clone()),
            Step::Advance(_) | Step::DecidedThrough => {}
        }
    }
    out.push(Step::Advance(f64::INFINITY));
    out
}

/// The same inputs and advances with every page's delivery in one page, before them, that ends the
/// stream.
pub fn all_at_once(steps: &[Step]) -> Vec<Step> {
    let mut objects = Vec::new();
    let mut tokens = Vec::new();
    let mut rest = Vec::new();
    for step in steps {
        match step {
            Step::Page(page) => match &page.delivery {
                Delivery::Objects(page_objects) => objects.extend_from_slice(page_objects),
                Delivery::Tokens(page_tokens) => tokens.extend_from_slice(page_tokens),
            },
            Step::Input(_) | Step::Advance(_) => rest.push(step.clone()),
            Step::DecidedThrough => {}
        }
    }
    assert!(
        objects.is_empty() || tokens.is_empty(),
        "a stream delivers objects or tokens"
    );
    let delivery = if tokens.is_empty() {
        Delivery::Objects(objects)
    } else {
        Delivery::Tokens(tokens)
    };
    let mut out = vec![Step::Page(Page {
        delivery,
        coverage_ms: f64::INFINITY,
        end_of_stream: true,
    })];
    out.extend(rest);
    out
}
