//! Native audio probing and decoding with unified BASS / release-engine timeline alignment.
//!
//! Core logic operates purely on `Read + Seek` streams (`symphonia::core::io::MediaSource`) with
//! zero filesystem paths. Frame 0 of every decoded stream aligns to the osu!/BASS & Ensomi song
//! timeline (`lead::mp3_lead_frames` for MP3, `iTunSMPB` / MP4 `elst` priming for AAC).

pub mod lead;

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};

use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Signal};
use symphonia::core::codecs::{
    CodecType, Decoder, DecoderOptions, CODEC_TYPE_AAC, CODEC_TYPE_MP3, CODEC_TYPE_NULL,
    CODEC_TYPE_VORBIS,
};
use symphonia::core::conv::FromSample;
use symphonia::core::errors::{Error as SymphoniaError, SeekErrorKind};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

/// Output channel layout requested from [`AudioStream`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioChannels {
    /// Downmix all channels to mono (averaged in `f64`).
    Mono,
    /// Interleaved stereo: mono input is duplicated to L/R; >2ch input takes the first two channels.
    Stereo,
}

impl AudioChannels {
    /// Number of interleaved samples per audio frame.
    pub fn count(self) -> usize {
        match self {
            AudioChannels::Mono => 1,
            AudioChannels::Stereo => 2,
        }
    }
}

/// Probed metadata and timeline properties of an audio source.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub total_frames: Option<u64>,
    pub duration_ms: Option<f64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub codec: String,
    /// Timeline alignment applied to Symphonia's raw decoded frames: positive prepends silence
    /// frames, negative skips leading raw frames.
    pub lead_frames: i32,
}

/// Errors produced by probing, decoding, or seeking audio.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Complete mono decode result in native memory.
#[derive(Clone, Debug)]
pub struct DecodedMono {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub info: AudioInfo,
}

/// Wraps any `Read + Seek + Send + Sync` stream as a Symphonia [`MediaSource`] with cached length.
struct SeekableSource<R> {
    inner: R,
    byte_len: Option<u64>,
    seekable: bool,
}

impl<R: Read + Seek + Send + Sync> SeekableSource<R> {
    fn new(mut inner: R) -> io::Result<Self> {
        let (byte_len, seekable) = match inner.seek(SeekFrom::End(0)) {
            Ok(len) => {
                inner.seek(SeekFrom::Start(0))?;
                (Some(len), true)
            }
            Err(_) => (None, false),
        };
        Ok(Self { inner, byte_len, seekable })
    }
}

impl<R: Read> Read for SeekableSource<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for SeekableSource<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl<R: Read + Seek + Send + Sync> MediaSource for SeekableSource<R> {
    fn is_seekable(&self) -> bool {
        self.seekable
    }

    fn byte_len(&self) -> Option<u64> {
        self.byte_len
    }
}

struct OpenedTrack {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    codec_type: CodecType,
    time_base: Option<TimeBase>,
    start_ts: u64,
    info: AudioInfo,
}

