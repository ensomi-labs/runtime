//! Frames the Swift app's extractor made from a small synthetic signal (`conformance/sync/README.md`).

use ensomi_sync::frames_f32le;
use serde_json::Value;

/// Rounding between vDSP's single-precision FFT and this crate's double-precision one, and a platform's `powf` on top.
/// On the Mac that wrote the fixture the largest difference is 7.2e-7, and on recordings 2.4e-6.
const PCEN_TOLERANCE: f64 = 1e-5;
// These matched exactly on the Mac; allow only double-precision rounding elsewhere.
const ENERGY_TOLERANCE_DB: f64 = 1e-12;
const TIME_TOLERANCE_MS: f64 = 1e-12;

#[test]
fn frames_match_the_swift_extractor() {
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/../../conformance/sync");
    let pcm = std::fs::read(format!("{directory}/frames.f32")).unwrap();
    let expected: Value = serde_json::from_slice(&std::fs::read(format!("{directory}/frames.json")).unwrap()).unwrap();
    let expected = expected["frames"].as_array().unwrap();
    let frames = frames_f32le(&pcm, 48_000.0).unwrap();
    assert_eq!(frames.len(), 16);
    assert_eq!(frames.len(), expected.len());
    for (index, (frame, expected)) in frames.iter().zip(expected).enumerate() {
        assert!((frame.energy_dbfs - expected["energyDBFS"].as_f64().unwrap()).abs() <= ENERGY_TOLERANCE_DB, "frame {index}");
        assert!((frame.recorded_time_ms - expected["recordedTimeMS"].as_f64().unwrap()).abs() <= TIME_TOLERANCE_MS, "frame {index}");
        let bands = expected["pcenMel"].as_array().unwrap();
        assert_eq!(bands.len(), frame.pcen_mel.len());
        for (band, (&ours, theirs)) in frame.pcen_mel.iter().zip(bands).enumerate() {
            let theirs = theirs.as_f64().unwrap();
            assert!((f64::from(ours) - theirs).abs() <= PCEN_TOLERANCE, "frame {index}, band {band}: {ours} and {theirs}");
        }
    }
}
