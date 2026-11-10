# R2 star reach and target-search experiment — 2026-11-10

## Decision rules (preregistered; do not rewrite after observing arms)

Started 11:24 UTC; stop about 13:54 UTC (2.5 hours). Baseline source is
`edf4066`; the first, isolated commit `4c94563` accepts finite whole-song and
section stars and clamps the prior band to 2–5. No other defaults change.
This is a study of **measured star and structural warning signs**, not
playability: nobody will play these charts. All conclusions carry that limit.

**Measurement.** Use `compute_mania_star_rating_20241007(objects, 4, 1.0)`
from ensomi-model tag `r2-1.0.1` (`885bb74c19bd68d1c77087d3ec8746244196daa5`),
loaded directly from the pinned file. Before arms, check the four published
examples against the reported rounded ratings (4.63, 3.57, 2.73, 3.19), and
check the structural metrics on hand-constructed charts. Runtime `r` is
reported separately, never used as a proxy for star.

**Sampling fixed before generation.** Four published songs: Adventure,
Catch the Moment, Hesitation Snow, Saigetsu. Eight distinct held-out audio
recordings from `data/r2-corpus.parquet`: `eval_split == heldout`, audio
present on disk, positive finite dominant BPM and note density, annotated
span 45–300 seconds. Deduplicate by audio SHA-256 (lexicographically first
chart path). Use percentile ranks of dominant BPM, annotated duration and
(notes / annotated second), start at the lowest BPM, then greedily maximize
minimum Euclidean distance to selected points, with path as tie breaker.
This spreads metadata, not generated outcomes; it is not a population-random
sample. Freeze the private manifest before any generation. Use seeds 0 and
1000, paired in every arm. No song exclusions based on generated output.

**Q1.** Requests 1–9 by 0.5 plus 1.875, 2.0, 2.0625, 2.125, 2.1875,
5.75, 5.8125, 5.875, 5.9375, 6.0, 6.125. Report median, p10–p90 and
min–max across song/seed charts, song medians, within-song absolute
seed-pair difference, in-table OLS slope and signed bias. The null spread
at an end is the median across songs of `abs(star(seed0)-star(seed1000))`
at request 2.0 or 5.875. Also report p90 of those differences. Plateaus
require equal hit objects for every paired song and seed, not just equal
rounded ratings. `.osu` metadata is not a difficulty measurement.

