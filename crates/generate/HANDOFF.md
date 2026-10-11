# Handoff: chart generator port (stages 1 to 3)

## State

Branch `core/v0`. Stage 1 is committed (c0e8b40). Stage 2 passes on all four songs
(`conformance/generate/wasm-parity.json`, committed). Stage 3 passes: `conformance/generate/same-as-ts.json` (13 cases, all equal; mean wall
seconds per chart in Node: TypeScript 26.6, Wasm 24.0, BeatThis about 18 of them). All three stages are done; what is left is in
the report to the human.

| Piece | Where | State |
| --- | --- | --- |
| Core: state machine, whole host pipeline | `crates/generate` (`ensomi-generate`) | done, no IO, no clock (injected `Options.clock`) |
| Native driver and replay binary | `crates/generate-ort` | stage 1 passes on all four songs: `conformance/generate/native-parity.json` |
| Wasm binding | `crates/generate-wasm`, `scripts/wasm-build.sh` | builds; wasm-bindgen-cli 0.2.129 is installed on the mac |
| JS API (`loadModels`, `generateChart`, `decodeAudio`, `localFilesSource`, `models.capabilities`) | `js/src` | written; used by the Wasm replay |
| Stage 2 check | `js/checks/replay.mjs` -> `conformance/generate/wasm-parity.json` | all four songs pass (BeatThis 1.06e-4 to 1.17e-4 on ORT-web, inside the 2e-4 allowance) |
| Stage 3 check | `js/checks/same-as-ts.mjs` -> `conformance/generate/same-as-ts.json` | passes |

## Next steps

1. Commit anything the mac wrote that is still untracked under `conformance/generate/`.
2. Open points: where the 17 MB soxr tables live for `loadModels`' default `runtimeSource` (`js/assets/` is absent);
   a browser run of the JS API; `wasm-opt` is not installed on the mac (the `.wasm` is 1.2 MB unoptimised by binaryen);
   resampling 48 kHz PCM in the core takes 1 to 3 s natively (`pcm` in the receipt), more in Wasm.

## Decisions taken (list them in the report)

- No clock in the core: `Options.clock` (ms) is injected, like the PCM; the Wasm binding passes `performance.now`.
  Receipt `seconds` and `wall_seconds` are wall-clock and excluded from the TS comparison.
- `Math.*` ported (`jsmath.rs`): fdlibm with the fused multiply-adds that Node on arm64 uses; bit-equal to Node 26 on
  3.2 M inputs. System libm and the `libm` crate are not.
- Receipt writer = `JSON.stringify(v, null, 2)` (`json.rs`, ES number formatting, insertion order).
- Runtime name in receipt and `.osu` comment is `ensomi-runtime` (`RUNTIME_NAME`); version `0.1.0` as TS.
- A BigInt seed prints as a JSON string in the receipt as in TS (`Request.seed_is_bigint`).
- `.osz` is built in JS with fflate (not in the core); `decodeAudio` is the TS code unchanged.
- The soxr tables (17 MB) are not committed: `Audio.resampler` carries them; JS `runtimeSource` reads them, default
  `js/assets/` (absent). Tonight's checks read `ensomi-web/packages/runtime/assets`.
- BeatThis graph tolerance 2e-4 instead of 1e-4: ort 2.0.0-rc.11 (ONNX Runtime 1.23.2, same version as
  onnxruntime-node) gives 1.35e-4 on catch-the-moment where onnxruntime-node gave 8.2e-5. No session option
  (kleidiai off, optimisation levels, deterministic) gets below 1.06e-4. Every downstream criterion still passes
  (grid within 1e-6 ms, identical heads, prior exactly equal, 898 identical decisions, identical hit objects).
- Not ported: `options.trace` and `options.random` of the TS API (the core has `Options.trace` for conformance and a
  `ReplayDraws` for recorded draws; the JS API exposes only `options.replay`).

## Risks

- The mac's checkout is the only place anything is built; `js/pkg` (generated) and `js/node_modules` are ignored.
- The JS API is not exercised in a browser; `loadCore` uses `fetch` for the `.wasm` by default, Node needs `options.core`.
- Wasm uses software `fma` for the fused ports; transcendental-heavy loops (spectrogram) are slower than native.
