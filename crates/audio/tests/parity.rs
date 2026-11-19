//! Click-track onset parity, seeking accuracy, channel mapping, metadata probing, and cancellation
//! tests for `ensomi-audio`.

use ensomi_audio::lead::{self, Mp3Header, DECODER_DELAY_FRAMES};
use ensomi_audio::{decode_mono_f32, probe, AudioChannels, AudioError, AudioStream};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

const SAMPLE_RATE: u32 = 44_100;
const DURATION_SECS: usize = 4;
const CLICK_TIMES_MS: [f64; 3] = [500.0, 1500.0, 2500.0];

/// Generates a 44.1 kHz stereo 16-bit WAV file with crisp 10 ms 2 kHz click bursts at
/// `CLICK_TIMES_MS` (amplitude 0.8 on L, 0.4 on R, so mono average is 0.6).
fn make_click_wav() -> (Vec<u8>, Vec<f32>) {
    let total_frames = SAMPLE_RATE as usize * DURATION_SECS;
    let mut mono_ref = vec![0.0_f32; total_frames];
    let mut pcm_s16 = vec![0i16; total_frames * 2];

    let click_len = (SAMPLE_RATE as usize) / 100; // 10 ms = 441 samples
    for &t_ms in &CLICK_TIMES_MS {
        let start = (t_ms / 1000.0 * SAMPLE_RATE as f64).round() as usize;
        for i in 0..click_len {
            let idx = start + i;
            if idx >= total_frames {
                break;
            }
            // Fast-attack burst: full onset at i == 0 so onset detection is sub-sample sharp.
            let env = 1.0 - (i as f64 / click_len as f64);
            let carrier = (std::f64::consts::TAU * 2_000.0 * (i as f64 + 0.5) / SAMPLE_RATE as f64).sin();
            let base = env * carrier;
            let l = (base * 0.8).clamp(-1.0, 1.0);
            let r = (base * 0.4).clamp(-1.0, 1.0);
            let li = (l * 32767.0).round() as i16;
            let ri = (r * 32767.0).round() as i16;
            pcm_s16[idx * 2] = li;
            pcm_s16[idx * 2 + 1] = ri;
            mono_ref[idx] = ((f64::from(li) / 32768.0 + f64::from(ri) / 32768.0) * 0.5) as f32;
        }
    }

    let wav = encode_wav_s16(&pcm_s16, SAMPLE_RATE, 2);
    (wav, mono_ref)
}

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
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
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

/// Finds the lag (in frames) that maximizes cross-correlation between `decoded` and `reference`
/// around the first click (`CLICK_TIMES_MS[0]`), giving sub-millisecond onset alignment accuracy
/// unaffected by lossy codec pre-ringing.
fn measure_lag_frames(decoded: &[f32], reference: &[f32], search_radius: isize) -> isize {
    let click_start = (CLICK_TIMES_MS[0] / 1000.0 * SAMPLE_RATE as f64).round() as usize;
    let win_start = click_start.saturating_sub(200);
    let win_end = (click_start + 700).min(reference.len());

    let mut best_lag = 0isize;
    let mut best_corr = f64::NEG_INFINITY;

    for lag in -search_radius..=search_radius {
        let mut corr = 0.0_f64;
        for i in win_start..win_end {
            let j = i as isize + lag;
            if j >= 0 && (j as usize) < decoded.len() {
                corr += f64::from(reference[i]) * f64::from(decoded[j as usize]);
            }
        }
        if corr > best_corr {
            best_corr = corr;
            best_lag = lag;
        }
    }
    best_lag
}

fn has_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ensomi-audio-test-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_ffmpeg(input: &Path, output: &Path, args: &[&str]) -> Vec<u8> {
    let status = Command::new("ffmpeg")
        .arg("-y")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(input)
        .args(args)
        .arg(output)
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "ffmpeg failed for {:?}", output);
    std::fs::read(output).unwrap()
}

