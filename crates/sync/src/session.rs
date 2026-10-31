//! One listening session: microphone blocks in, the alignment's state and position out.
//!
//! Around Sonalign this is the Swift app's `LiveAmbientSyncRuntime`: the last five seconds of frames, searched every
//! 100 ms of audio, and a final lock when the state is locked in the final phase. The one departure is the clock. The
//! Swift path trusts Core Audio's timestamps; an Android read can return tens of milliseconds late and in bursts, so
//! capture times here are filtered rather than used one by one ([`Clock`]).

use crate::{Error, Stream, HOP_MS};
use sonalign::{Engine, FeatureFrame, FeatureWindow, LockPhase, State, WithholdReason};
use std::collections::VecDeque;

/// The query Sonalign sees. The Swift path's `finalLockTargetDurationMS`.
const QUERY_MS: f64 = 5_000.0;
/// The audio between searches. Its `matchingProcessIntervalMS`.
const PROCESS_INTERVAL_MS: f64 = 100.0;
/// Audio missing beyond this breaks continuity. Its two hops, `maximumGapHops`.
const MAXIMUM_GAP_MS: f64 = 2.0 * HOP_MS;
/// How long blocks must stay that late for the gap to be audio lost. A read that stalled for less catches up within
/// this time and loses nothing, and losing the query to a false break costs more than a short gap does.
const GAP_PERSISTENCE_MS: f64 = 500.0;
/// How far back the clock looks for its earliest arrival.
const CLOCK_WINDOW_MS: f64 = QUERY_MS;

/// A position in the song at a capture time: `reference_time_ms` was playing at `capture_time_ms` on the caller's clock.
/// Now, on that clock, the song is at `reference_time_ms + (now - capture_time_ms)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub reference_time_ms: f64,
    pub capture_time_ms: f64,
}

/// What the session knows after a block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot {
    pub state: State,
    pub phase: LockPhase,
    /// Provisional before the lock is final: use [`Snapshot::is_final`] to act on it.
    pub position: Option<Position>,
    /// Temporal spectral agreement, not a probability. Zero with no position.
    pub confidence: f64,
    pub withhold_reason: Option<WithholdReason>,
}

impl Snapshot {
    /// The lock to act on, as the Swift path has it: locked, in the final phase.
    pub fn is_final(&self) -> bool {
        self.state == State::Locked && self.phase == LockPhase::Final
    }
}

/// Maps the sample-counted timeline Sonalign needs to the caller's clock.
///
/// Sonalign refuses irregular frame times, so frames are timed by counting samples from the segment's start, and
/// `offset_ms` turns that time into the caller's. The caller's stamps may be late, and never early: a block can be
/// delivered after it was captured, not before. So the mapping is the earliest stamp of the last seconds, against the
/// sample count, which also recovers from a late first block. A stamp late for one block is jitter; stamps that
/// stay late mean samples never arrived.
#[derive(Debug)]
struct Clock {
    origin_ms: f64,
    sample_rate: f64,
    samples: u64,
    /// `(time, lateness)`, with lateness rising from the front, which holds the minimum of the window.
    earliest: VecDeque<(f64, f64)>,
    late_since_ms: Option<f64>,
}

impl Clock {
    fn new(origin_ms: f64, sample_rate: f64) -> Self {
        Self { origin_ms, sample_rate, samples: 0, earliest: VecDeque::new(), late_since_ms: None }
    }

    /// Takes a block's stamp. False if the block does not continue the timeline.
    fn observe(&mut self, capture_time_ms: f64, samples: usize) -> bool {
        let time_ms = self.samples as f64 / self.sample_rate * 1_000.0;
        let lateness = capture_time_ms - self.origin_ms - time_ms;
        if let Some(&(_, floor)) = self.earliest.front()
            && lateness - floor > MAXIMUM_GAP_MS
        {
            if time_ms - *self.late_since_ms.get_or_insert(time_ms) >= GAP_PERSISTENCE_MS {
                return false;
            }
        } else {
            self.late_since_ms = None;
        }
        while self.earliest.back().is_some_and(|&(_, later)| later >= lateness) {
            self.earliest.pop_back();
        }
        self.earliest.push_back((time_ms, lateness));
        while self.earliest.front().is_some_and(|&(time, _)| time < time_ms - CLOCK_WINDOW_MS) {
            self.earliest.pop_front();
        }
        self.samples += samples as u64;
        true
    }