fn open_source<R: Read + Seek + Send + Sync + 'static>(
    source: R,
    hint_ext: Option<&str>,
    need_decoder: bool,
) -> Result<(Box<dyn FormatReader>, Option<Box<dyn Decoder>>, u32, CodecType, Option<TimeBase>, u64, AudioInfo), AudioError> {
    let mut wrapped = SeekableSource::new(source).map_err(|e| AudioError::Open {
        message: format!("cannot inspect stream: {e}"),
    })?;

    let lead_hint = if wrapped.seekable {
        lead::scan_container_lead(&mut wrapped.inner, wrapped.byte_len).map_err(|e| {
            AudioError::Open { message: format!("cannot read stream header: {e}") }
        })?
    } else {
        lead::ContainerLeadHint::default()
    };

    let mss = MediaSourceStream::new(Box::new(wrapped), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = hint_ext {
        let cleaned = ext.trim().trim_start_matches('.');
        if !cleaned.is_empty() {
            hint.with_extension(cleaned);
        }
    }

    let fmt_opts = FormatOptions { enable_gapless: false, ..Default::default() };
    let meta_opts = MetadataOptions::default();

    let mut probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .map_err(|e| match e {
            SymphoniaError::Unsupported(msg) => AudioError::Unsupported { message: msg.to_string() },
            SymphoniaError::IoError(err) => AudioError::Open { message: err.to_string() },
            other => AudioError::Open { message: other.to_string() },
        })?;

    let codecs = symphonia::default::get_codecs();
    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| {
            t.codec_params.codec != CODEC_TYPE_NULL && codecs.get_codec(t.codec_params.codec).is_some()
        })
        .or_else(|| probed.format.default_track())
        .ok_or_else(|| AudioError::Unsupported {
            message: "no supported audio track found".to_string(),
        })?;

    let track_id = track.id;
    let params = track.codec_params.clone();
    let codec_type = params.codec;
    let time_base = params.time_base;
    let start_ts = params.start_ts;

    let codec_name = codecs
        .get_codec(codec_type)
        .map(|desc| desc.short_name.to_string())
        .ok_or_else(|| AudioError::Unsupported {
            message: format!("unsupported audio codec ({codec_type:?})"),
        })?;

    let mut title: Option<String> = None;
    let mut artist: Option<String> = None;
    let mut meta_itunsmpb: Option<u32> = None;

    if let Some(mut container_meta) = probed.metadata.get() {
        while !container_meta.is_latest() {
            container_meta.pop();
        }
        if let Some(rev) = container_meta.current() {
            extract_tags(rev, &mut title, &mut artist, &mut meta_itunsmpb);
        }
    }
    {
        let mut format_meta = probed.format.metadata();
        while !format_meta.is_latest() {
            format_meta.pop();
        }
        if let Some(rev) = format_meta.current() {
            extract_tags(rev, &mut title, &mut artist, &mut meta_itunsmpb);
        }
    }

    let mut sample_rate = params.sample_rate;
    let mut channels = params
        .channels
        .map(|c| c.count() as u16)
        .or_else(|| params.channel_layout.map(|l| l.into_channels().count() as u16));

    let decoder = if need_decoder || sample_rate.is_none() || channels.is_none() {
        let mut dec = codecs
            .make(&params, &DecoderOptions::default())
            .map_err(|e| AudioError::Unsupported { message: e.to_string() })?;

        if sample_rate.is_none() || channels.is_none() {
            // Decode one packet to learn sample_rate/channels, then rewind if we keep the stream.
            if let Some((sr, ch)) = peek_first_packet_spec(probed.format.as_mut(), dec.as_mut(), track_id)? {
                sample_rate = sample_rate.or(Some(sr));
                channels = channels.or(Some(ch));
            }
            if need_decoder {
                let _ = probed.format.seek(
                    SeekMode::Accurate,
                    SeekTo::TimeStamp { ts: start_ts, track_id },
                );
                dec.reset();
            }
        }
        if need_decoder { Some(dec) } else { None }
    } else {
        None
    };

    let sample_rate = sample_rate.ok_or_else(|| AudioError::Unsupported {
        message: "missing sample rate".to_string(),
    })?;
    let channels = channels.ok_or_else(|| AudioError::Unsupported {
        message: "missing channel count".to_string(),
    })?;
    if sample_rate == 0 || channels == 0 {
        return Err(AudioError::Unsupported {
            message: format!("invalid audio parameters: rate={sample_rate}, channels={channels}"),
        });
    }

    let lead_frames = compute_lead_frames(
        codec_type,
        sample_rate,
        params.delay,
        &lead_hint,
        meta_itunsmpb,
    );

    let total_frames = params.n_frames.map(|n| {
        if lead_frames >= 0 {
            n.saturating_add(lead_frames as u64)
        } else {
            n.saturating_sub(u64::from(lead_frames.unsigned_abs()))
        }
    });
    let duration_ms = total_frames.map(|f| f as f64 * 1000.0 / f64::from(sample_rate));

    let info = AudioInfo {
        sample_rate,
        channels,
        total_frames,
        duration_ms,
        title,
        artist,
        codec: codec_name,
        lead_frames,
    };

    Ok((probed.format, decoder, track_id, codec_type, time_base, start_ts, info))
}

