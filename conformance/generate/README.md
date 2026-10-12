# Chart generator conformance

The generator (`crates/generate`) reproduces the TypeScript runtime and the Python release from recorded
inputs. Paths below are relative to `~/ensomi` on bings-mac; run the commands in `ensomi-runtime`.

| File | What it pins |
| --- | --- |
| `native-parity.json` | `crates/generate-ort/src/bin/parity.rs`: the four release recordings replayed natively. |
| `wasm-parity.json` | `js/checks/replay.mjs`: the same replay through the Wasm build and onnxruntime-web (WASM backend) in Node. |
| `native-parity-stream.json`, `wasm-parity-stream.json` | The same two replays with `--stream`: R2's arrangement committed per head row, from a `Prepared`. |
| `same-as-ts.json` | `js/checks/same-as-ts.mjs`: charts, `.osu` and receipts against the TypeScript runtime, seeds 0, 1 and 2, with V8's `Math` ported (before the core moved to `libm`), while both arranged a song a second time where the Python release did. ensomi-web's `npm run same-as-ts` now runs this comparison. |
| `apple-providers.json` | `crates/generate-ort/examples/providers.rs`: time, memory and chart equality of ONNX Runtime's CPU and Core ML providers on the four songs. |
| `native-charts/` | The same run's CPU chart and request for hesitation-snow, which `swift test` reproduces through the Swift API. |

Large intermediate files go under `bulk/`, which is never mirrored.

## Native replay

```sh
CARGO_TARGET_DIR=target/generate cargo run --release -p ensomi-generate-ort --bin parity -- \
  --model-dir ../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
  --reference ../ensomi-model/artifacts/runtime-ref \
  --assets assets/resampler \
  --out conformance/generate/native-parity.json
```

Criteria: every recorded graph call within 1e-4 (BeatThis 2e-4), grid positions within 1e-4 ms with the
same whole milliseconds and Mel frames, identical head times, prior exactly equal, identical sampled
choices for every recorded draw but those of a second arrangement, one arrangement in the receipt, identical hit
objects, timing points within 1e-6 ms, all other `.osu` text identical.

The runtime arranges a song once. The release arranged saigetsu and catch-the-moment a second time (`r2_pass_2/...`
in the recording), as its difficulty check asked: there those draws are left, and the chart is checked against the
release's first arrangement (`r2_pass_1/...`) instead of its chart, decision by decision natively and by the hit
objects the recorded decisions make in the Wasm replay. Hesitation-snow and adventure, which the release arranged
once, must give the release's chart.

With `--stream`, the generation commits a page per head row from a `Prepared` made by its own run of the audio
stages (`prepare`, then `generate_prepared`), as play while generating does. The graphs are not replayed again.
Criteria: stages 2 to 5 as above; the pages make the chart (one per head row and the final row, rising coverage, one
end of stream, their tokens the whole chart's); the own PRNG run, from the same `Prepared`, finalised and its pages
making its chart.

```sh
target/generate/release/parity --stream --model-dir ../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
  --reference ../ensomi-model/artifacts/runtime-ref --assets assets/resampler \
  --out conformance/generate/native-parity-stream.json
```

`node js/checks/replay.mjs --stream ...` does the same through the JavaScript `ChartScheduler`.

## Providers on Apple silicon

```sh
CARGO_TARGET_DIR=target/apple cargo build --release -p ensomi-generate-ort --features coreml --example providers
target/apple/release/examples/providers --model-dir ../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
  --reference ../ensomi-model/artifacts/runtime-ref --assets assets/resampler \
  --out conformance/generate/apple-providers.json --charts conformance/generate/bulk/charts
cp conformance/generate/bulk/charts/hesitation-snow.* conformance/generate/native-charts/
```

The input is what a client passes: the recorded PCM averaged to mono float32 at its recorded rate, without
the recorded frontend streams, with the release example's star, seed, title and artist. Each configuration
and song runs in a process of its own, which generates twice.

## Wasm replay and equality with TypeScript

```sh
scripts/wasm-build.sh
cd js && npm install && cd ..
node js/checks/replay.mjs --model-dir ../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
  --reference ../ensomi-model/artifacts/runtime-ref --assets assets/resampler \
  --core js/pkg/ensomi_generate_wasm_bg.wasm --out conformance/generate/wasm-parity.json
cd ../ensomi-web/packages/runtime && node --import tsx ../../../ensomi-runtime/js/checks/same-as-ts.mjs \
  --model-dir ../../../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
  --reference ../../../ensomi-model/artifacts/runtime-ref --assets assets \
  --core ../../../ensomi-runtime/js/pkg/ensomi_generate_wasm_bg.wasm \
  --out ../../../ensomi-runtime/conformance/generate/same-as-ts.json
```

## Time to first playable

```sh
CARGO_TARGET_DIR=target/generate cargo build --release -p ensomi-runtime-ffi --example stream
target/generate/release/examples/stream --model-dir ../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
  --reference ../ensomi-model/artifacts/runtime-ref --out <native.json>
node js/checks/stream.mjs --model-dir ../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
  --reference ../ensomi-model/artifacts/runtime-ref --assets assets/resampler \
  --core js/pkg/ensomi_generate_wasm_bg.wasm --out <wasm.json>
```

Seconds from the start of a generation to the scheduler's first `Ready` at 0 and to `Done`, playing while generating
against the whole chart, from PCM and from stored `Prepared` bytes, three runs each, natively through the UniFFI
binding and in Node through the JavaScript API; and the throughput r.

## Memory in Node

```sh
node --expose-gc js/checks/memory.mjs --model-dir ../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
  --reference ../ensomi-model/artifacts/runtime-ref --core js/pkg/ensomi_generate_wasm_bg.wasm [--browser] adventure
```

Two generations in a row through the Wasm core. Its linear memory never shrinks, so after a generation it is the
peak so far. On adventure (200 s), 65.8 MiB after loading the model, then after either generation 174.3 MiB for
96 kHz mono PCM and 101.0 MiB with `--browser` (the streams `decodeAudio` gives); 285.6 and 119.3 MiB before the
core read mono PCM where it is, took the browser's Mel stream once and dropped buffers it no longer reads
(2026-10-12).

## Numerics

The core computes `exp`, `log`, `log1p`, `log2`, `sin`, `cos`, `atan2` and `hypot` with the `libm` crate: pure
Rust without fused multiply-adds, so the native and the WebAssembly builds give the same bits on every platform.
The TypeScript runtime used V8's `Math`, whose last bits follow the machine (Node on Apple silicon fuses
multiply-adds, which WebAssembly can only emulate slowly). On the four songs with seeds 0 to 2 and the controls
case, charts, `.osu` and receipts still equalled the TypeScript runtime's (ensomi-web's `npm run same-as-ts`) while
this runtime, like it, arranged a song a second time where the difficulty check asked; they now differ wherever the
TypeScript runtime arranges a song twice. On other input a last-bit difference can show in receipt digits or, at a
near-tie, in a sampled choice. The Python release stays the reference: the replays above compare with its recorded
values.
