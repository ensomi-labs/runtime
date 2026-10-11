# Gameplay conformance fixtures

Each `*.json` file is one case: a chart, a ruleset, a sequence of inputs and advances, and the judgement events
and final score an implementation of the gameplay rules must produce. A streamed case has no chart; its objects
arrive in `page` steps. The `.osu` loader's cases are in [`osu/`](#osu-files). The rules are in
[`crates/gameplay/README.md`](../../crates/gameplay/README.md); the fixtures pin them. The files are written to be read by any
implementation (Rust, Swift, Kotlin, TypeScript) without the Rust code.

## Schema

```json
{
  "description": "free text",
  "chart": {
    "duration_ms": 3000,
    "objects": [
      {"lane": 0, "start_ms": 1000},
      {"lane": 1, "start_ms": 1500, "end_ms": 2000}
    ]
  },
  "ruleset": {"judge": "C"},
  "steps": [
    {"input": {"lane": 0, "phase": "press", "time_ms": 1040}},
    {"advance": 1500},
    {"input": {"lane": 1, "phase": "release", "time_ms": 2010.5}}
  ],
  "expected": {
    "events": [
      {"object": 0, "lane": 0, "part": "head", "tier": "bigP", "offset_ms": 40, "time_ms": 1040}
    ],
    "score": {
      "tier_counts": {"bigP": 1, "p1": 0, "p2": 0, "p3": 0, "g": 0, "m": 0},
      "accuracy": 1,
      "combo": 1,
      "max_combo": 1,
      "suggested_audio_offset_ms": -40
    },
    "late_inputs": 0
  }
}
```

| Field | Meaning |
| --- | --- |
| `chart` | EnsomiProtocol's `Chart` with proto field names: `duration_ms`, `objects` of `lane` (0-3), `start_ms`, optional `end_ms` (a hold). Sorted by `start_ms`, then `lane`. An object's number below is its index in `objects`. |
| `ruleset.judge` | `"A"` to `"E"`: the half-window table in the crate README. |
| `steps` | Executed in order. `{"input": ...}` delivers a key transition: `lane` 0-3, `phase` `"press"` or `"release"`, `time_ms` chart time as a number (fractions allowed). `{"advance": w}` declares every input stamped at or before `w` delivered. Streamed cases also have `{"page": ...}` and `{"decided_through_ms": t}` (below). |
| `expected.events` | Every judgement event the steps produce, in the order produced (call by call, an input's or advance's events in order), one per object. |
| `event.object`, `event.lane` | Index in `objects` and its lane. |
| `event.part` | `"head"`: decided at a tap, or at a hold whose head had not been held yet. `"tail"`: decided on a hold already held. |
| `event.tier` | `"bigP"`, `"p1"`, `"p2"`, `"p3"`, `"g"`, `"m"`. |
| `event.offset_ms` | Press error (press time minus note time) of the tap or hold head, or `null` when there is none (a miss). For a hold it is the head's error, also on a tail event. |
| `event.time_ms` | Chart time at which the outcome was decided: the input's time (or the clock, for a late one), or for a miss the time it fell due. |
| `event.step` | Optional; in streamed cases. Index in `steps` (from 0) of the step whose call returned the event. |
| `expected.score` | Score after the last step: `tier_counts`, `accuracy` (mean tier weight, 1 for no objects), `combo`, `max_combo`, `suggested_audio_offset_ms` (minus the mean `offset_ms` of non-miss events, `null` if none). |
| `expected.late_inputs` | Inputs delivered stamped before the session clock, handled at the clock. |

Numbers: times are milliseconds. Compare `time_ms` and `offset_ms` with a tolerance of `1e-6` and `accuracy` and the
suggested offset with `1e-9`; counts and names exactly.

A fixture is complete: its last step is an `advance` far beyond the chart, so that every miss has been decided.
Inputs are in nondecreasing time order unless the fixture is about late delivery.

## Streamed cases

A case without `chart` starts an empty streaming session (`Session::streaming`) and receives its objects in page
steps, in one of two forms, not mixed:

