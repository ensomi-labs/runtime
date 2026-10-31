# Android performance round 1

## Before changes: existing K Pad traces

Extracted in place from the existing private artifacts; no original traces copied. Four distinct uncached chart runs, one observation each, unknown thermal/background load. Loading starts after some setup; these are not the requested repeated baseline. Percentages use chart.ready wall seconds. Rows includes arrangement, prior and runtime export; timing includes resampling, both Mels, BeatThis and grid fitting.

| Run | Total s | Loading s (%) | Timing s (%) | Rows + export s (%) |
| --- | ---: | ---: | ---: | ---: |
| 1 | 9.980 | 2.114 (21.2) | 6.517 (65.3) | 1.350 (13.5) |
| 2 | 38.177 | 9.617 (25.2) | 26.463 (69.3) | 2.087 (5.5) |
| 3 | 39.920 | 23.141 (58.0) | 14.225 (35.6) | 2.550 (6.4) |
| 4 | 21.601 | 5.469 (25.3) | 14.018 (64.9) | 2.115 (9.8) |

Initial order from these traces: split timing and loading, then target BeatThis and duplicate decoding. The repeated baseline below puts decoding first (58–59%), BeatThis second (28%). No optimization is adopted before its paired measurements and hash gate.

Baseline AAR copied to `artifacts/android-perf-1/baseline/ensomi-runtime.aar`, SHA-256 `71ad6397a71e0948a1a04b50702b56fd0e7c084d7bd5dee9942b83e82117825b`. Source AAR remains untouched.

## Scope and method

No quality metric is validated here. An unequal `.osu` hash is a failed tier 0 gate, even if only one object differs. Tier 1 requires reporting object additions/removals/changes followed by the owner's listening and play review on a blinded, representative song/star/seed panel. Small logit errors or unchanged difficulty estimates cannot establish quality.

Test request: star 4, seed 20261029. Each timing is a fresh instrumented process running the non-debuggable `checks` APK, signed with certificate `099c619b240f210f9da238654d03bc62e97b76ea82aa36158cfe028d5a7f0314`. The direct runtime check uses one session opening and `generate`; the recognition reuse check separately uses the app's actual `prepare` and whole-chart scheduler path (two session openings). Do not confuse either with time spent waiting for a download or recognition. Timed runs leave operator profiling off; separate profiles collect ORT worker participation and sampled CPU placement. Peak RSS is sampled every 50 ms, with VmHWM also reported for the direct check. Thermal status must be zero. A result at zero does not mean fixed frequency or zero competing system activity.

