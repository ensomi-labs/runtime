# Chart generator conformance

The generator (`crates/generate`) reproduces the TypeScript runtime and the Python release from recorded
inputs. Paths below are relative to `~/ensomi` on bings-mac; run the commands in `ensomi-runtime`.

| File | What it pins |
| --- | --- |
| `native-parity.json` | `crates/generate-ort/src/bin/parity.rs`: the four release recordings replayed natively. |
| `wasm-parity.json` | `js/checks/replay.mjs`: the same replay through the Wasm build and onnxruntime-web (WASM backend) in Node. |
| `same-as-ts.json` | `js/checks/same-as-ts.mjs`: charts, `.osu` and receipts against the TypeScript runtime, seeds 0, 1 and 2. |
| `jsmath_cases.mjs` | Node's `Math` results for `crates/generate/examples/jsmath_check.rs`, which compares the fdlibm port bit for bit. |

Large intermediate files go under `bulk/`, which is never mirrored.

## Native replay

```sh
CARGO_TARGET_DIR=target/generate cargo run --release -p ensomi-generate-ort --bin parity -- \
  --model-dir ../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
  --reference ../ensomi-model/artifacts/runtime-ref \
  --assets ../ensomi-web/packages/runtime/assets \
  --out conformance/generate/native-parity.json
```

Criteria: every recorded graph call within 1e-4 (BeatThis 2e-4), grid positions within 1e-4 ms with the
same whole milliseconds and Mel frames, identical head times, prior exactly equal, identical sampled
choices for every recorded draw, identical hit objects, timing points within 1e-6 ms, all other `.osu`
text identical.

## Wasm replay and equality with TypeScript

```sh
scripts/wasm-build.sh
cd js && npm install && cd ..
node js/checks/replay.mjs --model-dir ../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
  --reference ../ensomi-model/artifacts/runtime-ref --assets ../ensomi-web/packages/runtime/assets \
  --core js/pkg/ensomi_generate_wasm_bg.wasm --out conformance/generate/wasm-parity.json
cd ../ensomi-web/packages/runtime && node --import tsx ../../../ensomi-runtime/js/checks/same-as-ts.mjs \
  --model-dir ../../../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
  --reference ../../../ensomi-model/artifacts/runtime-ref --assets assets \
  --core ../../../ensomi-runtime/js/pkg/ensomi_generate_wasm_bg.wasm \
  --out ../../../ensomi-runtime/conformance/generate/same-as-ts.json
```

## Numerics

The TypeScript runtime's `Math.exp`, `log`, `log1p`, `log2`, `sin`, `cos`, `atan2` and `hypot` are V8's fdlibm
ports (`hypot` is V8's own scaled sum). Node on Apple silicon returns results that clang compiled with fused
multiply-adds (`a * b + c` within one expression becomes one rounding), so `crates/generate/src/jsmath.rs`
ports the functions with the same fusions (`f64::mul_add`); it matches Node's results on 3.2 million random
inputs (`jsmath_cases.mjs`, `jsmath_check.rs`). The system libm differs from Node's in the last bit for 0.4 to
42 percent of inputs depending on the function, and the `libm` crate for 0 to 41 percent; those last bits reach
the receipt. Node on another CPU may round differently, so the TypeScript runtime's receipts are not
guaranteed bit-stable across machines; the Rust core follows Apple silicon.