    /// From the sample-counted timeline to the caller's clock.
    fn offset_ms(&self) -> f64 {
        self.earliest.front().map_or(0.0, |&(_, lateness)| lateness)
    }
}

/// Aligns one stream of microphone blocks to a [`crate::Reference`]. Calls are serialized by `&mut self`.
pub struct Session {
    engine: Engine,
    sample_rate: f64,
    stream: Stream,
    clock: Option<Clock>,
    /// The query: frames since the last break, no older than [`QUERY_MS`].
    frames: VecDeque<FeatureFrame>,
    started_ms: f64,
    last_processed_ms: Option<f64>,
    /// The newest frame time ever, which survives a break for Sonalign to see the timeline jump.
    latest_ms: f64,
    snapshot: Snapshot,
}

impl Session {
    pub(crate) fn new(engine: Engine, sample_rate: f64) -> Result<Self, Error> {
        Ok(Self {
            engine,
            sample_rate,
            stream: Stream::new(sample_rate, 0.0)?,
            clock: None,
            frames: VecDeque::new(),
            started_ms: 0.0,
            last_processed_ms: None,
            latest_ms: f64::NEG_INFINITY,
            snapshot: Snapshot { state: State::Ready, phase: LockPhase::None, position: None, confidence: 0.0, withhold_reason: None },
        })
    }

