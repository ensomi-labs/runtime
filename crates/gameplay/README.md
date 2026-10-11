# ensomi-gameplay

The 4K mania gameplay rules of the macOS app, as a Rust crate with no IO and no clock: judgement and
scoring of a chart against timestamped key input, the presentation state of judgement popups and lane
lighting, and the screen position of notes. Behaviour is pinned by [`conformance/gameplay`](../../conformance/gameplay).

```rust
let mut session = Session::new(&chart, Ruleset { judge: Judge::C })?;
let events = session.input(InputEvent { lane: 0, phase: Phase::Press, time_ms: 1040.0 });
let events = session.advance(1500.0);   // every input stamped at or before 1500 has been delivered
let score = session.score();
```

## Inputs and outputs

- **Chart**: EnsomiProtocol's `Chart { duration_ms, objects }`, objects `{ lane 0-3, start_ms, end_ms? }` in audio
  milliseconds, sorted by start then lane. `Session::new` rejects what the protocol forbids (lane, order,
  time beyond the duration, a note starting before the previous note of its lane ends or starts). A hold
  may end where the next note of its lane starts. An `end_ms` equal to `start_ms` is accepted: the Swift
  .osu loader produces such holds.
- **Ruleset**: the judge difficulty A to E.
- **Input**: lane, press or release, chart time in milliseconds (audio time plus the audio offset, applied by
  the caller exactly once). `advance(watermark_ms)`: the caller declares that every input stamped at or
  before the watermark has been delivered.
- **Judgement events**, one per object, from `input` and `advance`: object index, lane, `Head` or `Tail`,
  tier, signed offset, chart time the outcome was decided, and the object's start time.
- **Score**: tier counts, accuracy, combo, max combo and the suggested audio offset.
- Also `lane_states()`, `visible_objects(lower, upper)` for the renderer and `is_resolved()`.

## Rules

Judge windows are inclusive half-windows in milliseconds around the note time; the error is press time minus
note time.

| Judge | bigP | p1 | p2 | p3 | g |
| --- | ---: | ---: | ---: | ---: | ---: |
| A | 60 | 105 | 120 | 135 | 200 |
| B | 47 | 63 | 88 | 118 | 200 |
| C | 40 | 56 | 81 | 111 | 200 |
| D | 34 | 50 | 75 | 105 | 200 |
| E | 25 | 50 | 75 | 105 | 200 |

- **Tiers** `bigP`, `p1`, `p2`, `p3`, `g`, `m`. `bigP` shows Perfect, `p1` to `g` Good, `m` Miss.
- **Tap**: a press resolves the first unresolved waiting note of its lane unless the error is below `-g`, in
  which case the press is ignored (too early). A note not hit by `start + g` is missed.
- **Hold**: the head is judged like a tap but only starts the hold. Releasing within the tail windows (the
  head windows times 1.5, so `±1.5 g` at the outside) resolves the hold with the worse of the head and tail
  tiers. Releasing before `end - 1.5 g` misses it, as does still holding after `end + 1.5 g`, as does a head
  that was never hit. A hold produces one event; its `offset_ms` is the head's error, also when it ends in a
  tail outcome. There is no recovery from a miss of either end.
- **Note lock**: an unresolved note stops being hittable when the next unresolved note of its lane starts: it
  is missed at that start time. This applies to a held hold too.
- **Score**: accuracy is the mean of the weights 1, .9, .85, .8, .4, 0 (bigP, p1, p2, p3, g, m) over resolved
  objects, 1 before any. A hit raises combo, a miss resets it. The suggested audio offset is minus the mean
  error of hit taps and hold heads (misses contribute nothing), `None` before the first.

## Time and tick independence

For a chart, ruleset and input log, the events and the final score do not depend on when or how often
`advance` is called. Time-based outcomes are decided at their own chart times, not when a call observes them:

