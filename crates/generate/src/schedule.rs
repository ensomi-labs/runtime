//! When to generate and how far ahead, without IO. The [`Scheduler`] drives the pipeline from audio or a stored
//! [`Prepared`], takes the shell's heard time and requests as [`Event`]s, and answers with [`Action`]s: run a graph,
//! append a page to the gameplay session, play may start, wait, done.
//!
//! Play may start at t₀ once the chart is committed through c₀ with c₀ − t₀ ≥ L + max(0, (E − c₀)(p/r − 1)): the
//! lead L ([`Lead`]), the song's end E, the playback rate p from the heard events and the generation's throughput r,
//! committed chart milliseconds per busy wall millisecond since the first page. Then play never overtakes the chart,
//! and a device slower than playback starts later instead of stalling. While playing, the scheduler waits whenever
//! the same rule holds from the heard time.
use crate::export::{chart_tokens, Page, Token, TokenKind};
use crate::generate::{generate_prepared, prepare, Audio, Commit, GenerationResult, Options, Request};
use crate::machine::{Generator, Step, Tensors};
use crate::models::Models;
use crate::prepared::Prepared;
use std::rc::Rc;
use std::sync::Arc;

pub enum Source {
    /// PCM: the scheduler prepares it first.
    Audio(Audio),
    Prepared(Arc<Prepared>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The whole chart in one page at the end; play starts after it.
    WholeChart,
    /// A page per head row, the same chart; play starts once the start rule holds.
    AheadOfPlay,
}

/// The lead L = max(settle, render) + margin that the committed chart keeps ahead of play.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lead {
    /// From the rules: how far past an input the chart must reach for the session to settle it.
    pub settle_ms: f64,
    /// From the client: how far ahead of the heard time the playfield draws notes.
    pub render_ms: f64,
    /// From the client: room for the variance of the throughput on the device.
    pub margin_ms: f64,
}

impl Lead {
    pub fn ms(&self) -> f64 {
        self.settle_ms.max(self.render_ms) + self.margin_ms
    }
}

pub enum Event {
    /// During play: the chart time heard and the shell's monotonic clock, in milliseconds.
    Heard { chart_ms: f64, wall_ms: f64 },
    /// Play is to start at this chart time: 0, or a position such as a seek target. The pages that follow are for a
    /// new session, from the join point on.
    Start { at_ms: f64 },
    /// New controls for the chart from the first uncommitted head row on: only `star` and `controls` may differ from
    /// the running request, and committed pages never change. It is for requests that game events send during play,
    /// with or without Sonalign: a multiplayer attack that changes another player's coming chart, players
    /// coordinating. A pipeline that resumes from a committed prefix will apply them there. R2 cannot yet: it takes a
    /// request until the first page is committed, and the chart is then the one a fresh generation with it makes.
    /// During the audio stages it replaces the held request; after them the chart stages start again from the kept
    /// `Prepared`, so in `WholeChart`, which commits only at the end, a request during the arrangement restarts it.
    Request(Request),
    Stop,
    /// The outputs of the last `Run`.
    Outputs(Tensors),
}

#[derive(Debug)]
pub enum Action {
    Run { graph: String, inputs: Tensors },
    /// For the session's `append_tokens`.
    Commit(Page),
    /// Play may start, or resume after the chart fell behind it, at this chart time.
    Ready { join_ms: f64 },
    /// Nothing to do until the next event.
    Wait,
    /// Nothing more until another `Start`.
    Done,
}

/// The start rule c₀ − t₀ ≥ L + max(0, (E − c₀)(p/r − 1)).
pub(crate) fn can_start(c0: f64, t0: f64, end: f64, lead: f64, p: f64, r: f64) -> bool {
    c0 - t0 >= lead + f64::max(0.0, (end - c0) * (p / r - 1.0))
}

