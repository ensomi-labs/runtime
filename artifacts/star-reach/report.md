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

## Results

Pending. Private per-song data, charts and prepared audio stay under
`/Users/l/ensomi/ensomi-model/artifacts/star-reach-20261110/`.
Only aggregates, drivers and this report enter the public repository.
