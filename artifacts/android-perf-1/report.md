# Android performance round 1

## Scope and method

No quality metric is validated here. An unequal `.osu` hash is a failed tier 0 gate, even if only one object differs. Tier 1 requires reporting object additions/removals/changes followed by the owner's listening and play review on a blinded, representative song/star/seed panel. Small logit errors or unchanged difficulty estimates cannot establish quality.

Test request: star 4, seed 20261029. Each timing is a fresh instrumented process running the non-debuggable `checks` APK, signed with certificate `099c619b240f210f9da238654d03bc62e97b76ea82aa36158cfe028d5a7f0314`. The direct runtime check uses one session opening and `generate`; the recognition reuse check separately uses the app's actual `prepare` and whole-chart scheduler path (two session openings). Do not confuse either with time spent waiting for a download or recognition. Timed runs leave operator profiling off; separate profiles collect ORT worker participation and sampled CPU placement. Peak RSS is sampled every 50 ms, with VmHWM also reported for the direct check. The app-path peak covers the chart-making interval after capability discovery, not the initial probe. Its initial probe reproduces the successful decode/hash work but omits AudioProbe’s metadata-retriever and file-stat checks; no microphone, recognition UI or ambient-play end-to-end run is claimed. Thermal status must be zero. A result at zero does not mean fixed frequency or zero competing system activity.

