# Ensomi runtime

One Rust implementation of the logic every Ensomi client shares: the gameplay rules, the chart
sources and the chart generator. The macOS, iOS, Android, Windows and web clients bind to it
and keep only what is specific to their platform.

## Boundary

| Layer | Holds | Lives in |
| --- | --- | --- |
| Messages between programs | chart, generation, capabilities, chart stream (unreleased) | [EnsomiProtocol](https://github.com/ensomi-labs/protocol) |
| Model package | the manifest `ensomi-package.json`, format `ensomi-package/1` ([`docs/model-package.md`](docs/model-package.md)), which this runtime alone interprets; `runtime.json` and the files it names | the released model's `onnx/` directory (`sed-i/ensomi-r2`) |
| Conformance fixtures | inputs and expected outputs any implementation must reproduce | `conformance/` |
| Core | gameplay rules with their ruleset, input events, judgement events and score, clock and offset arithmetic, note layout, chart sources, the generator's host pipeline, live audio sync for Android | `crates/` |
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
| Android 13 (API 33), arm64-v8a | local AAR `ensomi-runtime.aar` ([`android/`](android)); built on macOS only for now | `scripts/build-android.sh` |
| Web (Chrome 91, Firefox 89, Safari 16.4), Node | npm `@ensomi/runtime` ([`js/`](js)) | `scripts/wasm-build.sh` |

The Wasm core is built with WebAssembly SIMD (`simd128`), which onnxruntime-web's own Wasm needs as well; the browser
versions are the first with it.

The packages carry the soxr tables in [`assets/resampler`](assets/resampler), with which the generator resamples PCM at
rates other than 22050 Hz. Releases: [`RELEASING.md`](RELEASING.md).

They also run the audio stages alone (`prepare`, whose result is stored as bytes per song) and a scheduler that
generates for play: R2's arrangement committed per head row, and play allowed to start once it cannot overtake the
chart ([`swift/README.md`](swift/README.md), [`js/README.md`](js/README.md)). The scheduler is in the core without IO
(`crates/generate/src/schedule.rs`); natively its pages go into the gameplay session inside Rust.

Live audio sync is in the core too (`crates/sync`): the features and session around
[Sonalign](https://github.com/ensomi-labs/sonalign), pinned exactly, for clients that do not have the Swift app's. Only
the Android build carries it ([`android/README.md`](android/README.md)); Apple apps link Sonalign as a Swift package.

Requests during generation: the scheduler takes a new request for the chart in progress (`Event::Request`), for play
in which game events change the coming chart, with or without Sonalign: a multiplayer attack on another player's
chart, players coordinating. Only `star` and `controls` may change, from the first uncommitted head row on; committed
pages never do. A pipeline that resumes from a committed prefix will apply such requests. R2 cannot yet: it takes a
request until it commits the first page, starting the chart stages again from the song's `Prepared` if they have begun,
so the chart is the one a fresh generation with the request makes. Once a page is committed it refuses one, leaving the
generation unchanged.

The generator arranges a song once, whether it commits per head row or the whole chart at the end, so both give the
same chart. The published Python R2 (`r2-1.0`) arranged a song a second time when the first arrangement missed the
star's difficulty target by more than the difficulty fit's seed spread; where it did, its chart differs from this
runtime's. The receipt's `controls.difficulty` records the miss.

The TypeScript runtime in `ensomi-web/packages/runtime` is retired: ensomi-web serves the Wasm build of this core.
A comparison with its charts holds only where it arranged the song once, as this runtime always does.

## Android build (macOS only for now)

```sh
scripts/build-android.sh
```

Builds `build/android/ensomi-runtime.aar` for arm64-v8a and keeps its generated Kotlin source beside it.
The AAR carries the native libraries, resampler tables and ONNX Runtime notices. The app consumes this one
local file and declares JNA 5.19.1 and kotlinx-coroutines-core 1.11.0. There is no Android CI or release asset yet.
[Android build and usage](android/README.md) covers prerequisites, the byte PCM APIs, the 8 MiB worker
helper and the emulator tests.

## Licence

Apache-2.0. The model published at `sed-i/ensomi-r2` is licensed MIT, as its
[card](https://huggingface.co/sed-i/ensomi-r2) states; BeatThis (CPJKU) keeps its own MIT notice
(`onnx/BeatThis-LICENSE.txt`) for `onnx/beatthis.onnx`, an ONNX export of its `final0` checkpoint, and its front-end
arrays `onnx/beatthis-mel-filter.bin` and `onnx/beatthis-hann-window.bin`. The package's manifest names that file as a
licence notice, one of the files a client downloads with the model, for the app to show: `checkPackage` in Swift and
JavaScript, and `models.notices` from `loadModels`. `runtime.json` names it too, which `modelFiles` and `noticeFiles`
read for a directory without the manifest.
