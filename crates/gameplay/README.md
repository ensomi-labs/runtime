# ensomi-gameplay

The 4K mania gameplay rules of the macOS app, as a Rust crate with no IO and no clock: judgement and
scoring of a chart, complete or streamed, against timestamped key input, the presentation state of judgement
popups and lane lighting, the screen position of notes, and the `.osu` loader. Behaviour is pinned by
[`conformance/gameplay`](../../conformance/gameplay).

```rust
let mut session = Session::new(&chart, Ruleset { judge: Judge::C })?;
let events = session.input(InputEvent { lane: 0, phase: Phase::Press, time_ms: 1040.0 });
let events = session.advance(1500.0);   // every input stamped at or before 1500 has been delivered
let score = session.score();

let mut session = Session::streaming(Ruleset { judge: Judge::C });
let events = session.append(&page.objects, page.coverage_ms.into(), page.end_of_stream)?;
let behind = session.decided_through_ms() < watermark_ms;   // after advance(watermark_ms)

let beatmap = parse_osu(&text)?;   // beatmap.chart, beatmap.title, beatmap.artist
```

## Inputs and outputs

- **Chart**: EnsomiProtocol's `Chart { duration_ms, objects }`, objects `{ lane 0-3, start_ms, end_ms? }` in audio
  milliseconds, sorted by start then lane. `Session::new` rejects what the protocol forbids (lane, order,
  time beyond the duration, a note starting before the previous note of its lane ends or starts). A hold
  may end where the next note of its lane starts. An `end_ms` equal to `start_ms` is accepted: the .osu
  loader produces such holds. A streamed chart arrives in pages instead (below).
- **Ruleset**: the judge difficulty A to E.
- **Input**: lane, press or release, chart time in milliseconds (audio time plus the audio offset, applied by
  the caller exactly once). `advance(watermark_ms)`: the caller declares that every input stamped at or
  before the watermark has been delivered.
- **Judgement events**, one per object, from `input`, `advance` and `append`: object index, lane, `Head` or `Tail`,
  tier, signed offset, chart time the outcome was decided, and the object's start time.
- **Score**: tier counts, accuracy, combo, max combo and the suggested audio offset.
- Also `lane_states()`, `visible_objects(lower, upper)` for the renderer and `is_resolved()` (the stream has
  ended and every object has an outcome).

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

## Streamed charts

`Session::streaming(ruleset)` starts without objects; pages add them. A page carries objects and a **coverage**:
every object starting at or before it has been delivered. Two forms:

- `append(objects, coverage_ms, end_of_stream)`: whole objects, as EnsomiProtocol's `ChartPage` sends them, each
  starting after the previous coverage, a hold with its final end.
- `append_tokens(tokens, coverage_ms, end_of_stream)`: the app's `Mania4KHitObject` form, `Tap`, `HoldStart`,
  `HoldEnd`. A hold's end may come in a later page; the coverage then also promises that every end at or before
  it has been delivered. Until its end arrives the hold is drawn `OpenEnded` (while not pressed), and no other
  note of its lane may arrive.

`end_of_stream` makes the coverage infinite. A page is rejected (`ChartError`) for what a chart forbids, for an
object or hold end at or before a coverage already declared (`BeforeCoverage`, also for anything after the end),
for a hold end without its start, and for a stream that ends with a hold still open. Taps and hold starts must
come in chart order; hold ends need only follow their start.

**The session decides nothing beyond the coverage.** Misses are resolved only as far as it, and an input waits
until the delivered objects settle it: its time must be covered, a press in a lane without a waiting note needs
the coverage `g` past it (a note could still arrive in reach), and the release of a hold whose end has not
arrived needs the coverage `1.5 g` past it (only then is the release known to be early) or the end. Inputs keep
their order; one that waits holds back those after it. Events come out of `input`, `advance` or `append`,
whichever call settles them, each with the time it was decided at as for a complete chart. The widest of these
waits, `1.5 g`, is `Ruleset::settle_lead_ms()`, the least lead a scheduler keeps the chart ahead of play.

So for given objects, inputs and advances, the events and score are those of the complete chart, whatever the
page schedule and advance cadence (the property test checks this for both forms).

**Falling behind.** `decided_through_ms()` is the chart time up to which the session has judged: the clock, held
back to the coverage and to the first waiting input. When it is below the watermark the caller passed to
`advance`, the stream has fallen behind play. The caller chooses what to do: wait for pages, or stop as the Swift
app does when its gameplay time passes the coverage. `coverage_ms()` reports the coverage.

## The .osu loader

`parse_osu(text)` is the app's `OsuMania4KParser` as a pure function. It requires `Mode` 3 and a `CircleSize`
within 0.0001 of 4, reads notes (type bit 1) and holds (bit 128) and rejects sliders, spinners and anything
unreadable, takes the lane from `floor(x * 4 / 512)` clamped to 0-3, and sorts by time, then lane, then file order.
A hold whose end is missing or before its start becomes a zero-length hold. The result must be a valid chart
(`OsuError::InvalidChart`). Title and artist prefer the Unicode fields; `duration_ms` is the latest note time.

Two choices of this loader: lines end at any line break (LF, CRLF, lone CR and the other Unicode breaks, as
Foundation splits), and times are rounded to the nearest whole millisecond, since a chart's times are whole
milliseconds. Swift keeps fractional times.

## Differences from the Swift engine

Where Swift's result depends on how `advance`, inputs and stream reads interleave, the rules above win (8 is a
looser check):

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
6. **Judging ahead of the stream.** The Swift engine judges with whatever has been ingested; the app only checks
   that the gameplay time is covered. Here a press or release waits as described under streamed charts. The
   app reads at least 537 ms ahead (scroll time plus 250 ms), more than the 300 ms an input can need, so a
   stream that keeps up with it gives the same results.
7. **Releasing a hold whose end has not arrived.** Swift misses it at once. Here it waits for the end, or for
   the coverage to show the release is early.
8. **Token order.** Swift also rejects hold ends out of time order among all tokens; here hold ends need only
   come after their start and after the previous coverage.

Everything else is the Swift behaviour: windows, tiers, tail multiplier, hold rules, note lock, ignoring early
presses, scoring and the offset suggestion.

## Not ported

- The stream readers (`Mania4KHitObjectStreaming`, cursors, read-through): the caller reads its source and passes
  pages.
- The key bindings and keyboard router (they stay in the app).
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

`cargo test -p ensomi-gameplay`: every fixture (also re-run under several advance schedules, and a streamed one
with all its pages at once), the `.osu` fixtures, property tests of tick independence over random charts, logs
and schedules, complete and streamed as objects or tokens under random page schedules, and of the per-frame
queries (`visible_objects`, `lane_states`, `is_resolved`) against a linear reference (`proptest`), stream
validation, and the cases of the Swift feedback and layout tests.

`cargo test --release -p ensomi-gameplay --test frame_cost -- --ignored --nocapture` times a 60 Hz frame
(`advance`, `visible_objects`, `lane_states`, `score`, `is_resolved`) over charts of 4,000 and 16,000
objects; the frame costs the same for both.