The tablet's CPU maximum-frequency groups are cores 0–3 at 2.4 GHz, 4–6 at 3.3 GHz, and 7 at 3.73 GHz. `performance_cores()` does not discover these on Android: it caps available parallelism at four. Explicit ORT thread counts leave workers without ORT affinity; the OS places them. Default sessions have one inter-op thread and sequential execution. Five graphs already run with one intra-op thread. Thus there is no six-graph default-pool oversubscription in the current build. ORT's worker spinning is enabled by default; the medium-song no-spin screen below is slower. See [ORT thread management](https://onnxruntime.ai/docs/performance/tune-performance/threading.html); the experiment uses pinned ORT 1.23.2, not newer documented spin-duration options.

The app pins model `r2-1.0.1`, manifest SHA-256 `6e05317427cc24f3d46c0e8dcdd7a44078954437129c60f8797acb80bea6a1fb`. Neither this pin nor the installed package is changed. Experimental graph copies use separate evidence directories.

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


The three inputs are the tablet’s own MP3s: short is The Gypsy Bard (140.264 s, 44.1 kHz), medium is Maroon 5 — Memories (189.505 s, 44.1 kHz), and long is Astilbe x arendsii — lettRE (314.628 s, 48 kHz). Only `.mp3` files under the authorized Music directory are used; no `.ncm` is opened. All nine baseline chart hashes match within each song. Their audio SHA-256 values are `f2f09f36a84584421af3543f613d5f351ab56c7b4748fd4fc8877bd8711103f2`, `a677044471e141b96b2873d6d37af8f8be2259d45ef8caa6cad995e5ebeefbdb`, and `49049c657475d9785218f3bb2957bdde411871ba775d16457a0f052f4cd5d620`, respectively. There are 1244, 1576 and 2902 baseline hit objects.

`pcm` is frontend resampling; `spectrogram` is BeatThis's log-Mel frontend; `mel` is the rows frontend plus derived features. `generate` is a subtotal, not another additive stage. Its excess over the receipt total includes the Kotlin/Rust PCM boundary and result conversion. `close` is session destruction, distinct from the R2 release graph. The native graph table below splits rows and arrangement further. Peak RSS is decimal MB and includes the app, not only ONNX weights.

The long song's decode range is large despite thermal status 0: 19.5–30.9 s. Its BeatThis range is only 0.040 s. This is why isolated before/after totals would not establish a gain. All candidate comparisons use adjacent A/B runs, with AB/BA/AB ordering when repeated three times. Operator-profile runs are excluded from ordinary timing means.

The decoder candidate fills available codec input slots without blocking before waiting for output. It preserves the chosen codec, input order, timestamps, gapless trimming and sample arithmetic. [MediaCodec's zero input timeout returns immediately](https://developer.android.com/reference/android/media/MediaCodec#dequeueInputBuffer(long)); the old loop waits after each compressed packet. The PCM and chart hashes pass, but the measured slowdown below rejects this candidate.

Potential further tier 0 work, not yet measured: retain a generator across the app's prepare/chart calls to avoid repeated session opening (the direct baseline opens in 0.78–0.79 s). This needs explicit lifetime/cancellation ownership and memory measurements; it is not included in any claimed saving. The current `Sessions::load` contract loads all six graphs, so lazy loading would also need an error-timing/API review.


| Song | Original-AAR direct chart SHA-256 |
| --- | --- |
| short | `ef6aa0dacb63dc2afc9219b4f01c45c97a07698fc46177d37b34215dcf2d6a2f` |
| medium | `85e9317c3f63a282c95e14aec28035a2acabae44ef56ca21cb07174d195c22e0` |
| long | `eb06c87fabeefd04b75c0fc090806a86edbbd5ce5f24d31266f1a57a49bf93ce` |

### Graph split, three runs per song

| Graph seconds, mean (range); calls/run; share of whole | Short | Medium | Long |
| --- | ---: | ---: | ---: |
| beatthis | 5.8531 (5.8389–5.8675); 5; 28.13% | 8.1734 (8.1650–8.1876); 7; 28.43% | 12.8657 (12.8573–12.8816); 11; 26.31% |
| rows_encoder | 0.0454 (0.0448–0.0465); 1; 0.22% | 0.0601 (0.0596–0.0603); 1; 0.21% | 0.1042 (0.1030–0.1063); 1; 0.21% |
| rows_step | 0.0448 (0.0430–0.0472); 282; 0.22% | 0.0466 (0.0453–0.0477); 289; 0.16% | 0.0701 (0.0688–0.0724); 437; 0.14% |
| r2_predict | 0.0507 (0.0493–0.0519); 808; 0.24% | 0.0705 (0.0672–0.0744); 1119; 0.25% | 0.1309 (0.1249–0.1367); 2050; 0.27% |
| r2_release | 0.0106 (0.0103–0.0108); 82; 0.05% | 0.0230 (0.0227–0.0233); 199; 0.08% | 0.0465 (0.0458–0.0477); 464; 0.10% |
| r2_commit | 0.5252 (0.5106–0.5478); 807; 2.52% | 0.7467 (0.7162–0.7783); 1118; 2.60% | 1.3805 (1.3233–1.4560); 2049; 2.82% |

These are separate aggregate-timing runs, with operator profiling off. All nine charts equal the original-AAR baselines. `rows` and `arrangement` in the receipt also contain host feature construction, sampling and state handling; their graph sums therefore do not replace the entire phase. The corpus prior takes about 0.00014–0.00051 s and is not a material target.

### Actual threads and CPU placement

One separate ORT operator profile plus 50 ms `/proc/self/task/*/stat` sampling per song; charts still match. These profiles add overhead and are not the timing baseline.

| Work | Threads actually observed | CPU placement in the three profiles |
| --- | --- | --- |
| Decode | Calling test thread, MediaCodec loop and Binder helpers | Calling thread and codec mostly cores 0–3, with work on 4–7 too |
| Open, frontend resampling, Mel/grid | Runtime caller | Sampled runtime CPU ticks on core 7; the timing phase also includes BeatThis workers |
| BeatThis | 1 caller + 3 workers, all participating | Caller on core 7 for 98.1–98.3% of kernel-event observations; workers predominantly 4–6, with occasional migrations to other cores |
| rows_encoder | 1 caller, no ORT workers | Same runtime thread; rows-phase samples on core 7 |
| rows_step | 1 caller, no ORT workers | Same runtime thread; rows-phase samples on core 7 |
| r2_predict | 1 caller, no ORT workers | Same runtime thread; arrangement-phase samples on core 7 |
| r2_release | 1 caller, no ORT workers | Same runtime thread; arrangement-phase samples on core 7 |
| r2_commit | 1 caller, no ORT workers | Same runtime thread; arrangement-phase samples on core 7 |
| Export / destruction | Runtime caller | Sampled runtime CPU ticks on core 7 |

All profiled kernels use CPUExecutionProvider. Worker counts come from positive changes in ORT's cumulative worker `num_run` counters, not from the number of threads merely created. Core observations are samples, not exhaustive residency; a 50 ms sample attributes its preceding tick delta to the last reported CPU. The sampler and GC are excluded from the work-placement statements.

BeatThis's profiled MatMul kernels take 3.82/5.36/8.44 s for short/medium/long, and Softmax 0.89/1.21/1.89 s. These are approximately 64% and 14–15% of its profiled stage time, respectively. New export/context work should inspect those operators first. R2 commit's operator work is led by Conv and Gemm. Operator profiling raises the long song's generate call from roughly 17.5 s to 20.5 s and peak RSS from roughly 1.92 GB to 2.60 GB, illustrating why those profiles cannot supply ordinary performance numbers.

## Adopted tier 0 app change: reuse the successful audio probe

Private branch `codex/android-perf-1-app`, commit `b0ab224`. Recognition passes the already retained mono PCM and hash to the chart maker only when the path and hash match the accepted audio. The ordinary URI path remains for recovery without retained PCM. The maker’s preparation, generation and export are unchanged. Sync continues using the same retained PCM; there is no new global cache.

Both arms use the original AAR and original decoder. Each arm first performs the same successful decode/hash probe, then invokes the actual app chart-maker path. Three adjacent pairs per song, AB/BA/AB. **All 9/9 complete chart SHA-256 comparisons equal their original-AAR app-path baseline; zero hit objects differ.** Thermal status is zero before and after every run. This is independent of the bulk-read comparison below; do not add their measured gains.

Time from chart request after the first probe and capability discovery:

| Song | Original s, mean (range) | Reused s, mean (range) | Seconds saved, mean (paired range) | Peak chart RSS MB, original → reused mean |
| --- | ---: | ---: | ---: | ---: |
| short | 21.339 (20.431–21.864) | 9.652 (9.546–9.809) | 11.687 (10.622–12.318) | 1856.0 → 1810.3 |
| medium | 30.203 (30.157–30.262) | 12.466 (12.442–12.489) | 17.736 (17.668–17.794) | 1878.5 → 1838.6 |
| long | 47.962 (43.105–50.492) | 19.079 (19.062–19.104) | 28.883 (24.033–31.388) | 1984.0 → 1913.6 |

Time including the first decode/hash probe and capability discovery:

| Song | Original s, mean (range) | Reused s, mean (range) | Seconds saved, mean (paired range) | Peak chart RSS MB, original → reused mean |
| --- | ---: | ---: | ---: | ---: |
| short | 32.688 (27.958–35.168) | 22.771 (22.494–23.078) | 9.917 (4.880–12.674) | 1856.0 → 1810.3 |
| medium | 48.047 (47.997–48.081) | 30.142 (29.733–30.543) | 17.905 (17.521–18.348) | 1878.5 → 1838.6 |
| long | 79.510 (74.576–82.014) | 50.742 (50.555–50.845) | 28.767 (23.732–31.384) | 1984.0 → 1913.6 |

The first probe remains necessary and variable. Reuse removes the second decode/hash, not that first pass. These are fresh processes and uncached charts; a previously stored chart has different behavior. Peak RSS above covers chart making only. The measured path omits recognition, microphone sync and playback, so these are not ambient-session join-time predictions. The combined app is checked separately below.

## Adopted tier 0 app setting: eight BeatThis threads on the K Pad

Private app commit `a88de2f` requests eight BeatThis threads only for model `25079RPDCC` (reported SoC `MT8799`, eight logical processors). Other devices keep the runtime’s default until measured. No global Android topology heuristic or affinity override is introduced. The five small graphs remain single-threaded.

Three interleaved pairs per song, AB/BA/AB, with the original decoder and the same aggregate-instrumented CPU AAR in both arms. Operator profiling is off for these pairs. Its four-thread control and all eight-thread charts match the original unmodified AAR: **9/9 full SHA-256 gates, zero changed hit objects**, thermal status zero throughout. Native interval includes opening, generation and closing; total also includes decode/hash. Decode varies independently, so the native and total gains are both reported.

| Song; pairs | Whole total A → B s (means) | Total seconds saved, mean (paired range) | Native interval A → B s (means) | Native seconds saved, mean (paired range) | BeatThis seconds saved, mean (paired range) | Peak RSS A → B MB (means); MB saved range |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| short; 3 | 21.084 → 19.394 | 1.690 (0.925–2.575) | 8.785 → 7.598 | 1.187 (1.144–1.227) | 1.183 (1.174–1.199) | 1831.3 → 1821.1; -7.9–19.6 |
| medium; 3 | 29.399 → 27.499 | 1.900 (1.620–2.073) | 11.744 → 10.116 | 1.627 (1.583–1.675) | 1.657 (1.651–1.663) | 1846.2 → 1841.9; -1.3–14.7 |
| long; 3 | 48.151 → 46.808 | 1.343 (0.428–2.126) | 18.375 → 15.865 | 2.510 (2.477–2.546) | 2.506 (2.462–2.528) | 1923.6 → 1924.6; -4.6–2.0 |

A separate short-song ORT profile confirms one caller plus seven participating BeatThis workers. Both caller and workers visit all cores 0–7; unlike the four-thread profile, the caller is not concentrated on core 7. The profile’s chart still matches, and its time/memory are excluded from paired means.

All nine total-time differences are positive, but their spread is wider than the native interval’s. RSS changes are small and inconsistent; no memory saving is attributed to the thread count. This measurement does not establish battery cost, sustained-load behavior or coexistence with ambient music. The app setting is deliberately scoped to the measured model. Checks build, 417 unit tests and lint pass.

## Adopted tier 0 app change: bulk PCM reads

Private branch `codex/android-perf-1-app`, commit `d51b19a`. Bulk reads for float32 and signed 16-bit codec output replace individual direct-buffer sample reads. Small reusable input arrays preserve the original channel summation/division order, trimming and consumed-buffer position. No decoder selection, timestamp policy or AAR changes.

Three adjacent pairs per song in AB/BA/AB order; the same original AAR is used in both arms. Every decoded PCM SHA-256 and complete chart SHA-256 matches (9/9 pairs, also against the initial original-AAR charts): zero changed hit objects. Thermal status 0 before/after every run. The medium-song time estimate is noisy; one pair was slower. These results describe a fresh checks process, not a warmed production session.

| Song | Original total s, mean (range) | Bulk-read total s, mean (range) | Seconds saved, mean (paired range) | Peak RSS MB, original → bulk mean |
| --- | ---: | ---: | ---: | ---: |
| short | 20.940 (20.852–20.992) | 20.560 (20.422–20.647) | 0.379 (0.241–0.552) | 1845.7 → 1821.7 |
| medium | 28.458 (27.707–28.936) | 27.910 (26.237–28.964) | 0.548 (-1.257–2.699) | 1848.2 → 1844.2 |
| long | 48.697 (48.466–49.146) | 47.885 (47.865–47.917) | 0.812 (0.562–1.274) | 1926.0 → 1924.5 |

Observed RSS differences are reported, not attributed to the tiny scratch arrays. Checks build, app unit tests and lint passed; the new unit check covers a direct buffer with nonzero offset, trimmed frames, trailing bytes and order-sensitive float summation.


### Decode attribution (separate diagnostic runs)

Original decoder and original AAR, one instrumented observation per song. These diagnostic counters are excluded from baseline and A/B means. All PCM and chart hashes match. All three files use `c2.android.mp3.decoder`, stereo PCM16 output (`encoding=2`); the app converts that to float32 mono. The adopted float-buffer branch is unit-tested; these MP3 device runs exercise the signed-16-bit branch.

| Song | Decode total s | Input feeding s | Output wait s | PCM conversion s | Output release s | Final array build s | Output buffers |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| short | 11.587 | 7.234 | 1.800 | 0.608 | 1.793 | 0.013 | 5371 |
| medium | 17.565 | 11.168 | 2.590 | 0.882 | 2.725 | 0.031 | 7256 |
| long | 30.678 | 19.569 | 4.607 | 1.539 | 4.722 | 0.040 | 13111 |

Input feeding includes acquiring the input buffer, extractor read/timestamp/advance, and queueing it; it is not all decode arithmetic or disk IO. The profile does not split those calls further. Codec/extractor interaction dominates; PCM conversion is only about 5%. This limits the bulk-read gain. A future same-codec asynchronous/batched API trial could still qualify for tier 0, but needs the same PCM/chart gate and format/lead/cancellation checks. The input-slot batching trial above already failed on time.

An alternative MP3 decoder belongs in tier 1 if its samples differ. Its absolute savings ceiling is the measured decode time (12–27 s in the initial means), not a predicted gain; time, hit-object differences and peak memory are unmeasured. It must match the explicit MP3 lead/gapless contract, run the complete chart comparisons on these songs and a broader owner-approved panel, then undergo the owner's quality review. Changing decoder does not establish quality merely by producing similar PCM.

The direct and app-path harnesses use different export metadata, so each has its own original-AAR chart SHA baseline. The app-path baseline hashes are `129197eb59dd7de739866bc83969b60a64082a35c2a667cda53f073d98e76e4f`, `bbdae2e07dd4045a67b4f37ebf9fa6fcbf06d7be161930bc1bb07b4e7f9d1e4d`, and `a6f32464125fa055afc9df015a6f3dea37b81f2855aff940750702126c57428e` for short/medium/long. Full byte comparisons are always within the same harness and request; the differing metadata is not excused as numerical noise.

## Rejected tier 0 candidate: feed available codec input slots

Three adjacent A/B pairs, one per song, original AAR in both arms. Every pair has identical decoded float32 PCM SHA-256 and chart SHA-256, including equality with the original baseline chart. All were slower, so screening stopped after those three pairs; there is no repeated same-song gain estimate and this code is not adopted.

| Song | Original decode s | Candidate decode s | Original total s | Candidate total s | Total seconds saved |
| --- | ---: | ---: | ---: | ---: | ---: |
| short | 11.591 | 14.485 | 20.384 | 23.252 | -2.868 |
| medium | 9.390 | 20.207 | 21.173 | 31.978 | -10.805 |
| long | 30.352 | 36.402 | 48.908 | 54.888 | -5.980 |

The initial direct baseline hashes through a 1 MiB FileInputStream buffer. Later checks use the same ContentResolver URI stream and 64 KiB buffer as AudioProbe/RuntimeChartMaker, and record `hashBufferBytes=65536`; separate results below identify that revision. Hash identity is unaffected. The actual app-path reuse comparison executes the maker's original hash path in its baseline arm.


## Adopted tier 0 runtime change: disable Android memory patterns

Normal Android sessions keep the CPU arena but disable ORT memory patterns. The existing `lowMemory` path still disables both. Other platforms retain their previous settings. No graph, arithmetic, thread count, compiler flag or FFI schema changes.

Three interleaved pairs per song (AB/BA/AB), original decoder and four BeatThis threads. Both arms use the same aggregate-instrumented CPU AAR, with operator profiling off: A uses the original memory settings and B disables only patterns. **All 9/9 full chart SHA-256 gates pass against the unmodified original AAR; zero hit objects differ.** Thermal status is zero before and after every run. The separate clean deliverable and original AAR contain identical bindings, helper classes, assets and notices; only the native runtime library differs.

| Song; pairs | Whole total A → B s (means) | Total seconds saved, mean (paired range) | Native interval A → B s (means) | Native seconds saved, mean (paired range) | BeatThis seconds saved, mean (paired range) | Peak RSS A → B MB (means); MB saved range |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| short; 3 | 20.689 → 20.897 | -0.208 (-0.782–0.091) | 8.777 → 8.686 | 0.091 (0.066–0.131) | 0.052 (0.038–0.064) | 1831.3 → 1178.5; 648.5–655.1 |
| medium; 3 | 29.319 → 29.105 | 0.214 (-0.157–0.768) | 11.705 → 11.631 | 0.074 (0.038–0.118) | 0.049 (0.040–0.067) | 1841.6 → 1197.9; 643.3–644.0 |
| long; 3 | 49.340 → 49.378 | -0.038 (-0.333–0.208) | 18.301 → 18.266 | 0.035 (0.005–0.066) | 0.015 (0.010–0.018) | 1925.9 → 1269.2; 656.2–657.3 |

Mean peak RSS falls by 652.8/643.7/656.6 MB (35.6%/35.0%/34.1%) for short/medium/long. This is principally a memory improvement. Native time improves by 0.035–0.091 s; whole-process differences are small and mixed because decode varies. No whole-process speed gain is claimed for this change. Keeping the arena avoids the large lowMemory slowdown in the screening table; changing the lowest-peak policy itself was not adopted.

The final integrated app and lowMemory checks below use the same clean AAR. Operator profiles and environment-variable experiment hooks are absent from the deliverable. The tested songs reach 315 s; this does not establish a universal peak-memory bound for arbitrary song lengths or other Android devices.

## CPU and build-option screening

One adjacent pair per candidate on the medium song, original decoder. These screen candidates for repetition; they are not three-pair gain estimates. Every candidate and control chart matches the original AAR. All thermal readings are zero. Native interval is open + generate + close, excluding the variable Android decode. `auto`’s apparent 7.534 s total saving came from decoding, not ORT.

| Candidate, medium song; one screening pair | Total s, A → B | Native interval s, A → B | BeatThis s, A → B | Peak RSS MB, A → B | Chart SHA equals original |
| --- | ---: | ---: | ---: | ---: | --- |
| one | 28.452 → 35.357 | 11.661 → 17.830 | 8.122 → 14.304 | 1846.5 → 1844.0 | True |
| two | 28.916 → 35.087 | 11.724 → 17.776 | 8.135 → 14.254 | 1842.9 → 1841.9 | True |
| auto | 28.977 → 21.443 | 11.697 → 11.720 | 8.126 → 8.125 | 1840.5 → 1855.8 | True |
| eight | 28.992 → 27.424 | 11.699 → 10.057 | 8.125 → 6.462 | 1841.9 → 1840.8 | True |
| nospin | 29.040 → 29.732 | 11.643 → 12.067 | 8.113 → 8.458 | 1840.3 → 1840.7 | True |
| affinity | 29.058 → 29.207 | 11.655 → 11.749 | 8.121 → 8.136 | 1840.3 → 1841.2 | True |
| lean | 29.337 → 35.608 | 11.680 → 19.887 | 8.131 → 16.395 | 1840.6 → 1060.3 | True |
| noarena | 29.389 → 31.143 | 11.726 → 13.552 | 8.115 → 10.033 | 1856.8 → 1100.6 | True |
| nopattern | 28.726 → 28.543 | 11.743 → 11.603 | 8.134 → 8.089 | 1842.6 → 1201.5 | True |
| small2 | 28.857 → 29.542 | 11.685 → 11.852 | 8.145 → 8.196 | 1840.8 → 1845.1 | True |
| lto | 28.869 → 28.814 | 11.690 → 11.698 | 8.104 → 8.127 | 1844.3 → 1843.9 | True |

`one`/`two`/`eight` are requested BeatThis thread counts; `auto` is zero. `nospin` disables intra-op spinning. `affinity` supplies `5;6;7`, but Android does not apply it (see below). `lean` is existing lowMemory. `noarena` disables only the arena; `nopattern` only patterns. `small2` gives all five non-BeatThis graphs two threads. `lto` changes only Rust to thin LTO and one codegen unit, with no CPU-feature or fast-math changes.

One/two BeatThis threads, no spinning, and two threads for the small graphs are rejected on this screen. Automatic threading and the affinity no-op provide no native gain. Thin LTO has +0.008 s native time in this pair; its 0.055 s total difference is not evidence of an improvement. Its stripped native library shrinks from 20.327 MB to 20.039 MB, but no LTO change is adopted. Explicit eight threads and normal-mode patterns-off pass the repeated comparisons above.

### Memory controls: screening, not an adoption gate

One medium-song observation per configuration, four BeatThis threads. These are an interaction study, not three repeated pairs and not a universal memory bound. All four chart hashes equal the original baseline.

| CPU arena | Memory patterns | BeatThis s | Peak RSS MB | Interpretation |
| --- | --- | ---: | ---: | --- |
| on | on | 8.134 | 1842.6 | Original normal mode |
| on | off | 8.089 | 1201.5 | Lower-peak normal mode; repeated gate above passes |
| off | on | 10.033 | 1100.6 | Faster than current lowMemory, but +40.3 MB in this sample |
| off | off | 16.395 | 1060.3 | Original lowMemory mode; lowest sampled peak |

The controls interact. Removing the arena is especially expensive when memory patterns are also disabled. Restoring patterns mitigates that cost but raises the sampled lowMemory peak; it is not adopted from this one song. The repeated candidate instead disables patterns in normal mode while keeping the arena. The existing lowMemory behavior is retained.

ORT’s execution frame traces allocations when it has no cached pattern, then requests a large contiguous pattern buffer on later runs. Its code explicitly notes that the arena may already hold smaller blocks from the earlier run. This provides a source-based explanation for the normal mode’s extra retained memory; the experiment did not collect an allocator-by-allocator heap ledger. With the arena disabled, per-run allocations are released instead of being retained by that arena. See [execution-frame allocation](https://github.com/microsoft/onnxruntime/blob/v1.23.2/onnxruntime/core/framework/execution_frame.cc) and [CPU arena implementation](https://github.com/microsoft/onnxruntime/blob/v1.23.2/onnxruntime/core/framework/bfc_arena.cc).

### Android-specific ORT threading findings

Pinned ORT 1.23.2 excludes Android from its cpuinfo-based physical-core discovery in `core/platform/posix/env.cc`. Its fallback is `hardware_concurrency / 2`, so automatic intra-op threads (`0`) resolve to four on this eight-core tablet, not eight. This is a source-based explanation for the identical BeatThis timing; explicit eight threads are tested separately.

The same file compiles `pthread_setaffinity_np` out on Android. Supplying `session.intra_op_thread_affinities` can parse and pass validation but does **not** pin Android workers in this build. The affinity-option screening pair is therefore a no-op control, not a successful affinity experiment. An actual Android affinity experiment would need a custom thread callback or explicit per-thread scheduling calls in platform glue, followed by observed masks and the hash gate. The baseline already runs mostly on cores 4–7, so no such invasive change is adopted. [Pinned ORT POSIX implementation](https://github.com/microsoft/onnxruntime/blob/v1.23.2/onnxruntime/core/platform/posix/env.cc).

## Implementation audit

- Recognition's `AudioProbe.probe` decodes mono PCM and hashes the file. `RecognitionController` retains that PCM for sync. `SessionCharts` originally gave only a URI to `RuntimeChartMaker`, which hashes and decodes again. The private candidate passes the retained decoding and matching hash directly to the maker; process-recovery/missing-PCM paths still decode normally. The same retained PCM remains available for sync. No global PCM cache is added.
- The current Kotlin helper already bulk-converts `FloatArray` to little-endian bytes; it does not construct `List<Float>`. The historical boxed-PCM issue is not present. UniFFI still copies bytes to Rust, and `pcm_f32le` allocates a float vector. Removing this safely would require an ownership/alignment design; the incoming byte allocation cannot simply be cast to a float vector with another allocation layout.
- Both Mels use the core's mixed-radix float64 FFT (1024 and 960 points), precomputed twiddles and sparse nonzero Mel weights. The soxr table resampler already uses four accumulators. Replacing the FFT, changing reduction order, enabling FMA/fast-math, or changing resampling is not assumed byte-preserving. `libm` is deliberate for cross-platform numerics.
- Rust's release build has no repository profile override: ordinary release optimization, 16 codegen units, no cross-crate LTO, generic `aarch64-linux-android`. Android's ORT build is Release `-O3`, CPU EP with KleidiAI. The thin-LTO/one-codegen-unit screen gives no native speed gain and is not adopted; no CPU-feature or fast-math flag is added. ORT dispatches its own CPU kernels independently of Rust target features.
- `lowMemory` disables the CPU arena and memory patterns in every graph. Allocation/release work can recur at every BeatThis chunk and every rows/R2 call. The measured two-by-two interaction above separates those controls. The lowest-memory contract remains unchanged.

## Tiers 1 and 2: candidates, not adopted

All timings and object differences below are **unmeasured unless explicitly supplied later**. An unmeasured difference is not zero. Stage-time ceilings are bounds on potential savings, not forecasts; acceleration also adds setup, transfers and fallbacks. Memory estimates based on weights exclude activations and allocator retention.

| Candidate | What can improve / time bound | Memory / effort | Required check and limitation |
| --- | --- | --- | --- |
| Tier 1: XNNPACK | At most the time in supported graph operators; use measured graph times as the ceiling | Another ORT source build, provider registration and per-graph selection; medium effort, peak unknown | Disabled in the AAR and absent from the binding's provider enum. The README documents the disabled providers and the broken GNU-libstdc++ prebuilt archive, but does **not** give a separate measured reason for disabling XNNPACK. Its supported 2D Conv/Gemm/MatMul subset leaves attention, rotary and GRU work on CPU. Measure partition coverage, fallback and both thread pools. [Provider documentation](https://onnxruntime.ai/docs/execution-providers/Xnnpack-ExecutionProvider.html). |
| Tier 1: NNAPI / MediaTek NPU | All six graph calls total 6.53/9.12/14.60 s (short/medium/long): an absolute savings ceiling, not a forecast; offload adds setup/transfers | NNAPI is not built or exposed; building/registering its public EP is medium/high effort. A direct vendor EP separately needs an SDK/plugin. Peak unknown | Enumerate devices and actual partition assignment on this tablet; disable NNAPI's CPU fallback for diagnostic runs. NNAPI was deprecated in Android 15, and the presence of an NPU proves no compatible driver or ORT backend. Qualcomm QNN is not a MediaTek backend. [Android status](https://developer.android.com/ndk/guides/neuralnetworks), [ORT NNAPI options](https://onnxruntime.ai/docs/execution-providers/NNAPI-ExecutionProvider.html). |
| Tier 1: GPU | BeatThis is the first graph to try: 5.88/8.17/12.82 s original stage ceiling; small graphs have little work per host round trip | New supported Android EP/build and buffers, high effort, peak unknown | CPU and Apple CoreML are the only exposed providers. CUDA/CoreML do not target this Android GPU. Measure partitioning, transfers and cold setup separately; preserve ONNX/backend boundary. |
| Tier 1: BeatThis INT8 / FP16 | 5.88/8.17/12.82 s original graph-time ceiling; INT8 probe below | 82.10 MB graph on disk; lower-precision weights could reduce its weight storage, not necessarily its large activations. Medium/high effort | Static quantization needs authorized calibration audio, per-op coverage and a converted local package. Keep graph I/O float32 because the runtime tensor adapter supports only f32/i64/bool. The pinned Android build also has `onnxruntime_ENABLE_CPU_FP16_OPS=OFF`; CPU FP16 is not a drop-in model-file swap. Test chunks, beats, fitted grids and complete hit objects; owner quality review follows. |
| Tier 1: rows encoder INT8 / FP16 | 0.045/0.060/0.104 s graph-time ceiling; INT8 probe below | 2.28 MB graph, whole-song dynamic attention may dominate activations; medium effort | Dynamic beat/frame axes, local-attention masks, cumulative sums and normalization require fallback/cast inspection. Any changed head time propagates to R2. |
| Tier 1: rows step INT8 / FP16 | 0.045/0.047/0.070 s graph-time ceiling; INT8 probe below | 12.41 MB graph including GRU, medium effort | Carry hidden state and copy history through the full song; compare sampled figures and full charts. Small teacher-forced numerical differences do not bound autoregressive differences. |
| Tier 1: R2 predict INT8 / FP16 | 0.051/0.071/0.131 s graph-time ceiling; INT8 probe below | 0.68 MB graph, small savings ceiling, medium effort | Compare legal-support decisions and sampled actions; all 625 logits matter near ties. |
| Tier 1: R2 release INT8 / FP16 | 0.011/0.023/0.047 s graph-time ceiling; INT8 probe below | 0.50 MB graph, dynamic candidate count, medium effort | Each oriented release depends on previous choices. Compare hold end times and full objects, not just action heads. |
| Tier 1: R2 commit INT8 / FP16 | 0.525/0.747/1.381 s graph-time ceiling; INT8 probe below | 8.02 MB graph, medium effort | Full carried temporal buffers must be exercised. Repeated small state differences can compound. |
| Tier 2: smaller/shorter-context BeatThis | Targets its measured dominant graph time | Smaller network/activations; high effort including new package and likely distillation/retraining | Current exporter produces dynamic-time float32 ONNX from a CPU model copy with rotary caching disabled. Runtime uses 1500-frame chunks, 6-frame borders, reverse overwrite aggregation. Changing context or export is a new model/package experiment, with grid and owner play review. |
| Tier 2: rows encoder local attention export | Targets whole-song activation peak; graph-time ceiling only 0.045/0.060/0.104 s | Potentially lower memory; new export and parity investigation, medium/high effort | Exporter's `_LocalAttention` computes a full score matrix and masks to the local window. Banded/sliced attention could avoid the dense matrix, but a new export is tier 2 here. Keep boundaries, validity and softmax support exact; no adoption in this round. |
| Tier 2: narrower rows / R2 networks | Targets encoder, step and arrangement graph time | Smaller weights/state; checkpoint work, export and calibration, high effort | Export supports released FiLM R2, optional landmarks, explicit GRU/TCN state. Width/depth changes need compatible schemas/state files and owner review of density, patterns and holds. No training or new export performed. |
| Tier 2: combine graph calls | Targets per-call overhead, bounded by the non-kernel part of graph time | Medium/high effort, memory uncertain | Cannot simply fuse predict→release→commit: the sampled action and sequential releases determine the committed history token. Rows are autoregressive too. Batching songs does not reduce one-song latency. |

Export facts above were read from ensomi-model `8f30b925f9e77d4b8e34fd4ca1436f73e3afe019`, [`src/ensomi_model/export`](https://github.com/ensomi-labs/ensomi-model/tree/8f30b925f9e77d4b8e34fd4ca1436f73e3afe019/src/ensomi_model/export) and its [ONNX guide](https://github.com/ensomi-labs/ensomi-model/blob/8f30b925f9e77d4b8e34fd4ca1436f73e3afe019/docs/guides/onnx_runtime.md). Existing graph sizes/node inventories were read from the released package only. No files in ensomi-model were changed. No sealed data, N1, model uploads or training were used.

### MediaTek integration boundary

The Dimensity developer guide distinguishes Neuron Adapter/NNAPI integration from the offline Neuron Runtime API, which it restricts to vendor-partition applications; it is not a normal APK drop-in. See [MediaTek’s deployment table](https://developer.mediatek.com/ai/6423c916c612745b3a4baa2d.html). The newer **Genio** Android documentation says its accelerated ORT libraries/EPs must be obtained from a MediaTek representative; it does not establish compatibility with this Dimensity tablet. See [MediaTek’s Android ORT access instructions](https://genio.mediatek.com/doc/iot-aihub/ai_hub/supported_os/android.html). Before a direct MediaTek execution-provider project, the owner would need a redistributable, Dimensity-9400+-compatible provider/SDK and confirmation that an ordinary APK can use it. No proprietary SDK access or NPU inference is claimed here.

### Cheap tier 1 probe: dynamic INT8 MatMul/Gemm conversion

Local copies only, ORT 1.23.2 `quantize_dynamic`, per-channel signed INT8 weights for constant-weight MatMul/Gemm. Their activations are dynamically quantized; this is not a storage-only conversion. Variable-by-variable attention MatMul, Conv and GRU stay float32, as do graph I/O. All six source graph hashes match the pinned package manifest and are checked again on the tablet. ONNX checker and Mac CPU session-load check passed for all six. This is neither an Android speed measurement nor a quality result. No inference on test songs was performed by this conversion check: changed hit-object counts and peak inference memory remain **unmeasured**. Files are ignored under `quantized/`.

| Graph | Original MB | Converted MB | MatMulInteger nodes |
| --- | ---: | ---: | ---: |
| beatthis | 82.099 | 22.885 | 62 |
| rows_encoder | 2.279 | 0.845 | 14 |
| rows_step | 12.413 | 11.052 | 9 |
| r2_predict | 0.683 | 0.215 | 9 |
| r2_release | 0.500 | 0.153 | 8 |
| r2_commit | 8.018 | 4.518 | 25 |

XNNPACK build probe: enabling it in an isolated copy of pinned ORT initially failed CMake generation: the `onnxruntimeTargets` install export referenced Abseil targets outside an export set. `ABSL_ENABLE_INSTALL=ON` did not resolve it: ORT explicitly turns that off for XNNPACK to avoid a duplicate target. `CMAKE_SKIP_INSTALL_RULES=ON` allowed the static build (no system installation is needed). The excluded `XNNPACK` and `pthreadpool` targets also had to be built explicitly before Rust could link them. This is build evidence, not yet evidence that XNNPACK runs the graphs.

### Tier 2 estimate from attention shapes

In the three separate BeatThis profiles, kernels whose input or output includes a `1500 × 1500` temporal-attention matrix account for 2.47/3.44/5.41 s, about 42% of total kernel time. Under an idealized quadratic-cost model, halving context to 750 frames and doubling the chunk count would save roughly 1.24/1.72/2.70 s of this work, before extra chunk/border/setup costs and changed kernel efficiency. This is a design estimate, not a benchmark or a quality result. One `[32,1,1500,1500]` float32 score tensor is 288 MB; its 750-frame counterpart is 72 MB. Total peak memory is not predicted by that tensor alone. No export, context setting, model file or training configuration was changed.