**Q2 arms.** Keep baseline masks unless the other levers fail the proposal
rule. Test rows linear extrapolation with the overall slope and each end
segment's slope; initially restrict log workload to [0, 6] as a numerical
exploration bound, not a supported domain. Test quantiles (low/high pairs)
0.1/0.75 and 0/0.95; the latter is outside the receipt's tested [0.1, 0.75].
Test multiplier ceiling 1.3 (within the fit's [0.8,1.3]) alone, and direct
multipliers 0.8/1.3 to separate a clipped quantile from the head-count lever.
The default lower bound is already 0.8. At minimum test 1, 1.5, 2,
5.875, 6.5, 7, 8, 9; add the Q1 request grid for any proposed curve.
Quantile and multiplier changes ramp linearly from their baseline value at
2.125/5.875 to their arm's endpoint at 1/9, then clamp. They therefore leave
the fitted interval unchanged. Combine the rows arm with the largest
correct-direction endpoint median movement with the best of the non-rows
arms, if each moves the endpoint by at least one null spread. If no such
combination, report that. Ties choose overall slope, then tested quantile,
then fit ceiling, then direct multiplier, then untested quantile.

If these stall, one final, explicitly unsupported mask arm may ramp the
reattack floor to 200 ms at 1 and 50 ms at 9, with high-end quad-run cap 2;
compare it alone and with the best rows arm. These are invented limits,
not fitted evidence of playability. No other levers without a dated,
pre-run amendment explaining them.

**Structural envelope fixed from baseline fitted edges.** For each song,
reference all four charts (2 requests × 2 seeds) at 2.0 and 5.875. Every
candidate at a proposed end must satisfy all following per-chart bounds;
no averaging hides a failed song. These are conservative screening rules,
not a validated playability classifier. Numerical values will be filled
from baseline before evaluating arms; formulas will not change.

| Indicator | Allowed envelope, using that song's edge minimum L and maximum H |
| --- | --- |
| Generation/export | succeeds, legal chart, at least 2 head rows, all 4 lanes used |
| Fallbacks per 1,000 head rows | <= max(5, 2H) |
| Beats without a head row | <= min(1, H + 0.15) |
| Notes per whole audio second | [0.5L, 1.5H] |
| Head rows with 3 notes; with 4 notes (separately) | <= min(1, H + 0.10) |
| Long-note heads / notes | [max(0, L - 0.10), min(1, H + 0.10)] |
| Shortest same-lane head-to-head reattack | >= max(50 ms, 0.8L) |
| Largest lane's share of notes | <= max(0.45, H + 0.10) |
| Longest head-free gap, including song ends, in grid beats | <= max(8, 2H) |
| Fit-domain and fallback support | multiplier within [0.8,1.3]; no selected fit `fallback_ok: false` |

Report clamp, above/below-tested-q and fit fallback flags separately;
clamping and untested q alone do not prove degeneration. A receipt's
`per_m` entries describe calibration points, not observations on this chart.
Inspect the selected multiplier's support rather than rejecting all charts
because a distant calibration entry failed.

**Proposal gate, separately per end.** Across the full tested extended
range, the median of the 12 song medians (each over two seeds) must be
non-decreasing with request (allow only 1e-9 arithmetic noise). At the end,
the paired median movement from the baseline plateau must exceed twice
the baseline null spread and be at least 0.5 stars. Every candidate must
stay in the structural envelope. Failure of any clause means no proposed
default. Report signed effects, failures and raw reach even when rejected.
Choose a cap as the furthest contiguous request from the fitted edge
satisfying monotonicity and structural bounds, before a reversal or failure;
never hide the uncapped observations. A capped curve must itself meet the
0.5-star and twice-spread gate. No extrapolation beyond the tested cap.
A surviving proposal gets a separate last commit enabling it only outside
[2.125,5.875], with byte parity inside that interval. If none survives,
leave all experimental options off by default.

**Q3.** Implement the requester's algorithm literally: N=6, tolerance=0.05,
saturation delta=0.01, slope clamped to [0.5,2], request bounded to T±3,
earliest winner on ties. Targets 1–9 by 0.5; seeds 0 and 1000. Compare
six seeds at fixed request and a proportional-step variant of the same
algorithm omitting secant slope. Identical candidates may be cached, but
count logical generations, not cache misses. All methods keep the best,
so returned error must never exceed first-try error. Report error median,
p90, shares <=0.05 and <=0.15, generations and saturation, per target and
pooled. Stratify interior and out-of-reach targets; pooled improvement
alone cannot justify steering. A different rule is a recommendation only
if its paired median error and p90 are no worse, at least one improves,
and mean logical budget is no greater; otherwise state the tradeoff.
No tuning on this sample presented as an independent validation.

## Method and provenance

The preregistration above was committed as `7f429ef`, before generating any
charts. A final proposed curve must also be non-decreasing on the unchanged
in-table baseline points; the per-end tables below are screening tests, not
a waiver of the full-curve gate. This clarification was written before any
lever arms ran.

The frozen manifest SHA-256 is
`49e43483dfeb4f5a011d0a1c97dbe51e5146d9b09ec725b0ca1fbe117ca28662`.
Selection found 467 eligible distinct held-out recordings and chose eight
with eight distinct corpus group IDs. Their source annotations span
58–270 BPM, 54.473–267.555 seconds and 1.445–26.047 notes/second. These are
selection metadata, not output measurements. The actual twelve full audio
recordings span 60.0–273.9 seconds. No cropping or post-generation selection.
No per-song table from the private corpus is included here.

The four published inputs use the recorded float64 PCM, channels averaged
in float64 then rounded to mono float32, at each recording's rate. Held-out
audio is decoded by ffmpeg to mono float32 at 44100 Hz. The native public
`prepare` API runs once per song; its `Prepared` bytes are reused by
`generate_prepared`. Every candidate uses one arrangement pass, default
natural long notes, no sections or pattern requests. CPU ONNX Runtime runs
the chart graphs on one intra-op thread; BeatThis uses the performance
cores during preparation. Device: Apple M5, 24 GiB RAM, macOS 27.0.
Concurrent jobs make these timings unsuitable as standalone latency claims.

The pinned calculator reproduces the published examples:
Adventure 4.6336314444, Catch the Moment 3.5713266497,
Hesitation Snow 2.7325226611, Saigetsu 3.1917039504.
These are consistency checks against known outputs, not human validation
of star as perceived difficulty. Hand-counted checks cover chords, LN
share, reattacks, empty beats, lane collapse, rounded beat boundaries and
a fractional multi-segment grid. The envelope has no human-labelled
validation: passing it only means no listed warning exceeded its bound.

Head rows mean distinct exported head timestamps; chord size counts heads,
not held lanes. LN share counts LN heads among all notes. Reattack means
head-to-head in one lane. NPS divides by the entire audio duration, including
silence. Empty beats use the rows model's whole-beat grid and include partial
edge beats; a 0.501 ms allowance assigns rounded heads to their intended
beat. Longest head-free gap includes both song ends. These definitions can
characterize a chart's structure but cannot judge its musicality.

Builds:

| Purpose | Source |
| --- | --- |
| Start | `edf4066` (`origin/main` at job start) |
| Only product behaviour change | `4c94563`: finite stars, band clamped to 2–5 |
| Baseline worker | `5ed87d8`, with the core exactly at `4c94563` |
| Opt-in lever worker | `d663b6b` |
| Measurement writer fix / aggregation | `4a84b0d` |
| Search plateau memoization | `fe64a94` |

The first baseline attempt exposed a Python result-writing collision between
requested and measured `star`; it stopped before producing any sweep table.
`4a84b0d` separates those fields, and the baseline sweep was restarted. No
arms or selection decisions used that failed attempt. `builds.json` in the
private directory records binary and runtime-config SHA-256 hashes.

Code reading confirms the supplied mechanism: clamped 16-point rows
workload table, band-only priors/masks/quantiles, fitted multiplier clipped
to [0.8,1.2], zero R2 conditions, and seeds `s` / `s+954` (low 32 bits).
The overall rows slope is 0.4999019906 log-workload/star; end slopes are
0.7929525934 below and 0.4829161723 above. The multiplier lower bound is
already 0.8; `direct` explicitly blends the chosen multiplier toward 0.8
or 1.3 to measure the head-count lever without a quantile/clamp confound.
For `direct`, the receipt’s `domain` and `clamped` describe the raw fit
solution before that explicit blend; `m_final` records the resulting
multiplier, checked against the fit’s full [0.8,1.3] domain.
`ceiling` only raises the allowed upper edge. The model's calibration
`per_m` says fallback support holds through 1.3 and fails from 1.45; this
experiment never requests a multiplier outside [0.8,1.3]. The receipt
contains `clamped` and `above_tested_band`, but does not carry `per_m`;
that support statement comes from the package, not each generated chart.

The requested cross-repository sweep found the old range in the model
README, released-system report/Python entry point, runtime validation and
capabilities metadata, and the Hub card. No applicable range contract was
found in the protocol or beatmap-lens. The separate `codex/star-open` job
owns the product's metadata/UI/documentation rollout. This experiment
changes no client pin, model file, release, tag, or published card.
The runtime and model sources are respectively
[the starting pipeline](https://github.com/ensomi-labs/runtime/blob/edf4066/crates/generate/src/generate.rs)
and [the pinned calculator](https://github.com/ensomi-labs/ensomi-model/blob/r2-1.0.1/src/ensomi_model/osu_core/difficulty.py).

## Reproduction

From the repository root. `select` needs numpy and pyarrow (available in the
model checkout's venv); all other Python drivers use the standard library.
`select` refuses to overwrite its frozen manifest. Use a fresh private
output directory for a new sample. Reuse this run's manifest/prepared files
to reproduce these exact songs. Do not put that directory inside this public
repository. Build the baseline at `5ed87d8` in a separate worktree to compare
with the saved experiment build; both binaries must use the same model.

```sh
MODEL="$HOME/ensomi/ensomi-model"
OUT="$MODEL/artifacts/star-reach-20261110"
cargo build --release -p ensomi-generate-ort --example star-reach
# Keep each compiled binary under target/ before changing source commits.
"$MODEL/.venv/bin/python" artifacts/star-reach/run.py select --model-repo "$MODEL" --private "$OUT"
python3 artifacts/star-reach/run.py validate --model-repo "$MODEL" --private "$OUT"
python3 artifacts/star-reach/run.py prepare --model-repo "$MODEL" --private "$OUT" --binary target/star-reach-baseline
python3 artifacts/star-reach/run.py sweep --model-repo "$MODEL" --private "$OUT" --binary target/star-reach-baseline --run baseline
python3 artifacts/star-reach/analyze.py --private "$OUT" --freeze-envelope
python3 artifacts/star-reach/run.py sweep --model-repo "$MODEL" --private "$OUT" --binary target/star-reach-arms --run single-arms --arms overall,end_segment,q_tested,q_outer,ceiling,direct
python3 artifacts/star-reach/run.py search --model-repo "$MODEL" --private "$OUT" --binary target/star-reach-baseline --run baseline --cache-baseline-plateaus
python3 artifacts/star-reach/parity.py --model-repo "$MODEL" --private "$OUT" --binary target/star-reach-arms
python3 artifacts/star-reach/analyze.py --private "$OUT"
python3 -m unittest discover -s artifacts/star-reach -p 'test_*.py'
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

`--assets`, `--onnx`, `--jobs`, `--songs`, `--arms` and `--full-grid` are
explicit driver arguments. The search's plateau cache is allowed only after
the complete Q1 sweep proves equal hit objects at each end. It canonicalizes
those requests to 2.125 or 5.875 while preserving the literal requests and
all six logical generations in search traces. It is a computation shortcut,
not a changed search rule. Controls are unchanged throughout.

## Additional search comparison (committed before its runs)

After Q1 established exact endpoint equality, add one model-aware search
rule, `bounded`: initialize `r = clamp(T, 2.125, 5.875)` and clamp every
secant update to that same interval instead of `T ± 3`. All other logic,
including N=6, tol=0.05, slope bounds [0.5,2], saturation delta 0.01,
seed increments and earliest ties, stays unchanged. Its first chart must
match the original algorithm's first chart. The bounds come from the model
table and Q1 equality, not tuning search errors. This tests the specific
hypothesis that avoiding known-equivalent requests saves a useful try.
The original three algorithms remain required comparisons. Judge this
follow-up by the same median-error, p90 and mean-generation gate. It uses
the same panel and therefore requires independent replication before a
population claim; it is not a new held-out validation set.

## Results

Baseline complete; lever and search results pending. Before the lever arms,
the numerical per-song envelopes were frozen in the private
`structural-envelope.json`, SHA-256
`98b7bdc4388eeda1bbb79e99efcadc6c07a13b5a2df5cb08a7a049373c19b0d0`.
The low/high null spreads are 0.290474 / 0.351173 stars, so the strict
movement thresholds are >0.580948 / >0.702346 stars, in addition to >=0.5.
The unchanged in-table baseline medians are monotone on the chosen grid.

Five of 24 high-edge reference charts violate the predeclared absolute
50 ms reattack floor (the minimum is 29 ms). This shows that the screen is
stricter than the current generator on rare reattacks, not that it is a
validated test of unplayability. The thresholds will not be relaxed after
seeing this result. Actual fallback and interval observations remain in the
tables, including baseline failures. All 624 baseline generations succeeded.

## Q1: baseline measurements

All star ratings below are measurements, not playability judgements. The centre is the median of 12 song medians; p10–p90 and extrema pool 24 charts. Seed spread is the median paired absolute difference between seeds 0 and 1000.

| Request | Song median | Chart p10–p90 | Chart min–max | Seed Δ median | Seed Δ p90 |
| --- | --- | --- | --- | --- | --- |
| 1.0000 | 2.477 | 2.178–3.055 | 2.021–3.439 | 0.290 | 0.600 |
| 1.5000 | 2.477 | 2.178–3.055 | 2.021–3.439 | 0.290 | 0.600 |
| 1.8750 | 2.477 | 2.178–3.055 | 2.021–3.439 | 0.290 | 0.600 |
| 2.0000 | 2.477 | 2.178–3.055 | 2.021–3.439 | 0.290 | 0.600 |
| 2.0625 | 2.477 | 2.178–3.055 | 2.021–3.439 | 0.290 | 0.600 |
| 2.1250 | 2.477 | 2.178–3.055 | 2.021–3.439 | 0.290 | 0.600 |
| 2.1875 | 2.489 | 2.231–3.072 | 2.021–3.399 | 0.211 | 0.631 |
| 2.5000 | 2.709 | 2.464–3.456 | 2.287–3.756 | 0.203 | 0.422 |
| 3.0000 | 3.237 | 2.894–3.770 | 2.410–4.194 | 0.266 | 0.839 |
| 3.5000 | 3.620 | 3.033–4.183 | 2.410–4.379 | 0.394 | 0.837 |
| 4.0000 | 3.790 | 2.942–4.613 | 2.730–5.065 | 0.350 | 0.866 |
| 4.5000 | 4.177 | 3.291–5.077 | 2.937–5.422 | 0.452 | 0.800 |
| 5.0000 | 4.345 | 3.980–5.124 | 3.601–5.609 | 0.411 | 0.632 |
| 5.5000 | 4.889 | 3.976–5.478 | 3.645–6.053 | 0.282 | 0.837 |
| 5.7500 | 5.120 | 4.152–5.733 | 3.645–6.115 | 0.405 | 0.683 |
| 5.8125 | 5.209 | 4.121–5.755 | 3.645–5.822 | 0.400 | 0.533 |
| 5.8750 | 5.266 | 4.121–5.809 | 3.645–6.649 | 0.351 | 0.704 |
| 5.9375 | 5.266 | 4.121–5.809 | 3.645–6.649 | 0.351 | 0.704 |
| 6.0000 | 5.266 | 4.121–5.809 | 3.645–6.649 | 0.351 | 0.704 |
| 6.1250 | 5.266 | 4.121–5.809 | 3.645–6.649 | 0.351 | 0.704 |
| 6.5000 | 5.266 | 4.121–5.809 | 3.645–6.649 | 0.351 | 0.704 |
| 7.0000 | 5.266 | 4.121–5.809 | 3.645–6.649 | 0.351 | 0.704 |
| 7.5000 | 5.266 | 4.121–5.809 | 3.645–6.649 | 0.351 | 0.704 |
| 8.0000 | 5.266 | 4.121–5.809 | 3.645–6.649 | 0.351 | 0.704 |
| 8.5000 | 5.266 | 4.121–5.809 | 3.645–6.649 | 0.351 | 0.704 |
| 9.0000 | 5.266 | 4.121–5.809 | 3.645–6.649 | 0.351 | 0.704 |

Hit-object plateau equality: {'low': True, 'high': True}. In-table OLS slope 0.657, intercept 1.235; mean measured-minus-request bias -0.188, median bias -0.050. These are descriptive across this selected panel, without a population confidence claim.

## Q2: arms and structural references

The baseline fitted-edge rows are included below. Each cell gives a median unless marked maximum or minimum. F is fallbacks/1,000 head rows (median/max); E is empty-beat share; 3/4 are the separate three-/four-head row shares; LN is long-note head share; A is the smallest same-lane reattack across all 24 charts in ms; L is maximum-lane share (median); G is longest head-free gap in beats (median). V counts charts violating at least one per-song envelope; C/Q are the shares with multiplier clamp / untested quantile. The JSON/CSV companions include p10, p90, minimum and maximum for every metric, plus workload r, workload request, multiplier, q, timing and failure counts.

### baseline

| Request | Star med [p10,p90] | F med/max | E | NPS | 3 / 4 | LN | A min | L | G | V | C / Q |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2.0000 | 2.477 [2.178,3.055] | 0.000/3.788 | 0.058 | 6.070 | 0.015/0.000 | 0.205 | 118.0 | 0.262 | 7.877 | 0 | 0.00/0.00 |
| 5.8750 | 5.266 [4.121,5.809] | 0.000/2.058 | 0.037 | 14.405 | 0.060/0.003 | 0.099 | 29.0 | 0.258 | 5.091 | 5 | 0.17/0.00 |

### Gate at the tested endpoints

| Arm | End | Median star | Paired movement | 2 × null | Monotone | Envelope failures | Pass |
| --- | --- | --- | --- | --- | --- | --- | --- |

### Contiguous cap check

| Arm | End | Last safe request | Median star | Paired movement | First failed request | Why stopped | Preliminary gate |
| --- | --- | --- | --- | --- | --- | --- | --- |

Unchanged in-table baseline medians are non-decreasing: True. A preliminary cap pass still needs the denser grid; this table cannot alone authorize a default.

Failures count repeated candidates at different requests; the private envelope file contains every song-specific bound. Passing this structural screen would still not establish playability. A full-grid/cap assessment is required before proposing a default.