fn open_track<R: Read + Seek + Send + Sync + 'static>(
    source: R,
    hint_ext: Option<&str>,
) -> Result<OpenedTrack, AudioError> {
    let (format, decoder, track_id, codec_type, time_base, start_ts, info) =
        open_source(source, hint_ext, true)?;
    Ok(OpenedTrack {
        format,
        decoder: decoder.expect("decoder requested"),
        track_id,
        codec_type,
        time_base,
        start_ts,
        info,
    })
}

fn compute_lead_frames(
    codec: CodecType,
    sample_rate: u32,
    codec_params_delay: Option<u32>,
    hint: &lead::ContainerLeadHint,
    meta_itunsmpb: Option<u32>,
) -> i32 {
    if codec == CODEC_TYPE_MP3 {
        if let Some(mp3_lead) = hint.mp3_lead {
            return mp3_lead;
        }
        if let Some(priming) = meta_itunsmpb {
            return -lead::DECODER_DELAY_FRAMES - (priming as i32);
        }
        return 0;
    }
    if codec == CODEC_TYPE_AAC {
        if let Some(priming) = meta_itunsmpb {
            return -(priming as i32);
        }
        if let Some((media_time, timescale)) = hint.mp4_elst_delay {
            let priming = lead::scale_to_sample_rate(media_time, timescale, sample_rate);
            return -(priming as i32);
        }
        if let Some(delay) = codec_params_delay {
            if delay > 0 {
                return -(delay as i32);
            }
        }
        return 0;
    }
    if codec == CODEC_TYPE_VORBIS {
        if let Some(delay) = codec_params_delay {
            if delay > 0 {
                return -(delay as i32);
            }
        }
    }
    0
}

fn extract_tags(
    rev: &MetadataRevision,
    title: &mut Option<String>,
    artist: &mut Option<String>,
    itunsmpb: &mut Option<u32>,
) {
    for tag in rev.tags() {
        let val = tag.value.to_string();
        let trimmed = val.trim();
        if trimmed.is_empty() {
            continue;
        }
        match tag.std_key {
            Some(StandardTagKey::TrackTitle) if title.is_none() => {
                *title = Some(trimmed.to_string());
            }
            Some(StandardTagKey::Artist) if artist.is_none() => {
                *artist = Some(trimmed.to_string());
            }
            _ => {}
        }
        if itunsmpb.is_none() && tag.key.contains("iTunSMPB") {
            *itunsmpb = lead::parse_itunsmpb_text(trimmed);
        }
    }
}

fn peek_first_packet_spec(
    format: &mut dyn FormatReader,
    decoder: &mut dyn Decoder,
    track_id: u32,
) -> Result<Option<(u32, u16)>, AudioError> {
    for _ in 0..16 {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok(None);
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(AudioError::Decode { message: e.to_string() }),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                if decoded.frames() > 0 || (spec.rate > 0 && spec.channels.count() > 0) {
                    return Ok(Some((spec.rate, spec.channels.count() as u16)));
                }
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(AudioError::Decode { message: e.to_string() }),
        }
    }
    Ok(None)
}

/// Fast header/tag probe without full decode.
pub fn probe<R: Read + Seek + Send + Sync + 'static>(
    source: R,
    hint_ext: Option<&str>,
) -> Result<AudioInfo, AudioError> {
    let (_, _, _, _, _, _, info) = open_source(source, hint_ext, false)?;
    Ok(info)
}

