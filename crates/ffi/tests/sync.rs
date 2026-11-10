//! Live audio sync through the bindings, with byte PCM as the Kotlin helpers send it.
#![cfg(feature = "sync")]

use ensomi_runtime_ffi::sync::{SyncError, SyncReference, SyncSession, SyncState, SyncWithholdReason};

/// Three seconds of a passage repeat nowhere: a tone that jumps every 120 ms, by a random walk over two octaves.
fn song(rate: f64, seconds: f64) -> Vec<f32> {
    let (mut state, mut phase, mut hertz) = (1u32, 0.0f64, 440.0f64);
    (0..(seconds * rate) as usize)
        .map(|i| {
            if i % (rate * 0.12) as usize == 0 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                hertz = 220.0 * 2f64.powf(f64::from(state >> 8) / f64::from(1u32 << 24) * 2.0);
            }
            phase += std::f64::consts::TAU * hertz / rate;
            (phase.sin() * 0.3 * (1.0 - (i as f64 / rate * 8.33).fract())) as f32
        })
        .collect()
}

fn bytes(pcm: &[f32]) -> Vec<u8> {
    pcm.iter().flat_map(|sample| sample.to_le_bytes()).collect()
}

#[test]
fn a_song_from_bytes_and_blocks_of_bytes_reach_a_final_lock() {
    let rate = 48_000.0;
    let pcm = song(rate, 30.0);
    let reference = SyncReference::from_pcm_f32le(bytes(&pcm), rate).unwrap();
    assert_eq!(reference.duration_ms(), 30_000.0);
    let session = SyncSession::new(reference, rate).unwrap();
    let (offset_ms, clock_ms) = (9_000.0, 1_000_000.0);
    for (index, block) in pcm[(offset_ms / 1_000.0 * rate) as usize..].chunks(960).enumerate() {
        let snapshot = session.push_f32le(bytes(block), clock_ms + index as f64 * 20.0).unwrap();
        assert_eq!(snapshot.reference_time_ms.is_some(), snapshot.capture_time_ms.is_some());
        if snapshot.is_final {
            assert_eq!(snapshot.state, SyncState::Locked);
            assert_eq!(snapshot.withhold_reason, None);
            let error = snapshot.reference_time_ms.unwrap() - (offset_ms + snapshot.capture_time_ms.unwrap() - clock_ms);
            assert!(error.abs() < 3.5, "{error}");
            return;
        }
    }
    panic!("the session never locked");
}

#[test]
fn a_new_session_has_heard_nothing() {
    let rate = 48_000.0;
    let reference = SyncReference::from_pcm_f32le(bytes(&song(rate, 1.0)), rate).unwrap();
    let session = SyncSession::new(reference, rate).unwrap();
    let snapshot = session.push_f32le(Vec::new(), 0.0).unwrap();
    assert_eq!(snapshot.state, SyncState::Ready);
    assert!(!snapshot.is_final);
    assert_eq!(snapshot.reference_time_ms, None);
    let snapshot = session.push_f32le(bytes(&[0.0; 4_800]), 0.0).unwrap();
    assert_eq!(snapshot.state, SyncState::Listening);
    assert_eq!(snapshot.withhold_reason, Some(SyncWithholdReason::InsufficientDuration));
}

#[test]
fn bad_input_is_an_error_and_not_a_panic() {
    let rate = 48_000.0;
    assert!(matches!(SyncReference::from_pcm_f32le(vec![0; 3], rate), Err(SyncError::Refused { .. })));
    assert!(matches!(SyncReference::from_pcm_f32le(bytes(&[0.0; 16]), rate), Err(SyncError::Refused { .. })));
    assert!(matches!(SyncReference::from_pcm_f32le(bytes(&[f32::NAN; 4_096]), rate), Err(SyncError::Refused { .. })));
    assert!(matches!(SyncReference::from_pcm_f32le(bytes(&[0.0; 4_096]), 0.0), Err(SyncError::Refused { .. })));
    let reference = SyncReference::from_pcm_f32le(bytes(&[0.0; 4_096]), rate).unwrap();
    assert!(matches!(SyncSession::new(reference.clone(), 1.5), Err(SyncError::Refused { .. })));
    let session = SyncSession::new(reference, rate).unwrap();
    assert!(matches!(session.push_f32le(vec![0; 5], 0.0), Err(SyncError::Refused { .. })));
    assert!(matches!(session.push_f32le(bytes(&[0.0; 16]), f64::INFINITY), Err(SyncError::Refused { .. })));
    assert!(matches!(session.push_f32le(bytes(&[f32::INFINITY; 16]), 0.0), Err(SyncError::Refused { .. })));
    assert_eq!(session.push_f32le(Vec::new(), 0.0).unwrap().state, SyncState::Ready);
}