```json
{"page": {"objects": [{"lane": 1, "start_ms": 1500}], "coverage_ms": 2400, "end_of_stream": false}}
{"page": {"tokens": [{"lane": 0, "time_ms": 2000, "kind": "hold_end"}], "coverage_ms": 2600, "end_of_stream": false}}
```

| Field | Meaning |
| --- | --- |
| `page.objects` | Whole objects as in `chart.objects` (EnsomiProtocol's `ChartPage`): `Session::append`. |
| `page.tokens` | The macOS app's form: `kind` `"tap"`, `"hold_start"` or `"hold_end"`, `time_ms` a number; a hold's end may come in a later page: `Session::append_tokens`. |
| `page.coverage_ms` | Every object (and token) at or before it has been delivered. |
| `page.end_of_stream` | Every object has been delivered. |
| `{"decided_through_ms": t}` | An assertion, not an action: `Session::decided_through_ms` returns `t` here. |

Objects are numbered in the order they arrive (taps and hold starts for tokens), which is chart order. Every page
is valid; the last step of a case comes after the end of the stream. Besides reproducing the expected output as
written, a streamed case must give the same events (ignoring `step`) and score with all its pages delivered at
once before the first step.

## Schedule independence

For every fixture with `late_inputs` of 0, the expected events and score are the same under any other advance
schedule: drop the `advance` steps, or add `advance` steps anywhere before an input stamped later than their
value, and keep the final one. Pages stay where they are; `step` is not compared. The Rust test re-runs each
fixture under several schedules to check that.

## Files

- `tap_windows_*`, `hold_head_windows_*`, `hold_tail_windows_*` for judges A to E: every window edge on both
  sides, and half a millisecond past it. Tail edges are 1.5 times the head windows.
- Holds: `hold_on_time`, `hold_late_tail_is_good`, `hold_released_early_breaks`, `hold_released_at_early_edge`,
  `hold_released_late_is_a_timeout`, `hold_never_released`, `hold_never_pressed`, `hold_worse_of_head_and_tail`,
  `hold_zero_length`, `hold_release_at_next_note_start`.
- Taps and windows: `tap_offset_suggestion`, `early_press_ignored`, `window_end_input_wins`,
  `press_after_window_is_a_timer_miss`, `press_after_window_goes_to_next_note`, `release_without_hold_ignored`,
  `press_after_last_note_ignored`.
- Note lock: `note_lock_press`, `note_lock_without_input`, `note_lock_hold_tail_release`.
- Order of simultaneous outcomes: `lock_before_window_at_equal_time`, `simultaneous_misses_in_chart_order`,
  `misses_without_input`.
- Score and lanes: `combo_and_accuracy`, `chord_and_lanes`, `empty_chart`.
- Chart time before 0: `input_before_zero`.
- Late delivery: `late_delivery_hit`, `late_delivery_after_miss`, `late_delivery_release`, `late_delivery_out_of_order`.
- `one_log_sparse_advances` and `one_log_dense_advances`: one log under two schedules, same expectations.
- Streams: `stream_coverage_lags_play` (advances and inputs past the coverage), `stream_press_waits_for_a_note_within_reach`,
  `stream_note_lock_across_pages`, `stream_end_of_stream`, and in token form `stream_hold_end_arrives_late` and
  `stream_tokens_zero_length_hold_and_note_at_hold_end`.

The cases come from the engine tests of the macOS app (`Mania4KPlaySessionModelTests`) translated to this schema,
and from the rules in the crate README. Where the new rules change a Swift result, the fixture description says so.

## .osu files

`osu/NAME.osu` is a small hand-written beatmap and `osu/NAME.json` what `parse_osu` makes of it (rules in the crate
README). The bytes matter (line ends), so `.osu` files are stored without line-end conversion.

```json
{
  "description": "free text",
  "expected": {
    "chart": {"duration_ms": 2250, "objects": [{"lane": 0, "start_ms": 1000}]},
    "title": "Unicode Title",
    "artist": null
  }
}
```

or, for a file that is rejected, `"expected": {"error": {"kind": ...}}` with `kind` one of `"unsupported_mode"`,
`"unsupported_key_count"` (with `keys`), `"unsupported_hit_object"` (with `line`, the trimmed line) and
`"invalid_chart"` (with `chart_error`, the `ChartError` variant in snake case, and `object`).