/// Streaming audio decoder for playback, preserving file sample rate and applying BASS/AAC lead
/// alignment across both sequential reads and [`AudioStream::seek_frame`].
pub struct AudioStream {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    codec_type: CodecType,
    time_base: Option<TimeBase>,
    start_ts: u64,
    info: AudioInfo,
    out_channels: AudioChannels,
    current_song_frame: u64,
    pending_silence: u64,
    raw_skip: u64,
    seek_target_ts: Option<u64>,
    buf: Vec<f32>,
    buf_pos: usize,
    eof: bool,
    decoded_any_frame: bool,
}

impl AudioStream {
    /// Opens an audio stream for decoding to `channels` (`Mono` or `Stereo`).
    pub fn open<R: Read + Seek + Send + Sync + 'static>(
        source: R,
        hint_ext: Option<&str>,
        channels: AudioChannels,
    ) -> Result<Self, AudioError> {
        let opened = open_track(source, hint_ext)?;
        let (pending_silence, raw_skip) = if opened.info.lead_frames >= 0 {
            (opened.info.lead_frames as u64, 0)
        } else {
            (0, u64::from(opened.info.lead_frames.unsigned_abs()))
        };
        Ok(Self {
            format: opened.format,
            decoder: opened.decoder,
            track_id: opened.track_id,
            codec_type: opened.codec_type,
            time_base: opened.time_base,
            start_ts: opened.start_ts,
            info: opened.info,
            out_channels: channels,
            current_song_frame: 0,
            pending_silence,
            raw_skip,
            seek_target_ts: None,
            buf: Vec::new(),
            buf_pos: 0,
            eof: false,
            decoded_any_frame: false,
        })
    }

    /// Probed metadata and timeline properties of this stream.
    pub fn info(&self) -> &AudioInfo {
        &self.info
    }

    /// Output channel layout (`Mono` or `Stereo`).
    pub fn channels(&self) -> AudioChannels {
        self.out_channels
    }

    /// Current song timeline position in frames.
    pub fn position_frame(&self) -> u64 {
        self.current_song_frame
    }

    /// Reads up to `max_frames` frames of interleaved `f32` PCM into a `Vec<f32>`.
    /// Returns an empty `Vec` on EOF or when `max_frames == 0`.
    pub fn read_f32(&mut self, max_frames: usize) -> Result<Vec<f32>, AudioError> {
        if max_frames == 0 || self.eof {
            return Ok(Vec::new());
        }
        let ch = self.out_channels.count();
        let initial_cap = max_frames.min(65_536).saturating_mul(ch);
        let mut out = Vec::with_capacity(initial_cap);
        let mut frames_written = 0usize;

        while frames_written < max_frames {
            let need = max_frames - frames_written;

            if self.pending_silence > 0 {
                let emit = (self.pending_silence as usize).min(need);
                out.resize(out.len() + emit * ch, 0.0);
                self.pending_silence -= emit as u64;
                self.current_song_frame += emit as u64;
                frames_written += emit;
                continue;
            }

            if self.buf_pos < self.buf.len() {
                let avail_frames = (self.buf.len() - self.buf_pos) / ch;
                let take = avail_frames.min(need);
                let take_samples = take * ch;
                out.extend_from_slice(&self.buf[self.buf_pos..self.buf_pos + take_samples]);
                self.buf_pos += take_samples;
                self.current_song_frame += take as u64;
                frames_written += take;
                if self.buf_pos >= self.buf.len() {
                    self.buf.clear();
                    self.buf_pos = 0;
                }
                continue;
            }

            if !self.fill_next_packet(None)? {
                self.eof = true;
                break;
            }
        }

        Ok(out)
    }

    /// Sample-accurate seek on the song timeline (accounting for `lead_frames`).
    pub fn seek_frame(&mut self, target_frame: u64) -> Result<(), AudioError> {
        let lead = self.info.lead_frames;
        let (pending_silence, target_raw) = if lead >= 0 {
            let lead_u = lead as u64;
            if target_frame < lead_u {
                (lead_u - target_frame, 0)
            } else {
                (0, target_frame - lead_u)
            }
        } else {
            (0, target_frame.saturating_add(u64::from(lead.unsigned_abs())))
        };

        self.buf.clear();
        self.buf_pos = 0;
        self.eof = false;
        self.current_song_frame = target_frame;
        self.pending_silence = pending_silence;
        self.raw_skip = 0;
        self.seek_target_ts = None;

        // Preroll before target for MDCT/filterbank codecs so overlap-add & synthesis state match
        // sequential decoding from 0.
        let preroll: u64 = match self.codec_type {
            CODEC_TYPE_MP3 => 2_304,
            CODEC_TYPE_AAC => 2_048,
            CODEC_TYPE_VORBIS => 4_096,
            _ => 0,
        };
        let seek_raw = target_raw.saturating_sub(preroll);
        let is_sample_tb = self
            .time_base
            .is_some_and(|tb| tb.numer == 1 && tb.denom == self.info.sample_rate);

        // Containers whose timebase is coarser than 1 sample (e.g., MKV at 1 ms resolution)
        // quantize packet timestamps, so a container seek cannot determine the exact frame index.
        // Rewind to start and skip decoded frames to guarantee sample-accurate positioning.
        if !is_sample_tb {
            return self.rewind_and_skip(target_raw);
        }

        let req_ts = self.start_ts.saturating_add(seek_raw);
        let seek_result = self
            .format
            .seek(SeekMode::Accurate, SeekTo::TimeStamp { ts: req_ts, track_id: self.track_id });

        match seek_result {
            Ok(seeked) => {
                self.decoder.reset();
                let target_ts = self.start_ts.saturating_add(target_raw);
                if seeked.actual_ts > target_ts {
                    // Container landed past target: rewind to start and decode-skip.
                    self.rewind_and_skip(target_raw)?;
                } else {
                    self.seek_target_ts = Some(target_ts);
                }
                Ok(())
            }
            Err(SymphoniaError::SeekError(SeekErrorKind::OutOfRange))
                if target_raw > 0
                    && self.info.total_frames.is_some_and(|total| target_frame >= total) =>
            {
                self.decoder.reset();
                self.eof = true;
                Ok(())
            }
            Err(err) => {
                // Fallback: rewind to start and decode-skip to target_raw.
                if self.rewind_and_skip(target_raw).is_ok() {
                    Ok(())
                } else {
                    Err(AudioError::Seek { message: err.to_string() })
                }
            }
        }
    }

    fn rewind_and_skip(&mut self, target_raw: u64) -> Result<(), AudioError> {
        let is_sample_tb = self
            .time_base
            .is_some_and(|tb| tb.numer == 1 && tb.denom == self.info.sample_rate);
        let res = if is_sample_tb {
            self.format.seek(
                SeekMode::Accurate,
                SeekTo::TimeStamp { ts: self.start_ts, track_id: self.track_id },
            )
        } else {
            self.format.seek(
                SeekMode::Accurate,
                SeekTo::Time { time: Time::new(0, 0.0), track_id: Some(self.track_id) },
            )
        };
        res.map_err(|e| AudioError::Seek { message: e.to_string() })?;
        self.decoder.reset();
        self.raw_skip = target_raw;
        self.seek_target_ts = None;
        Ok(())
    }

    fn fill_next_packet(&mut self, cancelled: Option<&AtomicBool>) -> Result<bool, AudioError> {
        let mut consecutive_decode_errors = 0u32;
        loop {
            if let Some(flag) = cancelled {
                if flag.load(Ordering::Relaxed) {
                    return Err(AudioError::Cancelled);
                }
            }

            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    return Ok(false);
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(AudioError::Decode { message: e.to_string() }),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            let pkt_ts = packet.ts();
            let decoded = match self.decoder.decode(&packet) {
                Ok(buf) => {
                    consecutive_decode_errors = 0;
                    buf
                }
                Err(SymphoniaError::DecodeError(msg)) => {
                    consecutive_decode_errors += 1;
                    if !self.decoded_any_frame && consecutive_decode_errors > 8 {
                        return Err(AudioError::Decode { message: msg.to_string() });
                    }
                    continue;
                }
                Err(SymphoniaError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    return Ok(false);
                }
                Err(e) => return Err(AudioError::Decode { message: e.to_string() }),
            };

            let frames = decoded.frames();
            if frames == 0 {
                continue;
            }
            self.decoded_any_frame = true;

            let skip_in_pkt = if let Some(target_ts) = self.seek_target_ts {
                let pkt_end = pkt_ts.saturating_add(frames as u64);
                if pkt_end <= target_ts {
                    continue;
                }
                self.seek_target_ts = None;
                target_ts.saturating_sub(pkt_ts) as usize
            } else if self.raw_skip > 0 {
                let skip = (self.raw_skip as usize).min(frames);
                self.raw_skip -= skip as u64;
                if skip == frames {
                    continue;
                }
                skip
            } else {
                0
            };

            self.buf.clear();
            self.buf_pos = 0;
            append_audio_buffer(decoded, skip_in_pkt, self.out_channels, &mut self.buf);
            if !self.buf.is_empty() {
                return Ok(true);
            }
        }
    }
}

