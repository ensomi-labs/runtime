//! The chart generator through the bindings, with the released model read from beside the repository where
//! bings-mac keeps it (conformance/generate/README.md); without it the test passes without running.

use ensomi_runtime_ffi::generator::{
    ChartControls, ChartGenerator, ExecutionProvider, GenerateChartRequest, GenerationListener, GenerationPhase,
    GeneratorError, GeneratorOptions,
};
use std::path::Path;
use std::sync::Arc;

/// A listener that panics when the generation reaches the rows, as a bug in the core would.
struct PanicsAtRows;

impl GenerationListener for PanicsAtRows {
    fn progress(&self, phase: GenerationPhase) {
        assert_ne!(phase, GenerationPhase::Rows, "a listener that panics");
    }
}

/// Reproducible noise at 22050 Hz, the rate BeatThis reads without a resampler table.
fn noise(seconds: usize) -> Vec<f32> {
    let mut state = 1u32;
    (0..seconds * 22_050)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / (1 << 24) as f32 - 0.5
        })
        .collect()
}

#[test]
fn a_panic_ends_its_generation_with_an_error_and_the_next_generation_works() {
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2/onnx");
    if !model.exists() {
        eprintln!("skipped: {} is absent", model.display());
        return;
    }
    let options = GeneratorOptions { provider: ExecutionProvider::Cpu, resampler_directory: None, low_memory: None };
    let generator = ChartGenerator::open(model.to_string_lossy().into_owned(), options).unwrap();
    let request = GenerateChartRequest { star: 3.0, seed: 0, controls: ChartControls { long_notes: None, sections: Vec::new() }, title: None, artist: None, audio_name: None };

    match generator.generate(noise(20), 22_050.0, request.clone(), Some(Arc::new(PanicsAtRows)), None) {
        Err(GeneratorError::Generation { code, message }) => {
            assert_eq!(code, "generation_failed");
            assert!(message.contains("a listener that panics"), "{message}");
        }
        other => panic!("the generation did not fail with the panic: {other:?}"),
    }
    let result = generator.generate(noise(20), 22_050.0, request, None, None).unwrap();
    assert!(!result.chart.objects.is_empty());
}