/// The join point: the first time at or after `from` at which no hold is open, that is started before it and ending
/// after it. `None` while the tokens committed through `coverage` cannot tell.
pub(crate) fn join_point(tokens: &[Token], coverage: f64, from: f64) -> Option<f64> {
    // Every hold, with an infinite end while its end has not arrived.
    let mut open: [Option<f64>; 4] = [None; 4];
    let mut holds: Vec<(f64, f64)> = Vec::new();
    for token in tokens {
        match token.kind {
            TokenKind::HoldStart => open[token.lane] = Some(token.time_ms),
            TokenKind::HoldEnd => holds.extend(open[token.lane].take().map(|start| (start, token.time_ms))),
            TokenKind::Tap => {}
        }
    }
    holds.extend(open.iter().flatten().map(|&start| (start, f64::INFINITY)));
    let mut at = from;
    loop {
        // Every hold that starts before `at` has arrived only once the coverage reaches it.
        if at > coverage {
            return None;
        }
        match holds.iter().filter(|&&(start, end)| start < at && at < end).map(|&(_, end)| end).reduce(f64::max) {
            None => return Some(at),
            Some(end) if end.is_infinite() => return None,
            Some(end) => at = end,
        }
    }
}

/// The committed chart and the play it serves, without the pipeline: pages in, pages for the session and `Ready`
/// out, and whether to wait.
pub(crate) struct Stream {
    lead: Lead,
    /// The song's end E, once the audio stages have run.
    pub(crate) end_ms: Option<f64>,
    tokens: Vec<Token>,
    coverage: f64,
    ended: bool,
    /// Coverage and busy milliseconds at the first page and at the latest.
    first: Option<(f64, f64)>,
    latest: (f64, f64),
    /// Where play was asked to start.
    target: Option<f64>,
    join: Option<f64>,
    /// Where play starts or resumes: the join point, or the heard time at which the chart fell behind.
    from: Option<f64>,
    ready: bool,
    /// How much of the chart went to the session since the last start.
    sent: usize,
    sent_coverage: f64,
    /// The first and the latest heard event since play last started.
    heard: Option<((f64, f64), (f64, f64))>,
}

impl Stream {
    pub(crate) fn new(lead: Lead) -> Stream {
        Stream { lead, end_ms: None, tokens: Vec::new(), coverage: f64::NEG_INFINITY, ended: false, first: None, latest: (f64::NEG_INFINITY, 0.0), target: None, join: None, from: None, ready: false, sent: 0, sent_coverage: f64::NEG_INFINITY, heard: None }
    }

    /// A page the pipeline committed after `busy_ms` of work.
    pub(crate) fn commit(&mut self, page: Page, busy_ms: f64) {
        self.first.get_or_insert((page.coverage_ms, busy_ms));
        self.latest = (page.coverage_ms, busy_ms);
        self.tokens.extend(page.tokens);
        self.coverage = page.coverage_ms;
        self.ended = page.end_of_stream;
    }

    pub(crate) fn start(&mut self, at_ms: f64) {
        (self.target, self.join, self.from, self.ready) = (Some(at_ms), None, None, false);
        (self.sent, self.sent_coverage, self.heard) = (0, f64::NEG_INFINITY, None);
    }

    pub(crate) fn heard(&mut self, chart_ms: f64, wall_ms: f64) {
        let first = self.heard.map_or((chart_ms, wall_ms), |(first, _)| first);
        self.heard = Some((first, (chart_ms, wall_ms)));
        // The session cannot settle what is heard now: play has to wait for the chart, then resume here.
        if self.ready && !self.ended && chart_ms + self.lead.settle_ms > self.coverage {
            (self.ready, self.from) = (false, Some(chart_ms));
        }
    }

    /// Committed chart milliseconds per busy millisecond since the first page.
    pub(crate) fn rate(&self) -> Option<f64> {
        let (c0, b0) = self.first?;
        let (c, b) = self.latest;
        (b > b0).then(|| (c - c0) / (b - b0))
    }

    /// Chart milliseconds heard per wall millisecond since play started; 1 before two heard events.
    fn playback_rate(&self) -> f64 {
        match self.heard {
            Some(((c0, w0), (c, w))) if w > w0 => (c - c0) / (w - w0),
            _ => 1.0,
        }
    }

    /// Whether play from `t0` stays behind the chart to the end.
    fn safe_from(&self, t0: f64) -> bool {
        self.ended
            || match (self.rate(), self.end_ms) {
                (Some(r), Some(end)) if r > 0.0 => can_start(self.coverage, t0, end, self.lead.ms(), self.playback_rate(), r),
                _ => false,
            }
    }

    /// Whether the pipeline may rest: play is under way and stays behind the chart from where it is heard.
    pub(crate) fn idle(&self) -> bool {
        self.ready && !self.ended && self.heard.is_some_and(|(_, (chart_ms, _))| self.safe_from(chart_ms))
    }

