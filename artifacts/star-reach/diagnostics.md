## Search diagnostics

The following prefix budgets reuse the same traces, stopping after N tries. They are descriptive checks on this sample, not independently tuned or validated constants.

| Scope | Rule | N | Median error | p90 error | ≤.05 | ≤.15 | Mean used |
| --- | --- | --- | --- | --- | --- | --- | --- |
| all | secant | 3 | 0.766 | 3.198 | 0.071 | 0.208 | 2.91 |
| interior | secant | 3 | 0.177 | 0.635 | 0.161 | 0.464 | 2.79 |
| all | secant | 4 | 0.703 | 3.152 | 0.091 | 0.243 | 3.84 |
| interior | secant | 4 | 0.130 | 0.557 | 0.208 | 0.548 | 3.63 |
| all | secant | 5 | 0.668 | 3.141 | 0.120 | 0.265 | 4.75 |
| interior | secant | 5 | 0.109 | 0.510 | 0.280 | 0.601 | 4.42 |
| all | secant | 6 | 0.636 | 3.099 | 0.140 | 0.284 | 5.63 |
| interior | secant | 6 | 0.083 | 0.475 | 0.321 | 0.637 | 5.14 |
| all | proportional | 3 | 0.835 | 3.198 | 0.076 | 0.208 | 2.91 |
| interior | proportional | 3 | 0.178 | 0.672 | 0.173 | 0.464 | 2.79 |
| all | proportional | 4 | 0.727 | 3.152 | 0.093 | 0.238 | 3.83 |
| interior | proportional | 4 | 0.132 | 0.639 | 0.214 | 0.536 | 3.62 |
| all | proportional | 5 | 0.682 | 3.141 | 0.115 | 0.262 | 4.74 |
| interior | proportional | 5 | 0.111 | 0.521 | 0.268 | 0.595 | 4.40 |
| all | proportional | 6 | 0.645 | 3.099 | 0.127 | 0.279 | 5.62 |
| interior | proportional | 6 | 0.100 | 0.507 | 0.292 | 0.625 | 5.14 |
| all | seeds | 3 | 0.826 | 3.152 | 0.086 | 0.189 | 3.00 |
| interior | seeds | 3 | 0.191 | 0.826 | 0.196 | 0.417 | 3.00 |
| all | seeds | 4 | 0.756 | 3.141 | 0.105 | 0.223 | 4.00 |
| interior | seeds | 4 | 0.148 | 0.761 | 0.244 | 0.500 | 4.00 |
| all | seeds | 5 | 0.720 | 3.099 | 0.123 | 0.248 | 5.00 |
| interior | seeds | 5 | 0.125 | 0.727 | 0.280 | 0.548 | 5.00 |
| all | seeds | 6 | 0.666 | 3.078 | 0.147 | 0.277 | 6.00 |
| interior | seeds | 6 | 0.096 | 0.606 | 0.333 | 0.607 | 6.00 |
| all | bounded | 3 | 0.703 | 3.152 | 0.078 | 0.221 | 2.91 |
| interior | bounded | 3 | 0.163 | 0.575 | 0.179 | 0.494 | 2.79 |
| all | bounded | 4 | 0.668 | 3.141 | 0.105 | 0.245 | 3.83 |
| interior | bounded | 4 | 0.123 | 0.534 | 0.244 | 0.554 | 3.61 |
| all | bounded | 5 | 0.636 | 3.099 | 0.130 | 0.275 | 4.72 |
| interior | bounded | 5 | 0.101 | 0.506 | 0.298 | 0.607 | 4.37 |
| all | bounded | 6 | 0.607 | 3.078 | 0.150 | 0.289 | 5.59 |
| interior | bounded | 6 | 0.080 | 0.475 | 0.339 | 0.631 | 5.07 |

Saturation causes: `{"bounded": {"flat_response_elsewhere": 25, "model_boundary": 278, "not_saturated": 105}, "proportional": {"flat_response_elsewhere": 23, "lower_plateau": 81, "not_saturated": 108, "upper_plateau": 196}, "secant": {"flat_response_elsewhere": 29, "lower_plateau": 85, "not_saturated": 96, "upper_plateau": 198}}`.

| Scope | Alternative | Wins/ties/losses vs secant | Median paired error Δ | Mean paired error Δ |
| --- | --- | --- | --- | --- |
| all | proportional | 23/342/43 | 0.000 | 0.004 |
| all | seeds | 118/215/75 | 0.000 | -0.008 |
| all | bounded | 60/343/5 | 0.000 | -0.024 |
| interior | proportional | 23/102/43 | 0.000 | 0.009 |
| interior | seeds | 71/23/74 | 0.000 | 0.036 |
| interior | bounded | 14/149/5 | 0.000 | -0.001 |
| outside | proportional | 0/240/0 | 0.000 | 0.000 |
| outside | seeds | 47/192/1 | 0.000 | -0.039 |
| outside | bounded | 46/194/0 | 0.000 | -0.040 |

Same-seed trajectory steps with both requests inside the fitted table (a selected set of steps, not a random derivative sample):

| Rule | Steps | Slope median [p10,p90] | Negative slopes | Star Δ < .01 | Slopes outside [.5,2] |
| --- | --- | --- | --- | --- | --- |
| secant | 399 | 1.328 [-0.302,3.964] | 53 | 25 | 247 |
| proportional | 417 | 1.492 [-0.118,3.142] | 51 | 21 | 242 |
| bounded | 494 | 1.387 [-0.160,4.199] | 57 | 29 | 301 |

### Published examples selected to expose search failures

largest published secant miss: **saigetsu**, target 5.5, starting seed 0. These cases were selected after measurement to explain failures.

