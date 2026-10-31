2026-11-01 01:42 JST — Started; runtime branch clean, required documentation read.
2026-11-01 01:42 JST — Baseline AAR copied and SHA-256 verified; private app worktree created.
2026-11-01 01:42 JST — Existing trace phases extracted as numbers only; initial priorities recorded before edits.
2026-11-01T01:45:03 — Baseline checks APK and instrumentation built; certificate verified; waiting for kpad-netease-hint lock; native profiling harness built.
2026-11-01T01:48:40 — cargo test --workspace passed; native and streamed conformance replay passed; private app unit tests/lint passed; no optimization adopted before device gate.
2026-11-01T01:53:55 — Native/Wasm whole-chart and streamed conformance all passed; six local INT8 MatMul/Gemm conversions checked and CPU-loaded; device lock still held by other job.
2026-11-01T02:08:49 — First device instrumentation failed before generation (Kotlin return type); corrected methods verified void. XNNPACK isolated build linked after install-rule/static-target fixes; runtime source restored.
2026-11-01 02:21 JST — Baseline collection underway, thermal 0 and original AAR; decode dominates (~60%), prepared input-buffer scheduling candidate in private worktree. No candidate adopted.
2026-11-01T02:23:59 — Original-AAR baseline 3 songs × 3 runs complete; hashes stable, thermal 0; full baseline table written before candidate runs.
2026-11-01T02:30:19 — Six-graph timing split complete, 3 runs/song; all nine instrumented charts equal original AAR. Operator/thread profiles next.
2026-11-01T02:35:24 — All three operator/core profiles collected; BeatThis uses 4 threads, other graphs 1; compressed evidence transfers after early lease release. Decoder A/B running.
2026-11-01T02:40:42 — Codec-input batching rejected: 3/3 pairs slower, PCM/chart hashes equal; code reverted. Tablet released to android-ship-1. Bulk PCM, reuse and ORT candidates ready/queued.
2026-11-01T03:00:41 — Adopted bulk PCM reads in private commit d51b19a: 0.379/0.548/0.812 s mean saved, 3 pairs/song; PCM and chart SHA gates 9/9, medium timing noisy. Decode diagnostics and original-path reuse A/B now running.
2026-11-01T03:18:53 — Probe reuse passes all 9 chart gates; mean chart-stage savings 11.687/17.736/28.883 s, 3 pairs/song. Combined app builds, 417 unit tests and lint pass; CPU option screening started.