    /// The next page for the session, then `Ready`, once the join point is known.
    pub(crate) fn next(&mut self) -> Option<Action> {
        let target = self.target?;
        if self.join.is_none() {
            let coverage = if self.ended { f64::INFINITY } else { self.coverage };
            self.join = if target <= 0.0 { Some(target) } else { join_point(&self.tokens, coverage, target + self.lead.ms()) };
            self.from = self.join;
        }
        let join = self.join?;
        if self.sent < self.tokens.len() || self.sent_coverage < self.coverage {
            // Objects before the join point are not played: every hold that starts before it has ended by it.
            let tokens = self.tokens[self.sent..].iter().filter(|t| if t.kind == TokenKind::HoldEnd { t.time_ms > join } else { t.time_ms >= join }).copied().collect();
            (self.sent, self.sent_coverage) = (self.tokens.len(), self.coverage);
            return Some(Action::Commit(Page { tokens, coverage_ms: self.coverage, end_of_stream: self.ended }));
        }
        let from = self.from?;
        if !self.ready && self.safe_from(from) {
            (self.ready, self.heard) = (true, None);
            return Some(Action::Ready { join_ms: from });
        }
        None
    }
}

enum Work {
    Prepare(Generator<Prepared>),
    Chart(Generator),
    Finished,
    Stopped,
}

/// Drives one chart for one song. Call [`Scheduler::next`] and act on what it returns; pass the shell's events to
/// [`Scheduler::handle`], the outputs of a `Run` before the next call.
pub struct Scheduler {
    models: Arc<Models>,
    mode: Mode,
    work: Work,
    /// The request the chart is generated for, held while the audio stages run.
    request: Request,
    /// The chart generator's options; the first chart generator takes `random` and `trace`.
    options: Options,
    clock: Rc<dyn Fn() -> f64>,
    outputs: Option<Tensors>,
    /// A `Run` is out.
    running: bool,
    /// The `Run` that is out is for a chart generator that a request replaced.
    stale: bool,
    /// Milliseconds between calls to `next`, except those after a `Ready` or a `Wait` for play.
    busy: f64,
    last: Option<f64>,
    prepared: Option<Arc<Prepared>>,
    result: Option<GenerationResult>,
    stream: Stream,
}

impl Scheduler {
    /// `options.commit` is set by the mode; `options.clock` also times the throughput, and `options.trace` sees the
    /// stages after the audio stages. `options.random` and `options.trace` go to the first chart generator only:
    /// one that a request starts again draws from the seeded generator, without a trace.
    pub fn new(models: Arc<Models>, source: Source, request: Request, mut options: Options, mode: Mode, lead: Lead) -> Scheduler {
        options.commit = if mode == Mode::AheadOfPlay { Commit::HeadRow } else { Commit::AtEnd };
        let clock = options.clock.clone();
        let mut scheduler = Scheduler { models, mode, work: Work::Finished, request, options, clock, outputs: None, running: false, stale: false, busy: 0.0, last: None, prepared: None, result: None, stream: Stream::new(lead) };
        match source {
            Source::Audio(audio) => scheduler.work = Work::Prepare(prepare(scheduler.models.clone(), audio, Options::new(scheduler.clock.clone()))),
            Source::Prepared(prepared) => scheduler.chart(prepared),
        }
        scheduler
    }

    /// Only a `Request` is refused: one that changes more than `star` and `controls`, or one that comes once a page
    /// is committed, which R2 cannot resume from. The generation is then unchanged.
    pub fn handle(&mut self, event: Event) -> Result<(), String> {
        match event {
            Event::Heard { chart_ms, wall_ms } => self.stream.heard(chart_ms, wall_ms),
            Event::Start { at_ms } => self.stream.start(at_ms),
            Event::Request(request) => return self.request(request),
            Event::Stop => self.work = Work::Stopped,
            Event::Outputs(outputs) => {
                // Those of a run for a chart generator that a request replaced are dropped.
                if !std::mem::take(&mut self.stale) {
                    self.outputs = Some(outputs);
                }
                self.running = false;
            }
        }
        Ok(())
    }

