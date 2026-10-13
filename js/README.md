# @ensomi/runtime

The Ensomi chart generator in the browser and in Node: the shared Rust core of
[ensomi-labs/runtime](https://github.com/ensomi-labs/runtime) compiled to WebAssembly, with the model's graphs
run by ONNX Runtime Web.

```js
import { loadModels, generateChart } from '@ensomi/runtime';

// The model's onnx/ directory (sed-i/ensomi-r2): ensomi-package.json and the files it lists.
const models = await loadModels(modelBaseUrl);
const result = await generateChart(models, pcm, sampleRate, { star: 3.5, seed: 7, title: 'Song', artist: 'Artist' });
result.chart;    // EnsomiProtocol chart: {duration_ms, objects: [{lane, start_ms, end_ms?}]}
result.osu;      // the chart as an osu!mania beatmap
result.receipt;  // EnsomiProtocol GenerationReceipt
```

R2 arranges the song once. The published Python R2 (`r2-1.0`) arranged it a second time when the first arrangement
missed the star's difficulty target by more than the difficulty fit's seed spread, so its chart differs wherever it
did; the receipt's `controls.difficulty` records the miss.

Playing while generating, from the audio stages run when the song was chosen:

```js
import { prepare, Prepared, ChartScheduler } from '@ensomi/runtime';

const prepared = await prepare(models, pcm, sampleRate);   // or await Prepared.fromBytes(cachedBytes)
cache.put(songHash, prepared.bytes());
const scheduler = await ChartScheduler.create(models, prepared, { star: 3.5, seed: 7 }, {
    mode: 'ahead-of-play', judge: 'C', renderLeadMs: scrollTimeMs, marginMs: 0,
    onPage: page => {},                                     // {tokens: [{lane, time_ms, kind}], coverage_ms, end_of_stream}
});
scheduler.start(0);
const { joinMs } = await scheduler.work();                 // play may start
scheduler.work().then(({ result }) => {});                 // generates ahead of play; the whole chart at the end
// each frame: scheduler.heard(chartMs, performance.now())
```

- `'ahead-of-play'` commits a page per head row of R2's arrangement; `'whole-chart'` sends the chart in one page at
  the end. Both make the same chart.
- `work()` resolves with `{kind: 'ready', joinMs}` once play from there stays behind the chart to the end, and with
  `{kind: 'done', result}` at the end; while play is far enough ahead it waits for `heard`, `start` or `stop`.
  `start(atMs)` at a position joins at the first time at or after `atMs` plus the lead where no hold is open, and
  sends the pages again from there. `rate` is the measured generation speed (chart ms per wall ms).
- `request(request)` changes `star` and `controls` for the chart in progress; `seed`, `audio`, `title` and `artist`
  stay. It is for requests from game events during play (multiplayer attacks, players coordinating), which a pipeline
  that resumes from a committed prefix will apply. R2 takes one until the first page is committed, from PCM or a
  `Prepared`, and the chart is then the one a fresh generation with it makes (with `'whole-chart'`, which commits at
  the end, a request during the arrangement restarts it); later it throws and the generation goes on unchanged.

- `pcm` is mono `Float32Array` or `Float64Array`, or `{samples, channels}` with interleaved samples. PCM at a rate
  other than 22050 Hz is resampled with the soxr tables in the package's `resampler/` directory.
- `loadModels` fetches the package's manifest, `ensomi-package.json` ([docs/model-package.md](../docs/model-package.md)),
  first. With it, it checks the manifest, fetches the config it names and refuses a manifest that leaves out a file
  that config names, as Swift's `open` does, before fetching anything else; then it fetches the other files it lists
  and checks each one's SHA-256 with `crypto.subtle` (in a browser, a secure context), and reads no other file; a
  mismatch rejects, naming the file.
  `models.package` is the checked manifest and `models.notices` its licence notices, BeatThis's MIT licence
  (`BeatThis-LICENSE.txt`), as `[{path, text}]` for the app to show with the model. `checkPackage(json)` resolves to
  the same checked manifest, `{model, license, notices, files: [{path, size, sha256}], totalBytes, config}`, and
  rejects a format other than `ensomi-package/1`, a `min_runtime` above this runtime's version, another model or
  pipeline, a path outside the package or out of order, or notices or a config not among the files.
- Without the manifest, `loadModels` reads `runtime.json` and the files it names, as 0.2 did: a source signals a file
  it does not have by rejecting with an error whose `code` is `'ENOENT'`, as Node's `readFile` does, and `urlSource`
  does so for an HTTP 404 and `localFilesSource` for a file not picked. A source of your own that rejects otherwise
  for the manifest fails loading. `models.notices` are then the notices `runtime.json` names, leaving out one the
  source lacks (a folder installed without it), and `noticeFiles(runtimeConfigJson)` resolves to their paths.
- `loadModels` reads the Wasm core (`pkg/`) and the tables from beside its own module: with `fetch` in a browser,
  from the file system in Node 22.3 or later. A bundler that moves the module passes `options.core` and
  `options.runtimeSource`. `src/index.d.ts` lists every option.
- The Wasm core uses WebAssembly SIMD, as ONNX Runtime Web does: Chrome 91, Firefox 89, Safari 16.4 or later.
- A panic in the core rejects its generation with a `WebAssembly.RuntimeError` carrying the panic's message, which
  also goes to `console.error`. The core's instance is then replaced, and the next `generateChart` loads the model
  into the new one from `source` again.

Apache-2.0. The model (`sed-i/ensomi-r2`) is licensed CC BY-NC-SA 4.0, except BeatThis (CPJKU): `onnx/beatthis.onnx`,
an ONNX export of its `final0` checkpoint, and its front-end arrays `onnx/beatthis-mel-filter.bin` and
`onnx/beatthis-hann-window.bin` keep BeatThis's MIT licence (`onnx/BeatThis-LICENSE.txt`, in `models.notices`).
