//! Live audio sync: where the sound a microphone hears lies in a known recording.
//!
//! [`Reference`] turns a whole decoded song into the index [Sonalign](https://github.com/ensomi-labs/sonalign)
//! searches. [`Session`] takes short microphone blocks and returns the state of the alignment and, once found, the
//! position in the song at a given capture time. Sonalign itself consumes feature frames: audio to frames lives here
//! ([`features`], [`resample`]), and the same code makes the reference and the query, as Sonalign requires.
//!
//! This is a port of the Swift app's live path, whose behaviour it keeps: `MicFeaturePayloadExtractor`,
//! `MicFeatureStreamBuffer`, `LiveAmbientSyncRuntime` and `AmbientSyncSessionEngine`. Like every core crate it does no IO
//! and reads no clock. The caller passes PCM and the time it was captured.

pub mod features;
pub mod resample;
mod session;

pub use features::{frames, frames_f32le, Stream};
pub use resample::Resampler;
pub use session::{Position, Session, Snapshot};
pub use sonalign::{Configuration, FeatureFrame, LockPhase, State, WithholdReason};

use sonalign::Engine;

/// The rate frames are computed at. Other rates are converted first.
pub const PROCESSING_RATE: u32 = 48_000;
/// The analysis window, in samples at [`PROCESSING_RATE`].
pub const WINDOW_SAMPLES: usize = 1_024;
/// The hop between windows, in samples at [`PROCESSING_RATE`].
pub const HOP_SAMPLES: usize = 512;
/// The hop in milliseconds, which is the hop Sonalign is told.
pub const HOP_MS: f64 = HOP_SAMPLES as f64 / PROCESSING_RATE as f64 * 1_000.0;
/// The analysis window in milliseconds. A frame's time is the end of its window.
pub const WINDOW_MS: f64 = WINDOW_SAMPLES as f64 / PROCESSING_RATE as f64 * 1_000.0;

/// Audio this crate refuses. A refused call changes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A sample rate outside 8 kHz to 192 kHz, or not a whole number of hertz.
    SampleRate,
    /// A sample or a capture time that is not finite.
    NotFinite,
    /// Byte PCM whose length is not a multiple of four.
    Misaligned,
    /// A song too short to give two frames.
    TooShort,
    /// Features Sonalign refuses, which ends the session: make another. Only absurd PCM gets this far.
    Features(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::SampleRate => f.write_str("the sample rate must be a whole number of hertz from 8000 to 192000"),
            Error::NotFinite => f.write_str("samples and capture times must be finite"),
            Error::Misaligned => f.write_str("f32 PCM bytes must be a multiple of four long"),
            Error::TooShort => f.write_str("the song is too short to align to"),
            Error::Features(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for Error {}

/// A song, indexed for alignment. Reusable: every [`Session`] starts from a copy of the index.
#[derive(Clone, Debug)]
pub struct Reference {
    engine: Engine,
    duration_ms: f64,
}

impl Reference {
    /// Indexes frames from [`frames`], which end at `duration_ms`.
    pub fn from_frames(frames: &[FeatureFrame], duration_ms: f64) -> Result<Self, Error> {
        if frames.len() < 2 {
            return Err(Error::TooShort);
        }
        // The Swift app's aligner uses Sonalign's default configuration; so does the live path, which needs none other.
        Ok(Self { engine: Engine::new(frames, HOP_MS, Configuration::default()), duration_ms })
    }

    /// Indexes mono PCM at `sample_rate`.
    pub fn from_pcm(pcm: &[f32], sample_rate: f64) -> Result<Self, Error> {
        Self::from_frames(&frames(pcm, sample_rate)?, pcm.len() as f64 / sample_rate * 1_000.0)
    }

    /// Indexes mono little-endian f32 PCM at `sample_rate`, with no second copy of the song.
    pub fn from_f32le(pcm: &[u8], sample_rate: f64) -> Result<Self, Error> {
        Self::from_frames(&frames_f32le(pcm, sample_rate)?, (pcm.len() / 4) as f64 / sample_rate * 1_000.0)
    }

    /// The song's length.
    pub fn duration_ms(&self) -> f64 {
        self.duration_ms
    }

    /// A session listening at `sample_rate`.
    pub fn session(&self, sample_rate: f64) -> Result<Session, Error> {
        Session::new(self.engine.clone(), sample_rate)
    }
}
