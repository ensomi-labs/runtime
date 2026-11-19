//! Native audio FD bindings: `probe_audio_fd`, `AudioDecoder`, `SyncReference::from_fd`,
//! `Prepared` path round-trips, and `ChartGenerator::prepare_from_fd`.
#![cfg(all(unix, feature = "audio"))]

use ensomi_runtime_ffi::audio::{probe_audio_fd, AudioChannels, AudioDecoder, AudioError};
use ensomi_runtime_ffi::generator::{
    Cancellation, ChartGenerator, ExecutionProvider, GeneratorError, GeneratorOptions, Prepared,
};
use std::fs::File;
use std::os::fd::IntoRawFd;
use std::path::{Path, PathBuf};

fn encode_wav_s16(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_bytes = (samples.len() * 2) as u32;
    let byte_rate = sample_rate * u32::from(channels) * 2;
    let block_align = channels * 2;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

fn write_temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ensomi-ffi-audio-{name}-{}.wav",
        std::process::id()
    ));
    std::fs::write(&path, bytes).unwrap();
    path
}

fn open_raw_fd(path: &Path) -> i32 {
    File::open(path).unwrap().into_raw_fd()
}

fn decode_f32le(bytes: &[u8]) -> Vec<f32> {
    assert_eq!(bytes.len() % 4, 0);
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&c| f32::from_le_bytes(c))
        .collect()
}

#[test]
fn probe_and_stream_decode_wav_fd() {
    let rate = 44_100u32;
    let frames = 4_096usize;
    let mut s16 = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let l = (((i as f64 / 32.0).sin() * 16_000.0).round()) as i16;
        let r = (((i as f64 / 48.0).cos() * 8_000.0).round()) as i16;
        s16.push(l);
        s16.push(r);
    }
    let wav = encode_wav_s16(&s16, rate, 2);
    let path = write_temp_file("stream", &wav);

    // Negative fd is rejected cleanly.
    assert!(matches!(probe_audio_fd(-1, Some("wav".to_string())), Err(AudioError::Open { .. })));

    // Probe valid FD.
    let info = probe_audio_fd(open_raw_fd(&path), Some("wav".to_string())).unwrap();
    assert_eq!(info.sample_rate, rate);
    assert_eq!(info.channels, 2);
    assert_eq!(info.total_frames, Some(frames as u64));
    assert_eq!(info.lead_frames, 0);
    assert!(info.codec.starts_with("pcm"));

    // Stream stereo with seek + EOF + close.
    let decoder = AudioDecoder::open_fd(
        open_raw_fd(&path),
        Some("wav".to_string()),
        AudioChannels::Stereo,
    )
    .unwrap();
    assert_eq!(decoder.info(), info);

    let first = decode_f32le(&decoder.read_f32le(1_000).unwrap());
    assert_eq!(first.len(), 2_000);
    assert!((first[0] - f32::from(s16[0]) / 32768.0).abs() < 1e-6);
    assert!((first[1] - f32::from(s16[1]) / 32768.0).abs() < 1e-6);

    // Seek to frame 1234 and verify exact samples.
    decoder.seek_frame(1_234).unwrap();
    let sought = decode_f32le(&decoder.read_f32le(512).unwrap());
    assert_eq!(sought.len(), 1_024);
    for i in 0..512 {
        let idx = (1_234 + i) * 2;
        assert!((sought[i * 2] - f32::from(s16[idx]) / 32768.0).abs() < 1e-6);
        assert!((sought[i * 2 + 1] - f32::from(s16[idx + 1]) / 32768.0).abs() < 1e-6);
    }

    // Drain to EOF.
    let mut remaining = 0usize;
    loop {
        let chunk = decoder.read_f32le(1_024).unwrap();
        if chunk.is_empty() {
            break;
        }
        remaining += chunk.len() / 8;
    }
    assert_eq!(1_234 + 512 + remaining, frames);

    // Close is idempotent; info() stays available; read/seek fail cleanly after close.
    decoder.close();
    decoder.close();
    assert_eq!(decoder.info().sample_rate, rate);
    assert!(matches!(decoder.read_f32le(64), Err(AudioError::Decode { .. })));
    assert!(matches!(decoder.seek_frame(0), Err(AudioError::Seek { .. })));

    let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "sync")]
