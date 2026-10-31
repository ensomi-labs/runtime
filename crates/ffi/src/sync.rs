//! Live audio sync ([`ensomi_sync`]): a song indexed from byte PCM, and a session that takes microphone blocks.
//!
//! Behind the `sync` feature, which the Android build turns on. The Apple apps take Sonalign as a Swift package, so their
//! library must not carry a second copy of its C symbols, and their checked-in bindings do not change.

use ensomi_sync::{Reference, Session, Snapshot, State, WithholdReason};
use std::sync::{Arc, Mutex, PoisonError};

/// Audio or a time the runtime cannot use. A refused block leaves the session as it was.
#[derive(Debug, uniffi::Error)]
pub enum SyncError {
    Refused { detail: String },
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let SyncError::Refused { detail } = self;
        f.write_str(detail)
    }
}

impl std::error::Error for SyncError {}

impl From<ensomi_sync::Error> for SyncError {
    fn from(error: ensomi_sync::Error) -> Self {
        SyncError::Refused { detail: error.to_string() }
    }
}

/// Sonalign's alignment states. `Locked` is the one to play from, with [`SyncSnapshot::is_final`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncState {
    Ready,
    Listening,
    Locking,
    Confirmed,
    Locked,
    Drifting,
    Relocking,
    Lost,
    Failed,
}

/// Why a snapshot holds no supported position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncWithholdReason {
    InsufficientDuration,
    InsufficientEnergy,
    InsufficientActiveFrames,
    InsufficientLandmarkEvidence,
    WeakAlignmentPeak,
    AmbiguousOffset,
    UnstableTrackingResidual,
    LostSignal,
    DecodeFailed,
    IndexUnavailable,
}

/// The session after a block. `reference_time_ms` was playing at `capture_time_ms` on the caller's clock, both set or
/// both unset: the song is at `reference_time_ms + (now - capture_time_ms)`. Before `is_final` they are provisional.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SyncSnapshot {
    pub state: SyncState,
    /// The lock to act on: the Swift app's `state == .locked && phase == .final`.
    pub is_final: bool,
    pub reference_time_ms: Option<f64>,
    pub capture_time_ms: Option<f64>,
    /// Temporal spectral agreement from 0 to 1, not a probability. The Swift path shows it and gates on nothing but the
    /// final lock.
    pub confidence: f64,
    pub withhold_reason: Option<SyncWithholdReason>,
}

impl From<Snapshot> for SyncSnapshot {
    fn from(snapshot: Snapshot) -> Self {
        Self {
            state: match snapshot.state {
                State::Ready => SyncState::Ready,
                State::Listening => SyncState::Listening,
                State::Locking => SyncState::Locking,
                State::Confirmed => SyncState::Confirmed,
                State::Locked => SyncState::Locked,
                State::Drifting => SyncState::Drifting,
                State::Relocking => SyncState::Relocking,
                State::Lost => SyncState::Lost,
                State::Failed => SyncState::Failed,
            },
            is_final: snapshot.is_final(),
            reference_time_ms: snapshot.position.map(|position| position.reference_time_ms),
            capture_time_ms: snapshot.position.map(|position| position.capture_time_ms),
            confidence: snapshot.confidence,
            withhold_reason: snapshot.withhold_reason.map(|reason| match reason {
                WithholdReason::InsufficientDuration => SyncWithholdReason::InsufficientDuration,
                WithholdReason::InsufficientEnergy => SyncWithholdReason::InsufficientEnergy,
                WithholdReason::InsufficientActiveFrames => SyncWithholdReason::InsufficientActiveFrames,
                WithholdReason::InsufficientLandmarkEvidence => SyncWithholdReason::InsufficientLandmarkEvidence,
                WithholdReason::WeakAlignmentPeak => SyncWithholdReason::WeakAlignmentPeak,
                WithholdReason::AmbiguousOffset => SyncWithholdReason::AmbiguousOffset,
                WithholdReason::UnstableTrackingResidual => SyncWithholdReason::UnstableTrackingResidual,
                WithholdReason::LostSignal => SyncWithholdReason::LostSignal,
                WithholdReason::DecodeFailed => SyncWithholdReason::DecodeFailed,
                WithholdReason::IndexUnavailable => SyncWithholdReason::IndexUnavailable,
            }),
        }
    }
}

/// A song indexed for sync. Sessions keep no reference to it.
#[derive(uniffi::Object)]
pub struct SyncReference(Reference);

#[uniffi::export]
impl SyncReference {
    /// Indexes a whole song from mono little-endian f32 PCM, at the file's sample rate. Blocking: run it off the main
    /// thread. The song does not cross as a list of floats, which it would at millions of samples.
    #[uniffi::constructor]
    pub fn from_pcm_f32le(pcm: Vec<u8>, sample_rate: f64) -> Result<Arc<Self>, SyncError> {
        Ok(Arc::new(Self(Reference::from_f32le(&pcm, sample_rate)?)))
    }

    /// The song's length.
    pub fn duration_ms(&self) -> f64 {
        self.0.duration_ms()
    }
}

/// One listening session. Calls to one session are made safe by a lock nobody but the caller's capture thread takes.
#[derive(uniffi::Object)]
pub struct SyncSession(Mutex<Session>);

#[uniffi::export]
impl SyncSession {
    /// A session for microphone PCM at `sample_rate`.
    #[uniffi::constructor]
    pub fn new(reference: Arc<SyncReference>, sample_rate: f64) -> Result<Arc<Self>, SyncError> {
        Ok(Arc::new(Self(Mutex::new(reference.0.session(sample_rate)?))))
    }

    /// Takes a block of mono little-endian f32 PCM, of any length, and returns the session's state after it.
    /// `capture_time_ms` is when the block's first sample was captured, on any monotonic clock of the caller's. Every
    /// 100 ms of audio this runs a search, which takes a few milliseconds: call it from a thread that may wait, not from
    /// an audio callback.
    pub fn push_f32le(&self, pcm: Vec<u8>, capture_time_ms: f64) -> Result<SyncSnapshot, SyncError> {
        let (samples, rest) = pcm.as_chunks::<4>();
        if !rest.is_empty() {
            return Err(ensomi_sync::Error::Misaligned.into());
        }
        let samples: Vec<f32> = samples.iter().map(|bytes| f32::from_le_bytes(*bytes)).collect();
        Ok(self.0.lock().unwrap_or_else(PoisonError::into_inner).push(&samples, capture_time_ms)?.into())
    }
}