    /// What to do now. Not an iterator: `Wait` and `Done` end nothing, and events change what follows.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Action, String> {
        let now = (self.clock)();
        self.busy += self.last.map_or(0.0, |last| now - last);
        self.last = Some(now);
        loop {
            if matches!(self.work, Work::Stopped) {
                return Ok(Action::Done);
            }
            if let Some(action) = self.stream.next() {
                // The shell acts on `Ready` before it asks for more; that time is not generation's.
                if matches!(action, Action::Ready { .. }) {
                    self.last = None;
                }
                return Ok(action);
            }
            if self.running {
                return Ok(Action::Wait);
            }
            if self.stream.idle() {
                self.last = None;
                return Ok(Action::Wait);
            }
            let outputs = self.outputs.take();
            match &mut self.work {
                Work::Prepare(generator) => match generator.step(outputs) {
                    Err(e) => return Err(self.fail(e)),
                    Ok(Step::Run { graph, inputs }) => return Ok(self.run(graph, inputs)),
                    Ok(Step::Done(prepared)) => self.chart(Arc::new(*prepared)),
                    Ok(_) => {}
                },
                Work::Chart(generator) => match generator.step(outputs) {
                    Err(e) => return Err(self.fail(e)),
                    Ok(Step::Run { graph, inputs }) => return Ok(self.run(graph, inputs)),
                    Ok(Step::Progress(_)) => {}
                    Ok(Step::Commit(page)) => self.stream.commit(page, self.busy),
                    Ok(Step::Done(result)) => self.finished(*result),
                },
                Work::Finished | Work::Stopped => return Ok(Action::Done),
            }
        }
    }

    /// While a graph runs, does a slice of the work that needs no outputs ([`Generator::work`]).
    pub fn work(&mut self) -> bool {
        match &mut self.work {
            Work::Prepare(generator) => generator.work(),
            Work::Chart(generator) => generator.work(),
            _ => false,
        }
    }

    /// The finished chart, `.osu` and receipt, once generation is done.
    pub fn result(&self) -> Option<&GenerationResult> {
        self.result.as_ref()
    }

    /// The audio stages' results, to store, once they have run.
    pub fn prepared(&self) -> Option<Arc<Prepared>> {
        self.prepared.clone()
    }

    /// The throughput r measured so far: committed chart milliseconds per busy wall millisecond.
    pub fn rate(&self) -> Option<f64> {
        self.stream.rate()
    }

    /// Takes a request until the first page is committed: it replaces the request held while the audio stages
    /// run, and starts the chart stages again after them. R2 cannot resume from a committed prefix, so it refuses
    /// one later.
    fn request(&mut self, request: Request) -> Result<(), String> {
        let running = &self.request;
        if (&request.audio, request.seed, request.seed_is_bigint, &request.title, &request.artist) != (&running.audio, running.seed, running.seed_is_bigint, &running.title, &running.artist) {
            return Err("unsupported_control: a request during generation may change only star and controls".to_string());
        }
        match (&self.work, self.prepared.clone()) {
            (Work::Prepare(_), _) => self.request = request,
            (Work::Chart(_), Some(prepared)) if self.stream.first.is_none() => {
                self.request = request;
                (self.outputs, self.stale) = (None, self.running);
                self.chart(prepared);
            }
            _ => return Err("unsupported_control: R2 cannot apply a request once a page is committed or the generation has ended; the generation is unchanged".to_string()),
        }
        Ok(())
    }

    /// Starts the chart stages from `prepared` for the request.
    fn chart(&mut self, prepared: Arc<Prepared>) {
        self.stream.end_ms = Some(prepared.song_ms);
        self.prepared = Some(prepared.clone());
        let o = &mut self.options;
        let options = Options { random: o.random.take(), trace: o.trace.take(), setting: o.setting.clone(), audio_name: o.audio_name.clone(), clock: o.clock.clone(), commit: o.commit };
        self.work = Work::Chart(generate_prepared(self.models.clone(), prepared, self.request.clone(), options));
    }

    fn run(&mut self, graph: String, inputs: Tensors) -> Action {
        self.running = true;
        Action::Run { graph, inputs }
    }

    /// A generator that failed is not stepped again.
    fn fail(&mut self, error: String) -> String {
        self.work = Work::Stopped;
        error
    }

    fn finished(&mut self, result: GenerationResult) {
        if self.mode == Mode::WholeChart {
            self.stream.commit(Page { tokens: chart_tokens(&result.chart), coverage_ms: result.chart.duration_ms, end_of_stream: true }, self.busy);
        }
        self.result = Some(result);
        self.work = Work::Finished;
    }
}
