//! The scheduler: generation for play, with its pages going into an attached gameplay session inside Rust.
//!
//! The shell calls `work()` on its inference worker; it runs graphs and appends committed pages to the session given
//! with the last `start`, and returns when play may start (`Ready`) or nothing is left to do (`Done`). While play is
//! far enough ahead it blocks until the next event. The frame loop calls `heard` and returns at once; `stop` ends a
//! blocked or running `work()` after its current graph run.

use crate::generator::{chart_result, clock, core_request, generation_error, panic_error, ChartGenerator, ChartResult, GenerateChartRequest, GeneratorError, Prepared};
use crate::{lock, GameplaySession};
use ensomi_gameplay::{ChartToken, Ruleset, TokenKind};
use ensomi_generate::{Action, Event, Lead, Mode, Options, Scheduler, Source};
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScheduleMode {
    /// The whole chart before play starts: for saving or exporting, and play after.
    WholeChart,
    /// Play while generating: the same chart, a page per head row, play from when the start rule holds.
    AheadOfPlay,
}

/// The lead L = max(settle lead, render lead) + margin that the chart keeps ahead of play.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct LeadPolicy {
    /// The rules of the session, whose widest wait is the settle lead (`Ruleset::settle_lead_ms`).
    pub ruleset: Ruleset,
    /// How far ahead of the heard time the playfield draws notes: the client's scroll time.
    pub render_lead_ms: f64,
    /// Room for the variance of the generation's throughput on this device.
    pub margin_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, uniffi::Enum)]
pub enum SchedulerAction {
    /// Play may start, or resume after the chart fell behind it, at this chart time. From a position other than 0,
    /// objects before it are not in the session.
    Ready { join_ms: f64 },
    /// Nothing more until another `start`.
    Done,
}

enum Message {
    Heard(f64, f64),
    Start(f64, Arc<GameplaySession>),
    Stop,
}

struct Core {
    scheduler: Scheduler,
    session: Option<Arc<GameplaySession>>,
}

// SAFETY: the scheduler is not `Send` for the `Rc`s of its generator and of the clock made here, none of which is
// shared with anything outside it; it is used only under `ChartScheduler::core`, by one thread at a time.
unsafe impl Send for Core {}

/// One chart generated for one song's play; see the module documentation.
#[derive(uniffi::Object)]
pub struct ChartScheduler {
    generator: Arc<ChartGenerator>,
    core: Mutex<Core>,
    messages: Mutex<VecDeque<Message>>,
    wake: Condvar,
}

fn token(token: &ensomi_generate::Token) -> ChartToken {
    let kind = match token.kind {
        ensomi_generate::TokenKind::Tap => TokenKind::Tap,
        ensomi_generate::TokenKind::HoldStart => TokenKind::HoldStart,
        ensomi_generate::TokenKind::HoldEnd => TokenKind::HoldEnd,
    };
    ChartToken { lane: token.lane as u32, time_ms: token.time_ms, kind }
}

impl ChartScheduler {
    fn new(generator: Arc<ChartGenerator>, source: Source, request: GenerateChartRequest, mode: ScheduleMode, lead: LeadPolicy) -> Result<Arc<Self>, GeneratorError> {
        let options = Options { audio_name: request.audio_name.clone(), ..Options::new(clock()) };
        let request = core_request(&request).map_err(generation_error)?;
        let mode = match mode {
            ScheduleMode::WholeChart => Mode::WholeChart,
            ScheduleMode::AheadOfPlay => Mode::AheadOfPlay,
        };
        let lead = Lead { settle_ms: lead.ruleset.settle_lead_ms(), render_ms: lead.render_lead_ms, margin_ms: lead.margin_ms };
        let scheduler = Scheduler::new(generator.models.clone(), source, request, options, mode, lead);
        Ok(Arc::new(Self { generator, core: Mutex::new(Core { scheduler, session: None }), messages: Mutex::new(VecDeque::new()), wake: Condvar::new() }))
    }

    fn send(&self, message: Message) {
        let mut messages = lock(&self.messages);
        // Only the latest heard time matters to a scheduler that has not taken the earlier ones.
        if let (Message::Heard(..), Some(Message::Heard(..))) = (&message, messages.back()) {
            messages.pop_back();
        }
        messages.push_back(message);
        self.wake.notify_one();
    }

