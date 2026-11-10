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