/// Decodes an entire audio source to mono `f32` (averaged in `f64`, BASS/AAC lead applied) in
/// native memory, checking `cancelled` cooperatively on every packet.
pub fn decode_mono_f32<R: Read + Seek + Send + Sync + 'static>(
    source: R,
    hint_ext: Option<&str>,
    cancelled: &AtomicBool,
) -> Result<DecodedMono, AudioError> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(AudioError::Cancelled);
    }
    let mut stream = AudioStream::open(source, hint_ext, AudioChannels::Mono)?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(AudioError::Cancelled);
    }

    // Cap pre-allocation to ~2 hours @ 48 kHz so a malformed header cannot OOM upfront.
    const MAX_PREALLOC: usize = 48_000 * 3600 * 2;
    let estimated = stream
        .info
        .total_frames
        .map(|f| (f as usize).min(MAX_PREALLOC))
        .unwrap_or(0);
    let mut samples = Vec::with_capacity(estimated);

    if stream.pending_silence > 0 {
        let silence = stream.pending_silence as usize;
        samples.resize(silence, 0.0);
        stream.pending_silence = 0;
    }

    while stream.fill_next_packet(Some(cancelled))? {
        samples.extend_from_slice(&stream.buf);
        stream.buf.clear();
        stream.buf_pos = 0;
    }

    let sample_rate = stream.info.sample_rate;
    let info = stream.info;
    Ok(DecodedMono { samples, sample_rate, info })
}