/// Strips any ID3v2 tag and the first Xing/Info/LAME frame from a LAME MP3 to produce a
/// headerless MP3 (still containing LAME's encoder delay samples, as real headerless MP3s do).
fn strip_to_headerless_mp3(lame_mp3: &[u8]) -> Vec<u8> {
    let start = lead::id3v2_bytes(lame_mp3);
    let mpeg = &lame_mp3[start..];
    // Find first sync frame (which is the Xing/Info frame) and skip past its length.
    let (off, frame_len) = find_first_mpeg_frame(mpeg).expect("first frame");
    mpeg[off + frame_len..].to_vec()
}

/// Zeros out the LAME extension inside a LAME MP3's Xing/Info frame, producing a Xing-only MP3.
fn strip_lame_extension_keep_xing(lame_mp3: &[u8]) -> Vec<u8> {
    let mut out = lame_mp3.to_vec();
    let start = lead::id3v2_bytes(&out);
    let (off, frame_len) = find_first_mpeg_frame(&out[start..]).expect("first frame");
    let frame_start = start + off;
    let frame_end = frame_start + frame_len;
    let frame = &mut out[frame_start..frame_end];
    // Find "Xing" or "Info" inside frame.
    let x = frame
        .windows(4)
        .position(|w| w == b"Xing" || w == b"Info")
        .expect("Xing/Info tag");
    let flags = u32::from_be_bytes([frame[x + 4], frame[x + 5], frame[x + 6], frame[x + 7]]);
    let mut p = x + 8;
    if (flags & 1) != 0 { p += 4; }
    if (flags & 2) != 0 { p += 4; }
    if (flags & 4) != 0 { p += 100; }
    if (flags & 8) != 0 { p += 4; }
    // Zero out the LAME extension bytes.
    for b in &mut frame[p..] {
        *b = 0;
    }
    out
}