    /// Generates until play may start, nothing is left to do, or the scheduler waits for an event (`None`).
    fn run(&self, core: &mut Core) -> Result<Option<SchedulerAction>, GeneratorError> {
        loop {
            for message in lock(&self.messages).drain(..) {
                match message {
                    Message::Heard(chart_ms, wall_ms) => core.scheduler.handle(Event::Heard { chart_ms, wall_ms }),
                    Message::Start(at_ms, session) => {
                        core.session = Some(session);
                        core.scheduler.handle(Event::Start { at_ms });
                    }
                    Message::Stop => core.scheduler.handle(Event::Stop),
                }
            }
            match core.scheduler.next().map_err(generation_error)? {
                Action::Run { graph, inputs } => {
                    let outputs = lock(&self.generator.sessions).run(&graph, inputs).map_err(generation_error)?;
                    core.scheduler.handle(Event::Outputs(outputs));
                }
                Action::Commit(page) => {
                    if let Some(session) = &core.session {
                        let tokens: Vec<ChartToken> = page.tokens.iter().map(token).collect();
                        session.append_page(&tokens, page.coverage_ms, page.end_of_stream).map_err(|e| generation_error(format!("session: {e}")))?;
                    }
                }
                Action::Ready { join_ms } => return Ok(Some(SchedulerAction::Ready { join_ms })),
                Action::Done => return Ok(Some(SchedulerAction::Done)),
                Action::Wait => return Ok(None),
            }
        }
    }
}

#[uniffi::export]
impl ChartScheduler {
    /// From mono PCM at `sample_rate`: the audio stages run first.
    #[uniffi::constructor]
    pub fn from_pcm(generator: Arc<ChartGenerator>, pcm: Vec<f32>, sample_rate: f64, request: GenerateChartRequest, mode: ScheduleMode, lead: LeadPolicy) -> Result<Arc<Self>, GeneratorError> {
        let audio = generator.audio(pcm, sample_rate)?;
        Self::new(generator, Source::Audio(audio), request, mode, lead)
    }

    /// From a stored `Prepared` of the song, without the audio stages.
    #[uniffi::constructor]
    pub fn from_prepared(generator: Arc<ChartGenerator>, prepared: Arc<Prepared>, request: GenerateChartRequest, mode: ScheduleMode, lead: LeadPolicy) -> Result<Arc<Self>, GeneratorError> {
        Self::new(generator, Source::Prepared(prepared.0.clone()), request, mode, lead)
    }

    /// Play is to start at `at_ms`, 0 or a position, judged by `session`, a new `GameplaySession.streaming`: from the
    /// next `work()` on, the chart from the join point goes into it.
    pub fn start(&self, at_ms: f64, session: Arc<GameplaySession>) {
        self.send(Message::Start(at_ms, session));
    }

    /// Each frame during play: the chart time heard and the shell's monotonic clock, in milliseconds.
    pub fn heard(&self, chart_ms: f64, wall_ms: f64) {
        self.send(Message::Heard(chart_ms, wall_ms));
    }

    /// Ends generation; `work()` then returns `Done`.
    pub fn stop(&self) {
        self.send(Message::Stop);
    }

    /// Generates, on the caller's thread, until play may start or nothing is left to do; see the module
    /// documentation. A panic in the core ends it with `GeneratorError::Generation`, a page the session refuses too.
    pub fn work(&self) -> Result<SchedulerAction, GeneratorError> {
        loop {
            let action = {
                let mut core = lock(&self.core);
                catch_unwind(AssertUnwindSafe(|| self.run(&mut core))).map_err(panic_error)??
            };
            if let Some(action) = action {
                return Ok(action);
            }
            // Waiting for play to move on, without holding the scheduler.
            drop(self.wake.wait_while(lock(&self.messages), |m| m.is_empty()).unwrap_or_else(PoisonError::into_inner));
        }
    }

    /// The whole chart, `.osu` and receipt, once generation is done.
    pub fn result(&self) -> Option<ChartResult> {
        lock(&self.core).scheduler.result().map(chart_result)
    }

    /// The song's `Prepared`, to store, once the audio stages have run.
    pub fn prepared(&self) -> Option<Arc<Prepared>> {
        lock(&self.core).scheduler.prepared().map(|p| Arc::new(Prepared(p)))
    }

    /// The generation's throughput so far: committed chart milliseconds per wall millisecond of work.
    pub fn rate(&self) -> Option<f64> {
        lock(&self.core).scheduler.rate()
    }
}