fn append_audio_buffer(
    buf: AudioBufferRef<'_>,
    skip_frames: usize,
    channels: AudioChannels,
    out: &mut Vec<f32>,
) {
    match buf {
        AudioBufferRef::U8(b) => append_typed(b.as_ref(), skip_frames, channels, out),
        AudioBufferRef::U16(b) => append_typed(b.as_ref(), skip_frames, channels, out),
        AudioBufferRef::U24(b) => append_typed(b.as_ref(), skip_frames, channels, out),
        AudioBufferRef::U32(b) => append_typed(b.as_ref(), skip_frames, channels, out),
        AudioBufferRef::S8(b) => append_typed(b.as_ref(), skip_frames, channels, out),
        AudioBufferRef::S16(b) => append_typed(b.as_ref(), skip_frames, channels, out),
        AudioBufferRef::S24(b) => append_typed(b.as_ref(), skip_frames, channels, out),
        AudioBufferRef::S32(b) => append_typed(b.as_ref(), skip_frames, channels, out),
        AudioBufferRef::F32(b) => append_typed(b.as_ref(), skip_frames, channels, out),
        AudioBufferRef::F64(b) => append_typed(b.as_ref(), skip_frames, channels, out),
    }
}

fn append_typed<S>(
    buf: &AudioBuffer<S>,
    skip_frames: usize,
    out_channels: AudioChannels,
    out: &mut Vec<f32>,
) where
    S: symphonia::core::sample::Sample,
    f64: FromSample<S>,
    f32: FromSample<S>,
{
    let total = buf.frames();
    if skip_frames >= total {
        return;
    }
    let ch_count = buf.spec().channels.count();
    if ch_count == 0 {
        return;
    }
    let kept = total - skip_frames;
    let planes = buf.planes();
    let ch_planes = planes.planes();

    match out_channels {
        AudioChannels::Mono => {
            out.reserve(kept);
            if ch_count == 1 {
                let c0 = &ch_planes[0][skip_frames..total];
                out.extend(c0.iter().map(|&s| f32::from_sample(s)));
            } else {
                let inv = 1.0 / (ch_count as f64);
                for frame in skip_frames..total {
                    let mut sum = 0.0_f64;
                    for plane in ch_planes.iter().take(ch_count) {
                        sum += f64::from_sample(plane[frame]);
                    }
                    out.push((sum * inv) as f32);
                }
            }
        }
        AudioChannels::Stereo => {
            out.reserve(kept * 2);
            if ch_count == 1 {
                let c0 = &ch_planes[0][skip_frames..total];
                for &s in c0 {
                    let v = f32::from_sample(s);
                    out.push(v);
                    out.push(v);
                }
            } else {
                let left = &ch_planes[0][skip_frames..total];
                let right = &ch_planes[1][skip_frames..total];
                for (&l, &r) in left.iter().zip(right.iter()) {
                    out.push(f32::from_sample(l));
                    out.push(f32::from_sample(r));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn make_ramp_wav(frames: usize, rate: u32) -> Vec<u8> {
        let data_bytes = (frames * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_bytes as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_bytes.to_le_bytes());
        for i in 0..frames {
            let s = ((i as i32 + 1) * 16).min(32_000) as i16;
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    #[test]
    fn positive_lead_silence_sequential_and_seek() {
        let wav = make_ramp_wav(2048, 44_100);
        let mut stream = AudioStream::open(Cursor::new(wav), Some("wav"), AudioChannels::Mono).unwrap();
        // Simulate a stream with +529 positive lead silence frames.
        stream.info.lead_frames = 529;
        stream.pending_silence = 529;
        stream.info.total_frames = stream.info.total_frames.map(|n| n + 529);

        // Read across the silence -> raw frame 0 boundary.
        let head = stream.read_f32(600).unwrap();
        assert_eq!(head.len(), 600);
        assert!(head[..529].iter().all(|&s| s == 0.0));
        assert!((head[529] - 16.0 / 32768.0).abs() < 1e-6);
        assert!((head[530] - 32.0 / 32768.0).abs() < 1e-6);

        // Seek inside lead silence (frame 500 -> 29 frames of silence, then raw frame 0).
        stream.seek_frame(500).unwrap();
        assert_eq!(stream.position_frame(), 500);
        let mid = stream.read_f32(40).unwrap();
        assert_eq!(mid.len(), 40);
        assert!(mid[..29].iter().all(|&s| s == 0.0));
        assert!((mid[29] - 16.0 / 32768.0).abs() < 1e-6);

        // Seek past lead silence (frame 529 + 100 -> raw frame 100).
        stream.seek_frame(629).unwrap();
        assert_eq!(stream.position_frame(), 629);
        let after = stream.read_f32(8).unwrap();
        assert!((after[0] - (101.0 * 16.0) / 32768.0).abs() < 1e-6);
    }
}