| Outcome | Decided at | Due for an observer at time `t` when |
| --- | --- | --- |
| Note lock miss | start of the next note of the lane | `t >= start` |
| Waiting note not hit | `start + g` | `t > start + g` |
| Held hold not released | `end + 1.5 g` | `t > end + 1.5 g` |

An input at `t` first resolves everything due at `t` in all lanes, in order of decision time, locks before
window ends at the same time, then chart order; then it is handled. So a press stamped exactly on the end of a
window still hits, and a press stamped on the start of the next note finds the earlier one force-missed.
`advance(w)` resolves what is due at `w`. Events therefore come in order of decision time, and the
concatenated events of all calls are the same for every advance schedule; `advance(f64::INFINITY)` resolves
everything that is left.

**Late delivery.** The session clock is the largest watermark or input time seen. An input stamped before it
is handled at the clock (its error is computed from the clock) and counted in `Session::late_inputs`. This
covers an input arriving after a watermark that claimed it was delivered and one arriving out of order.
Outcomes of such logs depend on the schedule by definition; the counter is the diagnostic. Fixtures with a
non-zero `late_inputs` are the only ones exempt from the schedule-independence check.

Inputs for lanes outside 0 to 3 are ignored.

## Differences from the Swift engine

Where Swift's result depends on how `advance` and inputs interleave, the rules above win:

1. **Chart time and order of time-based events.** Swift stamps misses with the time of the `advance` that notices
   them (a note lock with the time of the press or advance), so a hit made after a note's window closed, before
   a call noticed the miss, extends the combo and the miss then resets it. Here misses carry the time they were due
   and come before any later hit, so combo and max combo no longer depend on the cadence either.
2. **A press after the window closed, before an advance noticed.** Swift consumes the press as a miss of the
   old note, with a hit error and the press's time. Here the note was already missed at `start + g`, and the
   press goes to the next note of the lane or is ignored.
3. **A release after the tail window, before an advance noticed.** Swift misses the hold at the release time;
   here it was missed at `end + 1.5 g`.
4. **Other lanes' misses.** Swift's press and release resolve only their own lane's note lock; a miss in another
   lane waits for the next `advance`. Here every input resolves everything due, so the event order and times, and
   with them the combo, follow rule 1.
5. **Out-of-order input.** Swift's clock follows each input's stamp, also backwards. Here it never moves back
   (late delivery above).

Everything else is the Swift behaviour: windows, tiers, tail multiplier, hold rules, note lock, ignoring early
presses, scoring and the offset suggestion.

## Not ported

- Streaming ingest (objects arriving while playing, the stream watermark and cursors) and open-ended holds: a
  session takes a complete chart. The layout still accepts `OpenEnded` objects.
- The .osu parser and the key bindings and keyboard router (they stay in the app).
- `Mania4KJudgement`'s per-event `judgement` field and `Mania4KScoreState`'s `perfectCount`, `goodCount`,
  `missCount`, `averageHitErrorMs`: derive them (`Tier::judgement`, `TierCounts::{perfect, good, miss}`, the
  negated suggested offset).

## Feedback and layout

`GameplayFeedback` is the Swift `Mania4KGameplayFeedbackState`: the same constants and curves, with
`record_input(lane, phase, sequence_number, ui_time_ms)` taking the fields it uses rather than an input
event. Judgement popup choice, protection windows, the 600 ms batch retention and lane brightness follow the
Swift tests, ported in `tests/feedback.rs`.

`geometry`, `y_position` are `Mania4KNoteRenderLayout` with the frame replaced by `render_time_ms` and
`scroll_time_ms` and the unused `laneHeight` dropped. A hold's body height is `body_bottom_y - body_top_y`
and its centre is their mean.

## Tests

`cargo test -p ensomi-gameplay`: every fixture (also re-run under several advance schedules), a property test of tick
independence over random charts, logs and schedules (`proptest`), and the cases of the Swift feedback and layout tests.
