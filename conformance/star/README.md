# osu!mania star rating (20241007)

`vectors.json` is an array of four-key charts at clock rate 1. Each entry has a `name`,
`star_rating` (a double), and either `objects` or `osu` (a path relative to this directory).
Objects have `lane` (0–3), `start_ms`, and optional `end_ms`, in whole audio milliseconds.
Preserve their input order, including simultaneous notes. No chart duration enters the calculation.
Empty and one-note charts measure zero. Compare nonzero values with relative tolerance 1e-9.

The small charts were written for this fixture. The only other chart is the runtime's existing
`conformance/generate/native-charts/hesitation-snow.osu`, which measures 2.78122023258017.
No private dataset charts are included.

Expected values were produced with Python 3 and the standard library: read
`src/ensomi_model/osu_core/difficulty.py` using `git show r2-1.0.1:<path>` from ensomi-model,
execute that module, construct `RawHitObject(start_ms, end_ms or start_ms, lane)` for each
object (use `parse_osu_file` for the `.osu` entry), then call
`compute_mania_star_rating_20241007(raw_objects, 4, 1.0)`. JSON preserves the resulting doubles.
The generating script is not part of this fixture.

The calculator uses the reference's legacy depth-limited quicksort, rather than a stable sort.
`legacy-simultaneous` measures 0.7545548706241386 with that sort and 0.7747606838001129
with a stable time sort, so it detects a change in the treatment of equal start times.
