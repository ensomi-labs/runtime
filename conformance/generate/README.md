# Chart generator conformance

The generator (`crates/generate`) reproduces the TypeScript runtime and the Python release from recorded
inputs. Paths below are relative to `~/ensomi` on bings-mac; run the commands in `ensomi-runtime`.

| File | What it pins |
| --- | --- |
| `native-parity.json` | `crates/generate-ort/src/bin/parity.rs`: the four release recordings replayed natively. |
| `wasm-parity.json` | `js/checks/replay.mjs`: the same replay through the Wasm build and onnxruntime-web (WASM backend) in Node. |
| `same-as-ts.json` | `js/checks/same-as-ts.mjs`: charts, `.osu` and receipts against the TypeScript runtime, seeds 0, 1 and 2, with V8's `Math` ported (before the core moved to `libm`). ensomi-web's `npm run same-as-ts` now runs this comparison. |
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
choices for every recorded draw, identical hit objects, timing points within 1e-6 ms, all other `.osu`
text identical.

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

## Numerics

The core computes `exp`, `log`, `log1p`, `log2`, `sin`, `cos`, `atan2` and `hypot` with the `libm` crate: pure
Rust without fused multiply-adds, so the native and the WebAssembly builds give the same bits on every platform.
The TypeScript runtime used V8's `Math`, whose last bits follow the machine (Node on Apple silicon fuses
multiply-adds, which WebAssembly can only emulate slowly). On the four songs with seeds 0 to 2 and the controls
case, charts, `.osu` and receipts still equal the TypeScript runtime's (ensomi-web's `npm run same-as-ts`); on
other input a last-bit difference can show in receipt digits or, at a near-tie, in a sampled choice. The Python
release stays the reference: the replays above compare with its recorded values.
