//! Native audio probing and streaming decode from file descriptors ([`ensomi_audio`]).
//!
//! Behind the `audio` feature, enabled for Android so audio stays in native memory with zero JVM
//! heap copies and shares the unified BASS / release-engine timeline alignment.

use std::fmt;
use std::sync::{Arc, Mutex};

/// Output channel layout requested from [`AudioDecoder`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AudioChannels {
    Mono,
    Stereo,
}

impl From<AudioChannels> for ensomi_audio::AudioChannels {
    fn from(value: AudioChannels) -> Self {
        match value {
            AudioChannels::Mono => ensomi_audio::AudioChannels::Mono,
            AudioChannels::Stereo => ensomi_audio::AudioChannels::Stereo,
        }
    }
}

/// Probed metadata and timeline alignment of an audio source.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub total_frames: Option<u64>,
    pub duration_ms: Option<f64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub codec: String,
    pub lead_frames: i32,
}

impl From<ensomi_audio::AudioInfo> for AudioInfo {
    fn from(info: ensomi_audio::AudioInfo) -> Self {
        Self {
            sample_rate: info.sample_rate,
            channels: info.channels,
            total_frames: info.total_frames,
            duration_ms: info.duration_ms,
            title: info.title,
            artist: info.artist,
            codec: info.codec,
            lead_frames: info.lead_frames,
        }
    }
}

/// Errors produced when probing, decoding, or seeking an audio stream.
#[derive(Debug, uniffi::Error)]
pub enum AudioError {
    Open { message: String },
    Unsupported { message: String },
    Decode { message: String },
    Seek { message: String },
    Cancelled,
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::Open { message } => write!(f, "audio open: {message}"),
            AudioError::Unsupported { message } => write!(f, "audio unsupported: {message}"),
            AudioError::Decode { message } => write!(f, "audio decode: {message}"),
            AudioError::Seek { message } => write!(f, "audio seek: {message}"),
            AudioError::Cancelled => write!(f, "audio decode cancelled"),
        }
    }
}

impl std::error::Error for AudioError {}

impl From<ensomi_audio::AudioError> for AudioError {
    fn from(err: ensomi_audio::AudioError) -> Self {
        match err {
            ensomi_audio::AudioError::Open { message } => AudioError::Open { message },
            ensomi_audio::AudioError::Unsupported { message } => AudioError::Unsupported { message },
            ensomi_audio::AudioError::Decode { message } => AudioError::Decode { message },
            ensomi_audio::AudioError::Seek { message } => AudioError::Seek { message },
            ensomi_audio::AudioError::Cancelled => AudioError::Cancelled,
        }
    }
}

/// Takes ownership of a detached Unix file descriptor, closing it reliably when the returned
/// [`std::fs::File`] is dropped.
#[cfg(unix)]
pub(crate) fn file_from_fd(fd: i32) -> Result<std::fs::File, AudioError> {
    use std::os::fd::FromRawFd;
    if fd < 0 {
        return Err(AudioError::Open {
            message: format!("invalid file descriptor: {fd}"),
        });
    }
    // SAFETY: The FFI contract transfers ownership of a detached open FD to Rust.
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

#[cfg(not(unix))]
pub(crate) fn file_from_fd(_fd: i32) -> Result<std::fs::File, AudioError> {
    Err(AudioError::Unsupported {
        message: "file descriptor decoding is only supported on Unix platforms".to_string(),
    })
}

/// Fast header and tag probe without decoding the full audio stream.
/// Takes ownership of `fd` and closes it before returning.
#[uniffi::export]
pub fn probe_audio_fd(fd: i32, hint_ext: Option<String>) -> Result<AudioInfo, AudioError> {
    let file = file_from_fd(fd)?;
    let info = ensomi_audio::probe(file, hint_ext.as_deref())?;
    Ok(info.into())
}

/// Streaming native audio decoder for playback, thread-safe and explicitly closable.
#[derive(uniffi::Object)]
pub struct AudioDecoder {
    info: AudioInfo,
    stream: Mutex<Option<ensomi_audio::AudioStream>>,
}

#[uniffi::export]
impl AudioDecoder {
    /// Opens an audio stream from a detached file descriptor, taking ownership of `fd`.
    /// Closes `fd` on error, [`AudioDecoder::close`], or drop.
    #[uniffi::constructor]
    pub fn open_fd(
        fd: i32,
        hint_ext: Option<String>,
        channels: AudioChannels,
    ) -> Result<Arc<Self>, AudioError> {
        let file = file_from_fd(fd)?;
        let stream = ensomi_audio::AudioStream::open(file, hint_ext.as_deref(), channels.into())?;
        let info = stream.info().clone().into();
        Ok(Arc::new(Self { info, stream: Mutex::new(Some(stream)) }))
    }

    /// Probed metadata and timeline properties of this stream.
    pub fn info(&self) -> AudioInfo {
        self.info.clone()
    }

    /// Reads up to `max_frames` frames as interleaved little-endian `f32` bytes; empty on EOF.
    pub fn read_f32le(&self, max_frames: u32) -> Result<Vec<u8>, AudioError> {
        let mut guard = crate::lock(&self.stream);
        let stream = guard.as_mut().ok_or_else(|| AudioError::Decode {
            message: "audio decoder is closed".to_string(),
        })?;
        let samples = stream.read_f32(max_frames as usize)?;
        let mut bytes = Vec::with_capacity(samples.len() * 4);
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(bytes)
    }

    /// Sample-accurate seek on the song timeline (accounting for `lead_frames`).
    pub fn seek_frame(&self, frame: u64) -> Result<(), AudioError> {
        let mut guard = crate::lock(&self.stream);
        let stream = guard.as_mut().ok_or_else(|| AudioError::Seek {
            message: "audio decoder is closed".to_string(),
        })?;
        stream.seek_frame(frame)?;
        Ok(())
    }
}

impl AudioDecoder {
    /// Releases the underlying decoder and closes the file descriptor immediately. Idempotent.
    ///
    /// Note: UniFFI's Kotlin bindings already implement `AutoCloseable.close()` on every object,
    /// which frees the native handle and drops `AudioStream` (closing the FD). Keeping this outside
    /// `#[uniffi::export]` avoids a conflicting `fun close(): Unit` overload in Kotlin.
    pub fn close(&self) {
        *crate::lock(&self.stream) = None;
    }
}