| Rule | Try | Requested | Seed | Measured | Absolute error |
| --- | --- | --- | --- | --- | --- |
| secant | 1 | 5.5000 | 0 | 3.9641 | 1.5359 |
| secant | 2 | 7.0359 | 0 | 4.2371 | 1.2629 |
| secant | 3 | 8.5000 | 0 | 4.2371 | 1.2629 |
| secant | 4 | 7.0359 | 1 | 3.4571 | 2.0429 |
| secant | 5 | 7.0359 | 2 | 3.5866 | 1.9134 |
| secant | 6 | 7.0359 | 3 | 3.5849 | 1.9151 |
| seeds | 1 | 5.5000 | 0 | 3.9641 | 1.5359 |
| seeds | 2 | 5.5000 | 1 | 4.3638 | 1.1362 |
| seeds | 3 | 5.5000 | 2 | 3.6576 | 1.8424 |
| seeds | 4 | 5.5000 | 3 | 3.4810 | 2.0190 |
| seeds | 5 | 5.5000 | 4 | 3.7426 | 1.7574 |
| seeds | 6 | 5.5000 | 5 | 3.5839 | 1.9161 |
| proportional | 1 | 5.5000 | 0 | 3.9641 | 1.5359 |
| proportional | 2 | 7.0359 | 0 | 4.2371 | 1.2629 |
| proportional | 3 | 8.2988 | 0 | 4.2371 | 1.2629 |
| proportional | 4 | 7.0359 | 1 | 3.4571 | 2.0429 |
| proportional | 5 | 7.0359 | 2 | 3.5866 | 1.9134 |
| proportional | 6 | 7.0359 | 3 | 3.5849 | 1.9151 |
| bounded | 1 | 5.5000 | 0 | 3.9641 | 1.5359 |
| bounded | 2 | 5.8750 | 0 | 4.2371 | 1.2629 |
| bounded | 3 | 5.8750 | 1 | 3.4571 | 2.0429 |
| bounded | 4 | 5.8750 | 2 | 3.5866 | 1.9134 |
| bounded | 5 | 5.8750 | 3 | 3.5849 | 1.9151 |
| bounded | 6 | 5.8750 | 4 | 3.2269 | 2.2731 |

largest published proportional advantage: **saigetsu**, target 4.0, starting seed 1000. These cases were selected after measurement to explain failures.

| Rule | Try | Requested | Seed | Measured | Absolute error |
| --- | --- | --- | --- | --- | --- |
| secant | 1 | 4.0000 | 1000 | 4.5083 | 0.5083 |
| secant | 2 | 3.4917 | 1000 | 3.4705 | 0.5295 |
| secant | 3 | 3.7565 | 1000 | 3.4705 | 0.5295 |
| secant | 4 | 4.0000 | 1001 | 3.4375 | 0.5625 |
| secant | 5 | 4.0000 | 1002 | 2.7758 | 1.2242 |
| secant | 6 | 4.0000 | 1003 | 3.5302 | 0.4698 |
| seeds | 1 | 4.0000 | 1000 | 4.5083 | 0.5083 |
| seeds | 2 | 4.0000 | 1001 | 3.4375 | 0.5625 |
| seeds | 3 | 4.0000 | 1002 | 2.7758 | 1.2242 |
| seeds | 4 | 4.0000 | 1003 | 3.5302 | 0.4698 |
| seeds | 5 | 4.0000 | 1004 | 2.9491 | 1.0509 |
| seeds | 6 | 4.0000 | 1005 | 3.2972 | 0.7028 |
| proportional | 1 | 4.0000 | 1000 | 4.5083 | 0.5083 |
| proportional | 2 | 3.4917 | 1000 | 3.4705 | 0.5295 |
| proportional | 3 | 4.0212 | 1000 | 4.5606 | 0.5606 |
| proportional | 4 | 3.4606 | 1000 | 3.4705 | 0.5295 |
| proportional | 5 | 3.9901 | 1000 | 4.1343 | 0.1343 |
| proportional | 6 | 3.8558 | 1000 | 3.4705 | 0.5295 |
| bounded | 1 | 4.0000 | 1000 | 4.5083 | 0.5083 |
| bounded | 2 | 3.4917 | 1000 | 3.4705 | 0.5295 |
| bounded | 3 | 3.7565 | 1000 | 3.4705 | 0.5295 |
| bounded | 4 | 4.0000 | 1001 | 3.4375 | 0.5625 |
| bounded | 5 | 4.0000 | 1002 | 2.7758 | 1.2242 |
| bounded | 6 | 4.0000 | 1003 | 3.5302 | 0.4698 |

## Workload r and measured star

| Arm | End | Median star Δ | Median r Δ | Opposite sign / nonzero pairs |
| --- | --- | --- | --- | --- |
| ceiling | low | 0.000 | 0.000 | 0/0 |
| ceiling | high | 0.000 | 0.000 | 0/0 |
| direct | low | -0.260 | -0.154 | 3/24 |
| direct | high | 0.957 | 0.380 | 0/24 |
| end_segment | low | -0.393 | 0.011 | 9/24 |
| end_segment | high | 0.797 | -0.104 | 20/24 |
| overall | low | -0.236 | 0.016 | 13/24 |
| overall | high | 0.935 | -0.099 | 17/24 |
| overall+direct | low | — | — | 0/0 |
| overall+direct | high | 1.372 | 0.029 | 11/24 |
| q_outer | low | -0.260 | -0.154 | 3/24 |
| q_outer | high | 0.601 | 0.217 | 1/23 |
| q_tested | low | -0.215 | -0.125 | 5/24 |
| q_tested | high | 0.475 | 0.149 | 1/20 |