The tablet's CPU maximum-frequency groups are cores 0–3 at 2.4 GHz, 4–6 at 3.3 GHz, and 7 at 3.73 GHz. `performance_cores()` does not discover these on Android: it caps available parallelism at four. Explicit ORT thread counts leave workers without ORT affinity; the OS places them. Default sessions have one inter-op thread and sequential execution. Five graphs already run with one intra-op thread. Thus there is no six-graph default-pool oversubscription in the current build. ORT's worker spinning is enabled by default; its effect still needs measurement, not inference from thread count. See [ORT thread management](https://onnxruntime.ai/docs/performance/tune-performance/threading.html); the experiment uses pinned ORT 1.23.2, not newer documented spin-duration options.

## Implementation audit

- Recognition's `AudioProbe.probe` decodes mono PCM and hashes the file. `RecognitionController` retains that PCM for sync. `SessionCharts` originally gave only a URI to `RuntimeChartMaker`, which hashes and decodes again. The private candidate passes the retained decoding and matching hash directly to the maker; process-recovery/missing-PCM paths still decode normally. The same retained PCM remains available for sync. No global PCM cache is added.
- The current Kotlin helper already bulk-converts `FloatArray` to little-endian bytes; it does not construct `List<Float>`. The historical boxed-PCM issue is not present. UniFFI still copies bytes to Rust, and `pcm_f32le` allocates a float vector. Removing this safely would require an ownership/alignment design; the incoming byte allocation cannot simply be cast to a float vector with another allocation layout.
- Both Mels use the core's mixed-radix float64 FFT (1024 and 960 points), precomputed twiddles and sparse nonzero Mel weights. The soxr table resampler already uses four accumulators. Replacing the FFT, changing reduction order, enabling FMA/fast-math, or changing resampling is not assumed byte-preserving. `libm` is deliberate for cross-platform numerics.
- Rust's release build has no repository profile override: ordinary release optimization, 16 codegen units, no cross-crate LTO, generic `aarch64-linux-android`. Android's ORT build is Release `-O3`, CPU EP with KleidiAI. Thin LTO plus one Rust codegen unit is a separate candidate; no CPU-feature or fast-math flag is added. ORT dispatches its own CPU kernels independently of Rust target features.
- `lowMemory` disables the CPU arena and memory patterns in every graph. Allocation/release work can recur at every BeatThis chunk and every rows/R2 call. This explains a mechanism to test, not yet a measured attribution. Test arena and memory patterns independently before changing the public option's contract.

## Tiers 1 and 2: candidates, not adopted

All timings and object differences below are **unmeasured unless explicitly supplied later**. An unmeasured difference is not zero. Stage-time ceilings are bounds on potential savings, not forecasts; acceleration also adds setup, transfers and fallbacks. Memory estimates based on weights exclude activations and allocator retention.

| Candidate | What can improve / time bound | Memory / effort | Required check and limitation |
| --- | --- | --- | --- |
| Tier 1: XNNPACK | At most the time in supported graph operators; use measured graph times as the ceiling | Another ORT source build, provider registration and per-graph selection; medium effort, peak unknown | Disabled in the AAR and absent from the binding's provider enum. The README documents the disabled providers and the broken GNU-libstdc++ prebuilt archive, but does **not** give a separate measured reason for disabling XNNPACK. Its supported 2D Conv/Gemm/MatMul subset leaves attention, rotary and GRU work on CPU. Measure partition coverage, fallback and both thread pools. [Provider documentation](https://onnxruntime.ai/docs/execution-providers/Xnnpack-ExecutionProvider.html). |
| Tier 1: NNAPI / MediaTek NPU | Graph time is an upper bound; offload may lose to transfer/partition cost | NNAPI is not built or exposed; vendor SDK/plugin integration would be substantial, peak unknown | Enumerate devices and actual partition assignment on this tablet; disable NNAPI's CPU fallback for diagnostic runs. NNAPI was deprecated in Android 15, and the presence of an NPU proves no compatible driver or ORT backend. Qualcomm QNN is not a MediaTek backend. [Android status](https://developer.android.com/ndk/guides/neuralnetworks), [ORT NNAPI options](https://onnxruntime.ai/docs/execution-providers/NNAPI-ExecutionProvider.html). |
| Tier 1: GPU | BeatThis is the first graph to try; per-row graphs have little work per host round trip | New supported Android EP/build and buffers, high effort, peak unknown | CPU and Apple CoreML are the only exposed providers. CUDA/CoreML do not target this Android GPU. Measure partitioning, transfers and cold setup separately; preserve ONNX/backend boundary. |
| Tier 1: BeatThis INT8 / FP16 | Bounded by BeatThis time | 82.10 MB graph on disk; lower-precision weights could reduce its weight storage, not necessarily its large activations. Medium/high effort | Static quantization needs authorized calibration audio, per-op coverage and a converted local package. Keep graph I/O float32 because the runtime tensor adapter supports only f32/i64/bool. The pinned Android build also has `onnxruntime_ENABLE_CPU_FP16_OPS=OFF`; CPU FP16 is not a drop-in model-file swap. Test chunks, beats, fitted grids and complete hit objects; owner quality review follows. |
| Tier 1: rows encoder INT8 / FP16 | Bounded by the one encoder call | 2.28 MB graph, whole-song dynamic attention may dominate activations; medium effort | Dynamic beat/frame axes, local-attention masks, cumulative sums and normalization require fallback/cast inspection. Any changed head time propagates to R2. |
| Tier 1: rows step INT8 / FP16 | Bounded by all step calls | 12.41 MB graph including GRU, medium effort | Carry hidden state and copy history through the full song; compare sampled figures and full charts. Small teacher-forced numerical differences do not bound autoregressive differences. |
| Tier 1: R2 predict INT8 / FP16 | Bounded by predict calls | 0.68 MB graph, small savings ceiling, medium effort | Compare legal-support decisions and sampled actions; all 625 logits matter near ties. |
| Tier 1: R2 release INT8 / FP16 | Bounded by release calls | 0.50 MB graph, dynamic candidate count, medium effort | Each oriented release depends on previous choices. Compare hold end times and full objects, not just action heads. |
| Tier 1: R2 commit INT8 / FP16 | Bounded by commit calls | 8.02 MB graph, medium effort | Full carried temporal buffers must be exercised. Repeated small state differences can compound. |
| Tier 2: smaller/shorter-context BeatThis | Targets its measured dominant graph time | Smaller network/activations; high effort including new package and likely distillation/retraining | Current exporter produces dynamic-time float32 ONNX from a CPU model copy with rotary caching disabled. Runtime uses 1500-frame chunks, 6-frame borders, reverse overwrite aggregation. Changing context or export is a new model/package experiment, with grid and owner play review. |
| Tier 2: rows encoder local attention export | Targets dense attention work and whole-song activation peak | Potentially lower memory; new export and parity investigation, medium/high effort | Exporter's `_LocalAttention` computes a full score matrix and masks to the local window. Banded/sliced attention could avoid the dense matrix, but a new export is tier 2 here. Keep boundaries, validity and softmax support exact; no adoption in this round. |
| Tier 2: narrower rows / R2 networks | Targets encoder, step and arrangement graph time | Smaller weights/state; checkpoint work, export and calibration, high effort | Export supports released FiLM R2, optional landmarks, explicit GRU/TCN state. Width/depth changes need compatible schemas/state files and owner review of density, patterns and holds. No training or new export performed. |
| Tier 2: combine graph calls | Targets per-call overhead, bounded by the non-kernel part of graph time | Medium/high effort, memory uncertain | Cannot simply fuse predict→release→commit: the sampled action and sequential releases determine the committed history token. Rows are autoregressive too. Batching songs does not reduce one-song latency. |

Export facts above were read from ensomi-model `8f30b925f9e77d4b8e34fd4ca1436f73e3afe019`, [`src/ensomi_model/export`](https://github.com/ensomi-labs/ensomi-model/tree/8f30b925f9e77d4b8e34fd4ca1436f73e3afe019/src/ensomi_model/export) and its [ONNX guide](https://github.com/ensomi-labs/ensomi-model/blob/8f30b925f9e77d4b8e34fd4ca1436f73e3afe019/docs/guides/onnx_runtime.md). Existing graph sizes/node inventories were read from the released package only. No files in ensomi-model were changed. No sealed data, N1, model uploads or training were used.

### Cheap tier 1 probe: dynamic INT8 weight conversion

Local copies only, `quantize_dynamic`, per-channel signed INT8, MatMul/Gemm only; Conv and GRU stay float32. ONNX checker and Mac CPU session-load check passed for all six. This is neither an Android speed measurement nor a quality result. No inference on test songs was performed by this conversion check: changed hit-object counts and peak inference memory remain **unmeasured**. Files are ignored under `quantized/`.

| Graph | Original MB | Converted MB | MatMulInteger nodes |
| --- | ---: | ---: | ---: |
| beatthis | 82.099 | 22.885 | 62 |
| rows_encoder | 2.279 | 0.845 | 14 |
| rows_step | 12.413 | 11.052 | 9 |
| r2_predict | 0.683 | 0.215 | 9 |
| r2_release | 0.500 | 0.153 | 8 |
| r2_commit | 8.018 | 4.518 | 25 |

XNNPACK build probe: enabling it in an isolated copy of pinned ORT initially failed CMake generation: the `onnxruntimeTargets` install export referenced Abseil targets outside an export set. `ABSL_ENABLE_INSTALL=ON` did not resolve it: ORT explicitly turns that off for XNNPACK to avoid a duplicate target. `CMAKE_SKIP_INSTALL_RULES=ON` allowed the static build (no system installation is needed). The excluded `XNNPACK` and `pthreadpool` targets also had to be built explicitly before Rust could link them. This is build evidence, not yet evidence that XNNPACK runs the graphs.

## Measured base profile (before candidate runs)

| Stage seconds, mean (range) | Short | Medium | Long |
| --- | ---: | ---: | ---: |
| decode | 12.192 (11.888–12.484); 58.0% | 17.229 (17.069–17.353); 59.4% | 26.952 (19.506–30.927); 59.4% |
| hash | 0.007 (0.005–0.008); 0.0% | 0.011 (0.010–0.012); 0.0% | 0.012 (0.011–0.013); 0.0% |
| open | 0.789 (0.785–0.795); 3.8% | 0.778 (0.772–0.783); 2.7% | 0.793 (0.782–0.799); 1.7% |
| pcm | 0.414 (0.413–0.414); 2.0% | 0.559 (0.558–0.560); 1.9% | 1.059 (1.053–1.066); 2.3% |
| spectrogram | 0.130 (0.129–0.132); 0.6% | 0.175 (0.174–0.176); 0.6% | 0.289 (0.286–0.291); 0.6% |
| beatthis | 5.876 (5.853–5.918); 27.9% | 8.170 (8.160–8.181); 28.2% | 12.819 (12.805–12.845); 28.2% |
| mel | 0.420 (0.418–0.422); 2.0% | 0.572 (0.569–0.574); 2.0% | 0.948 (0.941–0.952); 2.1% |
| grid | 0.222 (0.220–0.224); 1.1% | 0.228 (0.227–0.229); 0.8% | 0.380 (0.379–0.381); 0.8% |
| rows | 0.129 (0.128–0.130); 0.6% | 0.149 (0.146–0.150); 0.5% | 0.239 (0.238–0.240); 0.5% |
| prior | 0.000 (0.000–0.000); 0.0% | 0.000 (0.000–0.000); 0.0% | 0.001 (0.000–0.001); 0.0% |
| arrangement | 0.616 (0.605–0.625); 2.9% | 0.861 (0.824–0.905); 3.0% | 1.547 (1.534–1.564); 3.4% |
| export | 0.001 (0.001–0.001); 0.0% | 0.001 (0.001–0.001); 0.0% | 0.002 (0.002–0.002); 0.0% |
| generate | 7.901 (7.862–7.948); 37.6% | 10.842 (10.802–10.894); 37.4% | 17.503 (17.480–17.536); 38.6% |
| close | 0.134 (0.133–0.135); 0.6% | 0.126 (0.125–0.129); 0.4% | 0.131 (0.129–0.135); 0.3% |
| total | 21.023 (20.686–21.300) | 28.986 (28.821–29.154) | 45.391 (37.988–49.348) |
| peakRssMB | 1831.253 (1827.332–1833.386) | 1841.043 (1840.628–1841.717) | 1923.030 (1920.332–1925.550) |
short n 3 hashes ['ef6aa0dacb63dc2afc9219b4f01c45c97a07698fc46177d37b34215dcf2d6a2f'] seconds [140.26448979591837]
medium n 3 hashes ['85e9317c3f63a282c95e14aec28035a2acabae44ef56ca21cb07174d195c22e0'] seconds [189.50530612244899]
long n 3 hashes ['eb06c87fabeefd04b75c0fc090806a86edbbd5ce5f24d31266f1a57a49bf93ce'] seconds [314.628]


The three inputs are the tablet's own MP3s: short 140.264 s at 44.1 kHz, medium 189.505 s at 44.1 kHz, long 314.628 s at 48 kHz. All nine baseline chart hashes match within each song. Their audio SHA-256 values are `f2f09f36a84584421af3543f613d5f351ab56c7b4748fd4fc8877bd8711103f2`, `a677044471e141b96b2873d6d37af8f8be2259d45ef8caa6cad995e5ebeefbdb`, and `49049c657475d9785218f3bb2957bdde411871ba775d16457a0f052f4cd5d620`, respectively. There are 1244, 1576 and 2902 baseline hit objects.

`pcm` is frontend resampling; `spectrogram` is BeatThis's log-Mel frontend; `mel` is the rows frontend plus derived features. `generate` is a subtotal, not another additive stage. Its excess over the receipt total includes the Kotlin/Rust PCM boundary and result conversion. `close` is session destruction, distinct from the R2 release graph. The native graph table below splits rows and arrangement further. Peak RSS is decimal MB and includes the app, not only ONNX weights.

The long song's decode range is large despite thermal status 0: 19.5–30.9 s. Its BeatThis range is only 0.040 s. This is why isolated before/after totals would not establish a gain. All candidate comparisons use adjacent A/B runs, with AB/BA/AB ordering when repeated three times. Operator-profile runs are excluded from ordinary timing means.

The decoder candidate fills available codec input slots without blocking before waiting for output. It preserves the chosen codec, input order, timestamps, gapless trimming and sample arithmetic. [MediaCodec's zero input timeout returns immediately](https://developer.android.com/reference/android/media/MediaCodec#dequeueInputBuffer(long)); the old loop waits after each compressed packet. This remains an experiment until PCM and chart hashes pass on the same songs.

Potential further tier 0 work, not yet measured: retain a generator across the app's prepare/chart calls to avoid repeated session opening (the direct baseline opens in 0.78–0.79 s). This needs explicit lifetime/cancellation ownership and memory measurements; it is not included in any claimed saving. The current `Sessions::load` contract loads all six graphs, so lazy loading would also need an error-timing/API review.
