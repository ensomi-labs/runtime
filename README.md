# Ensomi runtime

One Rust implementation of the logic every Ensomi client shares: the gameplay rules, the chart
sources and the chart generator. The macOS, iOS, Android, Windows and web clients bind to it
and keep only what is specific to their platform.

## Boundary

| Layer | Holds | Lives in |
| --- | --- | --- |
| Contracts | chart, generation, capabilities, model package, gameplay ruleset, input log, replay, result, chart stream | [EnsomiProtocol](https://github.com/ensomi-labs/protocol) |
| Conformance fixtures | inputs and expected outputs any implementation must reproduce | `conformance/` |
| Core | gameplay rules, clock and offset arithmetic, note layout, chart sources, the generator's host pipeline | `crates/` |
| Compute | the published ONNX graphs, run by ONNX Runtime with the platform's execution provider | each binding |
| Platform shell | audio codec, audio output and its clock, timestamped input, display link, rendering, UI, storage, model download | each app |

Rules that keep the boundary:

- **The core does no IO.** It never reads a clock, opens a file, decodes audio or runs a graph.
  The caller passes in time, timestamped input, PCM and graph outputs. The generator yields a
  request to run a named graph on given tensors and resumes on the outputs, so the same code
  serves synchronous ONNX Runtime on native platforms and asynchronous ONNX Runtime Web in the
  browser.
- **Judgement depends on the chart, the ruleset and the input log only**, never on how often the
  caller advances time.
- **Resampling and feature extraction are in the core**; only the codec differs per platform.
- **Each platform draws the playfield natively** from the core's note layout.
- **Models stay ONNX.** A compute backend is an execution-provider choice, not code here.

## Packages

| Clients | Package | Built by |
| --- | --- | --- |
| macOS 14, iOS 17 | Swift package `EnsomiRuntime` ([`Package.swift`](Package.swift), [`swift/`](swift)); the xcframework is a release asset | `scripts/build-apple.sh` |
| Web (Chrome 91, Firefox 89, Safari 16.4), Node | npm `@ensomi/runtime` ([`js/`](js)) | `scripts/wasm-build.sh` |

The Wasm core is built with WebAssembly SIMD (`simd128`), which onnxruntime-web's own Wasm needs as well; the browser
versions are the first with it.

Both packages carry the soxr tables in [`assets/resampler`](assets/resampler), with which the generator resamples PCM at
rates other than 22050 Hz. Releases: [`RELEASING.md`](RELEASING.md).

Both also run the audio stages alone (`prepare`, whose result is stored as bytes per song) and a scheduler that
generates for play: R2's arrangement committed per head row, and play allowed to start once it cannot overtake the
chart ([`swift/README.md`](swift/README.md), [`js/README.md`](js/README.md)). The scheduler is in the core without IO
(`crates/generate/src/schedule.rs`); natively its pages go into the gameplay session inside Rust.

The generator arranges a song once, whether it commits per head row or the whole chart at the end, so both give the
same chart. The published Python R2 (`r2-1.0`) arranged a song a second time when the first arrangement missed the
star's difficulty target by more than the difficulty fit's seed spread; where it did, its chart differs from this
runtime's. The receipt's `controls.difficulty` records the miss.

The TypeScript runtime in `ensomi-web/packages/runtime` is retired: ensomi-web serves the Wasm build of this core.
A comparison with its charts holds only where it arranged the song once, as this runtime always does.

## Licence

Apache-2.0. The model weights published at `sed-i/ensomi-r2` carry their own licence
(CC BY-NC-SA 4.0).