    /// Takes mono PCM captured from `capture_time_ms` on: the time of the block's first sample on the caller's monotonic
    /// clock, in any origin. Blocks of any length. A refused block changes nothing.
    pub fn push(&mut self, samples: &[f32], capture_time_ms: f64) -> Result<Snapshot, Error> {
        if !capture_time_ms.is_finite() || samples.iter().any(|sample| !sample.is_finite()) {
            return Err(Error::NotFinite);
        }
        if samples.is_empty() {
            return Ok(self.snapshot);
        }
        let continues = self.clock.as_mut().is_some_and(|clock| clock.observe(capture_time_ms, samples.len()));
        if !continues {
            let first = self.clock.is_none();
            // Sonalign drops its track when it sees the timeline jump by more than two hops.
            let origin_ms = capture_time_ms.max(self.latest_ms + 3.0 * HOP_MS);
            self.stream.restart(origin_ms);
            self.frames.clear();
            let mut clock = Clock::new(origin_ms, self.sample_rate);
            clock.observe(capture_time_ms, samples.len());
            self.clock = Some(clock);
            if first {
                self.started_ms = origin_ms;
            } else {
                let locked_before = matches!(self.snapshot.phase, LockPhase::Confirmed | LockPhase::Final);
                self.snapshot.state = if locked_before { State::Lost } else { State::Listening };
                self.snapshot.withhold_reason = Some(WithholdReason::LostSignal);
                self.snapshot.position = None;
                self.snapshot.confidence = 0.0;
            }
        }
        self.stream.push(samples, &mut self.frames);
        let Some(latest_ms) = self.frames.back().map(|frame| frame.recorded_time_ms) else {
            return Ok(self.snapshot);
        };
        self.latest_ms = latest_ms;
        while self.frames.front().is_some_and(|frame| frame.recorded_time_ms < latest_ms - QUERY_MS) {
            self.frames.pop_front();
        }
        if self.last_processed_ms.is_none_or(|last| latest_ms < last || latest_ms - last >= PROCESS_INTERVAL_MS) {
            self.last_processed_ms = Some(latest_ms);
            let window = FeatureWindow::new(self.frames.iter().cloned().collect()).map_err(|error| Error::Features(error.0))?;
            let offset_ms = self.clock.as_ref().map_or(0.0, Clock::offset_ms);
            let result = self.engine.process(&window, latest_ms - self.started_ms);
            self.snapshot = Snapshot {
                state: result.state,
                phase: result.phase,
                position: result.estimate.map(|estimate| Position {
                    reference_time_ms: estimate.reference_time_ms,
                    capture_time_ms: estimate.query_endpoint_recorded_time_ms + offset_ms,
                }),
                confidence: result.confidence,
                withhold_reason: result.withhold_reason,
            };
        }
        Ok(self.snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clock_with_exact_stamps_has_no_offset() {
        let mut clock = Clock::new(1_000.0, 48_000.0);
        for block in 0..100 {
            assert!(clock.observe(1_000.0 + block as f64 * 20.0, 960));
        }
        assert!(clock.offset_ms().abs() < 1e-9);
    }

    #[test]
    fn late_stamps_do_not_move_the_clock() {
        let mut clock = Clock::new(1_000.0, 48_000.0);
        for block in 0..100 {
            // Stamped when a 60 ms burst was delivered: lateness 40, 20 or 0 ms.
            let late = [40.0, 20.0, 0.0][block % 3];
            assert!(clock.observe(1_000.0 + block as f64 * 20.0 + late, 960));
        }
        assert!(clock.offset_ms().abs() < 1e-9);
    }

    #[test]
    fn a_late_first_block_is_recovered_from() {
        // The first block was captured at 1000 and stamped at 1050, which became the origin.
        let mut clock = Clock::new(1_050.0, 48_000.0);
        assert!(clock.observe(1_050.0, 960));
        for block in 1..50 {
            assert!(clock.observe(1_000.0 + block as f64 * 20.0, 960));
        }
        assert!((clock.offset_ms() + 50.0).abs() < 1e-9, "{}", clock.offset_ms());
    }

    #[test]
    fn a_stamp_that_stays_late_breaks_continuity() {
        let mut clock = Clock::new(0.0, 48_000.0);
        for block in 0..50 {
            assert!(clock.observe(block as f64 * 20.0, 960));
        }
        // 30 ms of samples went missing: each stamp now runs 30 ms past the count, and persistence is 500 ms.
        for block in 50..75 {
            assert!(clock.observe(block as f64 * 20.0 + 30.0, 960), "{block}");
        }
        assert!(!clock.observe(75.0 * 20.0 + 30.0, 960));
    }

    #[test]
    fn one_late_stamp_does_not_break_continuity() {
        let mut clock = Clock::new(0.0, 48_000.0);
        for block in 0..200 {
            let late = if block % 7 == 0 { 150.0 } else { 0.0 };
            assert!(clock.observe(block as f64 * 20.0 + late, 960));
        }
    }

    #[test]
    fn a_stalled_thread_catches_up_without_a_break() {
        let mut clock = Clock::new(0.0, 48_000.0);
        for block in 0..50 {
            assert!(clock.observe(block as f64 * 20.0, 960));
        }
        // A 400 ms stall: the next blocks wait in the driver and are read at once, stamped when read.
        for (i, block) in (50..70).enumerate() {
            assert!(clock.observe(1_400.0 + i as f64 * 0.1, 960), "{block}");
        }
        assert!(clock.offset_ms().abs() < 1e-9);
    }

    #[test]
    fn the_clock_follows_a_rate_difference() {
        // A microphone 100 ppm slow: its count falls behind the caller's clock, and the old floor ages out.
        let mut clock = Clock::new(0.0, 48_000.0);
        for block in 0..3_000 {
            assert!(clock.observe(block as f64 * 20.0 * 1.0001, 960));
        }
        let expected = 2_999.0 * 20.0 * 0.0001 - CLOCK_WINDOW_MS * 0.0001;
        assert!((clock.offset_ms() - expected).abs() < 0.1, "{}", clock.offset_ms());
    }
}