#[test]
fn sync_reference_from_fd_locks_session() {
    use ensomi_runtime_ffi::sync::{SyncError, SyncReference, SyncSession, SyncState};

    let rate = 48_000u32;
    let seconds = 15usize;
    let (mut state, mut phase, mut hertz) = (1u32, 0.0f64, 440.0f64);
    let mut pcm_f32 = Vec::with_capacity(rate as usize * seconds);
    let mut pcm_s16 = Vec::with_capacity(rate as usize * seconds);
    for i in 0..(rate as usize * seconds) {
        if i % (rate as usize * 12 / 100) == 0 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            hertz = 220.0 * 2f64.powf(f64::from(state >> 8) / f64::from(1u32 << 24) * 2.0);
        }
        phase += std::f64::consts::TAU * hertz / f64::from(rate);
        let sample = phase.sin() * 0.3 * (1.0 - (i as f64 / f64::from(rate) * 8.33).fract());
        let s = (sample * 32767.0).round() as i16;
        pcm_s16.push(s);
        pcm_f32.push(f32::from(s) / 32768.0);
    }

    let wav = encode_wav_s16(&pcm_s16, rate, 1);
    let path = write_temp_file("sync", &wav);

    // Negative FD fails with SyncError::Refused.
    assert!(matches!(
        SyncReference::from_fd(-1, Some("wav".to_string())),
        Err(SyncError::Refused { .. })
    ));

    let reference = SyncReference::from_fd(open_raw_fd(&path), Some("wav".to_string())).unwrap();
    assert!((reference.duration_ms() - 15_000.0).abs() < 1.0);

    let session = SyncSession::new(reference, f64::from(rate)).unwrap();
    let offset_ms = 4_000.0;
    let clock_ms = 500_000.0;
    let mut locked = false;
    for (index, block) in pcm_f32[(offset_ms / 1_000.0 * f64::from(rate)) as usize..]
        .chunks(960)
        .enumerate()
    {
        let bytes: Vec<u8> = block.iter().flat_map(|s| s.to_le_bytes()).collect();
        let snap = session.push_f32le(bytes, clock_ms + index as f64 * 20.0).unwrap();
        if snap.is_final {
            assert_eq!(snap.state, SyncState::Locked);
            let err = snap.reference_time_ms.unwrap()
                - (offset_ms + snap.capture_time_ms.unwrap() - clock_ms);
            assert!(err.abs() < 3.5, "sync error: {err}");
            locked = true;
            break;
        }
    }
    assert!(locked, "SyncSession failed to lock on SyncReference::from_fd");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn prepare_from_fd_and_cancellation_when_model_present() {
    let model = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2/onnx");
    if !model.exists() {
        eprintln!("skipped: {} is absent", model.display());
        return;
    }

    let options = GeneratorOptions {
        provider: ExecutionProvider::Cpu,
        resampler_directory: None,
        low_memory: None,
        beatthis_threads: None,
    };
    let generator = ChartGenerator::open(model.to_string_lossy().into_owned(), options).unwrap();

    // 10s of reproducible noise at 22,050 Hz (no resampler tables needed).
    let mut state = 1u32;
    let pcm_s16: Vec<i16> = (0..22_050 * 10)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let f = (state >> 8) as f32 / (1 << 24) as f32 - 0.5;
            (f * 32767.0).round() as i16
        })
        .collect();
    let wav = encode_wav_s16(&pcm_s16, 22_050, 1);
    let path = write_temp_file("prepare", &wav);

    // Pre-cancelled Cancellation returns GeneratorError::Cancelled without leaking FD.
    let cancel = Cancellation::new();
    cancel.cancel();
    assert!(matches!(
        generator.prepare_from_fd(open_raw_fd(&path), Some("wav".to_string()), Some(cancel)),
        Err(GeneratorError::Cancelled)
    ));

    // Normal prepare_from_fd produces valid Prepared and round-trips through save_to_path/from_path.
    let prepared = generator
        .prepare_from_fd(open_raw_fd(&path), Some("wav".to_string()), None)
        .unwrap();
    assert!((prepared.song_ms() - 10_000.0).abs() < 50.0);

    let cache_path = std::env::temp_dir().join(format!(
        "ensomi-ffi-prepare-cache-{}.bin",
        std::process::id()
    ));
    let cache_str = cache_path.to_string_lossy().into_owned();
    prepared.save_to_path(cache_str.clone()).unwrap();
    let reloaded = Prepared::from_path(cache_str).unwrap();
    assert_eq!(reloaded.song_ms(), prepared.song_ms());
    assert_eq!(reloaded.bytes(), prepared.bytes());

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&cache_path);
}