fn find_first_mpeg_frame(b: &[u8]) -> Option<(usize, usize)> {
    const BITRATES_V1: [usize; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    const RATES_V1: [usize; 3] = [44_100, 48_000, 32_000];
    for j in 0..b.len().saturating_sub(4) {
        if b[j] == 0xFF && (b[j + 1] & 0xE0) == 0xE0 {
            let ver = (b[j + 1] >> 3) & 3;
            let layer = (b[j + 1] >> 1) & 3;
            let br_i = usize::from(b[j + 2] >> 4);
            let sr_i = usize::from((b[j + 2] >> 2) & 3);
            if ver == 3 && layer == 1 && br_i > 0 && br_i < 15 && sr_i < 3 {
                let pad = usize::from((b[j + 2] >> 1) & 1);
                let len = 144 * BITRATES_V1[br_i] * 1000 / RATES_V1[sr_i] + pad;
                if j + len + 4 <= b.len() && b[j + len] == 0xFF && (b[j + len + 1] & 0xE0) == 0xE0 {
                    return Some((j, len));
                }
            }
        }
    }
    None
}

#[test]
fn synthetic_wav_probe_decode_channels_seek_and_cancel() {
    let (wav, mono_ref) = make_click_wav();
    let info = probe(Cursor::new(wav.clone()), Some("wav")).unwrap();
    assert_eq!(info.sample_rate, SAMPLE_RATE);
    assert_eq!(info.channels, 2);
    assert_eq!(info.total_frames, Some(mono_ref.len() as u64));
    assert_eq!(info.lead_frames, 0);
    assert!(info.codec.starts_with("pcm"));

    // Mono decode matches f64-averaged reference bit-for-bit.
    let cancel = AtomicBool::new(false);
    let decoded = decode_mono_f32(Cursor::new(wav.clone()), Some("wav"), &cancel).unwrap();
    assert_eq!(decoded.sample_rate, SAMPLE_RATE);
    assert_eq!(decoded.samples.len(), mono_ref.len());
    for (i, (&a, &b)) in decoded.samples.iter().zip(&mono_ref).enumerate() {
        assert!((a - b).abs() < 1e-6, "sample {i}: {a} != {b}");
    }

    // Stereo stream preserves L != R channels, and mono WAV duplicates to L == R.
    let mut stereo = AudioStream::open(Cursor::new(wav.clone()), Some("wav"), AudioChannels::Stereo).unwrap();
    let click_frame = (CLICK_TIMES_MS[0] / 1000.0 * SAMPLE_RATE as f64).round() as u64;
    stereo.seek_frame(click_frame).unwrap();
    let chunk = stereo.read_f32(32).unwrap();
    assert_eq!(chunk.len(), 64);
    assert!(chunk[0].abs() > 0.01);
    assert!((chunk[0] - chunk[1] * 2.0).abs() < 1e-3, "L={} R={}", chunk[0], chunk[1]);

    // Seek backward and forward on mono stream matches reference exactly.
    let mut stream = AudioStream::open(Cursor::new(wav.clone()), Some("wav"), AudioChannels::Mono).unwrap();
    for target in [22_050u64, 0, 66_150, 12_345, 110_250, 1] {
        stream.seek_frame(target).unwrap();
        assert_eq!(stream.position_frame(), target);
        let got = stream.read_f32(512).unwrap();
        assert_eq!(got.len(), 512);
        assert_eq!(got, &mono_ref[target as usize..target as usize + 512]);
    }

    // Seeking past EOF yields empty reads cleanly.
    stream.seek_frame(mono_ref.len() as u64 + 1000).unwrap();
    assert!(stream.read_f32(256).unwrap().is_empty());

    // Cooperative cancellation before decode immediately returns Cancelled.
    let cancelled = AtomicBool::new(true);
    let err = decode_mono_f32(Cursor::new(wav), Some("wav"), &cancelled).unwrap_err();
    assert_eq!(err, AudioError::Cancelled);
}

#[test]
fn mono_input_duplicates_to_stereo_and_multichannel_takes_first_two() {
    // 1-channel WAV
    let mono_samples: Vec<i16> = (0..1024).map(|i| ((i as i32 * 31) % 20000 - 10000) as i16).collect();
    let wav1 = encode_wav_s16(&mono_samples, 48_000, 1);
    let mut s = AudioStream::open(Cursor::new(wav1), Some("wav"), AudioChannels::Stereo).unwrap();
    let out = s.read_f32(1024).unwrap();
    assert_eq!(out.len(), 2048);
    for i in 0..1024 {
        assert_eq!(out[i * 2], out[i * 2 + 1]);
    }

    // 4-channel WAV: stereo takes ch0 & ch1, mono averages all 4 in f64
    let mut quad = Vec::with_capacity(256 * 4);
    for _ in 0..256 {
        quad.extend_from_slice(&[16384i16, 8192, -8192, -16384]); // L=0.5, R=0.25, sum=0.0
    }
    let wav4 = encode_wav_s16(&quad, 48_000, 4);
    let mut s_st = AudioStream::open(Cursor::new(wav4.clone()), Some("wav"), AudioChannels::Stereo).unwrap();
    let st = s_st.read_f32(4).unwrap();
    assert!((st[0] - 0.5).abs() < 1e-4 && (st[1] - 0.25).abs() < 1e-4);

    let cancel = AtomicBool::new(false);
    let mono = decode_mono_f32(Cursor::new(wav4), Some("wav"), &cancel).unwrap();
    assert!(mono.samples.iter().all(|&x| x.abs() < 1e-6));
}

#[test]
fn ffmpeg_click_track_onset_parity_and_seek_accuracy() {
    if !has_ffmpeg() {
        eprintln!("skipped: ffmpeg not available");
        return;
    }

    let dir = temp_dir("parity");
    let (wav_bytes, mono_ref) = make_click_wav();
    let wav_path = dir.join("click.wav");
    std::fs::write(&wav_path, &wav_bytes).unwrap();

    let flac_bytes = run_ffmpeg(
        &wav_path,
        &dir.join("click.flac"),
        &["-c:a", "flac", "-metadata", "title=Click Track", "-metadata", "artist=Ensomi Test"],
    );
    let lame_mp3 = run_ffmpeg(
        &wav_path,
        &dir.join("click_lame.mp3"),
        &[
            "-c:a", "libmp3lame", "-b:a", "192k",
            "-metadata", "title=LAME Click", "-metadata", "artist=Ensomi LAME",
        ],
    );
    let headerless_mp3 = strip_to_headerless_mp3(&lame_mp3);
    let xing_only_mp3 = strip_lame_extension_keep_xing(&lame_mp3);
    let m4a_aac = run_ffmpeg(
        &wav_path,
        &dir.join("click.m4a"),
        &[
            "-c:a", "aac", "-b:a", "192k",
            "-metadata", "title=AAC Click", "-metadata", "artist=Ensomi AAC",
        ],
    );
    let m4a_alac = run_ffmpeg(&wav_path, &dir.join("click_alac.m4a"), &["-c:a", "alac"]);
    let ogg_vorbis = {
        let out_path = dir.join("click.ogg");
        let try_libvorbis = Command::new("ffmpeg")
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(&wav_path)
            .args([
                "-c:a", "libvorbis", "-q:a", "6",
                "-metadata", "title=Vorbis Click", "-metadata", "artist=Ensomi Vorbis",
            ])
            .arg(&out_path)
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if try_libvorbis {
            std::fs::read(&out_path).unwrap()
        } else {
            run_ffmpeg(
                &wav_path,
                &out_path,
                &[
                    "-c:a", "vorbis", "-strict", "-2", "-b:a", "192k",
                    "-metadata", "title=Vorbis Click", "-metadata", "artist=Ensomi Vorbis",
                ],
            )
        }
    };
    let mkv_flac = run_ffmpeg(&wav_path, &dir.join("click.mkv"), &["-c:a", "flac"]);

    // Verify metadata probing across formats.
    let flac_info = probe(Cursor::new(flac_bytes.clone()), Some("flac")).unwrap();
    assert_eq!(flac_info.title.as_deref(), Some("Click Track"));
    assert_eq!(flac_info.artist.as_deref(), Some("Ensomi Test"));

    let mp3_info = probe(Cursor::new(lame_mp3.clone()), Some("mp3")).unwrap();
    assert_eq!(mp3_info.title.as_deref(), Some("LAME Click"));
    assert_eq!(mp3_info.artist.as_deref(), Some("Ensomi LAME"));

    let aac_info = probe(Cursor::new(m4a_aac.clone()), Some("m4a")).unwrap();
    assert_eq!(aac_info.title.as_deref(), Some("AAC Click"));
    assert_eq!(aac_info.artist.as_deref(), Some("Ensomi AAC"));

    let ogg_info = probe(Cursor::new(ogg_vorbis.clone()), Some("ogg")).unwrap();
    assert_eq!(ogg_info.title.as_deref(), Some("Vorbis Click"));
    assert_eq!(ogg_info.artist.as_deref(), Some("Ensomi Vorbis"));

    // Extract the LAME encoder delay from our LAME MP3 so we know BASS's exact expected offset
    // for headerless and Xing-only MP3 (where BASS trims 0, keeping enc_delay + 529 frames).
    let lame_delay = match lead::mp3_header(&lame_mp3) {
        Mp3Header::Lame { delay, .. } => delay as isize,
        other => panic!("expected LAME header, got {other:?}"),
    };
    assert_eq!(lead::mp3_header(&headerless_mp3), Mp3Header::None);
    assert_eq!(lead::mp3_header(&xing_only_mp3), Mp3Header::Xing);

    // Expected lag vs original WAV (in frames):
    // - WAV, FLAC, ALAC, MKV(FLAC), LAME MP3, M4A(AAC), OGG Vorbis: 0 frames (within < 1 ms)
    // - Headerless MP3 & Xing-only MP3: BASS trims 0 frames, so click is at +lame_delay + 529!
    let expected_bass_untrimmed = lame_delay + DECODER_DELAY_FRAMES as isize;

    let cases: Vec<(&str, &str, Vec<u8>, isize, bool)> = vec![
        ("WAV", "wav", wav_bytes, 0, true),
        ("FLAC", "flac", flac_bytes, 0, true),
        ("LAME MP3", "mp3", lame_mp3, 0, false),
        ("Headerless MP3", "mp3", headerless_mp3, expected_bass_untrimmed, false),
        ("Xing-only MP3", "mp3", xing_only_mp3, expected_bass_untrimmed, false),
        ("M4A (AAC)", "m4a", m4a_aac, 0, false),
        ("M4A (ALAC)", "m4a", m4a_alac, 0, true),
        ("OGG Vorbis", "ogg", ogg_vorbis, 0, false),
        ("MKV (FLAC)", "mkv", mkv_flac, 0, true),
    ];

    let cancel = AtomicBool::new(false);
    for (name, ext, data, expected_lag, lossless) in cases {
        let probed = probe(Cursor::new(data.clone()), Some(ext)).unwrap();
        let decoded = decode_mono_f32(Cursor::new(data.clone()), Some(ext), &cancel).unwrap();
        let lag = measure_lag_frames(&decoded.samples, &mono_ref, 3000);
        let err_frames = lag - expected_lag;
        let err_ms = err_frames as f64 * 1000.0 / f64::from(SAMPLE_RATE);
        println!(
            "{name:15} | codec={:10} | lead={:6} | lag={:5} | expected={:5} | err={:+3} frames ({:+.3} ms)",
            probed.codec, probed.lead_frames, lag, expected_lag, err_frames, err_ms
        );
        assert!(
            err_ms.abs() < 1.0,
            "{name}: onset error {err_ms:+.3} ms ({err_frames} frames) exceeds 1.0 ms"
        );

        // Verify sample-accurate seek_frame matches sequential decode across multiple positions!
        let mut stream = AudioStream::open(Cursor::new(data), Some(ext), AudioChannels::Mono).unwrap();
        let click2 = (CLICK_TIMES_MS[1] / 1000.0 * SAMPLE_RATE as f64).round() as u64;
        let click3 = (CLICK_TIMES_MS[2] / 1000.0 * SAMPLE_RATE as f64).round() as u64;
        for target in [click2, 0u64, click3, 12_345, click2.saturating_sub(137)] {
            stream.seek_frame(target).unwrap();
            assert_eq!(stream.position_frame(), target);
            let got = stream.read_f32(1024).unwrap();
            let t = target as usize;
            assert!(
                t + got.len() <= decoded.samples.len() && got.len() == 1024,
                "{name} seek({target}): got {} frames",
                got.len()
            );
            let expected = &decoded.samples[t..t + 1024];
            let max_diff = got
                .iter()
                .zip(expected)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            let tol = if lossless { 1e-6 } else { 5e-3 };
            assert!(
                max_diff <= tol,
                "{name} seek({target}): max sample diff {max_diff} exceeds {tol}"
            );
        }
    }

    // Also test Apple afconvert AAC M4A (with iTunSMPB) when available on macOS.
    if Command::new("afconvert").arg("--help").output().is_ok() {
        let apple_m4a_path = dir.join("click_apple.m4a");
        let st = Command::new("afconvert")
            .args(["-f", "m4af", "-d", "aac", "-b", "192000"])
            .arg(&wav_path)
            .arg(&apple_m4a_path)
            .status()
            .unwrap();
        if st.success() {
            let apple_m4a = std::fs::read(&apple_m4a_path).unwrap();
            let probed = probe(Cursor::new(apple_m4a.clone()), Some("m4a")).unwrap();
            let decoded = decode_mono_f32(Cursor::new(apple_m4a), Some("m4a"), &cancel).unwrap();
            let lag = measure_lag_frames(&decoded.samples, &mono_ref, 3000);
            let err_ms = lag as f64 * 1000.0 / f64::from(SAMPLE_RATE);
            println!(
                "{:15} | codec={:10} | lead={:6} | lag={:5} | expected={:5} | err={:+3} frames ({:+.3} ms)",
                "Apple M4A(AAC)", probed.codec, probed.lead_frames, lag, 0, lag, err_ms
            );
            assert!(err_ms.abs() < 1.0, "Apple M4A(AAC) error {err_ms:+.3} ms");
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}
