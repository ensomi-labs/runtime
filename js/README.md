# @ensomi/runtime

The Ensomi chart generator in the browser and in Node: the shared Rust core of
[ensomi-labs/runtime](https://github.com/ensomi-labs/runtime) compiled to WebAssembly, with the model's graphs
run by ONNX Runtime Web.

```js
import { loadModels, generateChart } from '@ensomi/runtime';

// The model's onnx/ directory (sed-i/ensomi-r2): runtime.json and the files it names.
const models = await loadModels(modelBaseUrl);
const result = await generateChart(models, pcm, sampleRate, { star: 3.5, seed: 7, title: 'Song', artist: 'Artist' });
result.chart;    // EnsomiProtocol chart: {duration_ms, objects: [{lane, start_ms, end_ms?}]}
result.osu;      // the chart as an osu!mania beatmap
result.receipt;  // EnsomiProtocol GenerationReceipt
```

- `pcm` is mono `Float32Array` or `Float64Array`, or `{samples, channels}` with interleaved samples. PCM at a rate
  other than 22050 Hz is resampled with the soxr tables in the package's `resampler/` directory.
- `loadModels` reads the Wasm core (`pkg/`) and the tables from beside its own module: with `fetch` in a browser,
  from the file system in Node 22.3 or later. A bundler that moves the module passes `options.core` and
  `options.runtimeSource`. `src/index.d.ts` lists every option.
- The Wasm core uses WebAssembly SIMD, as ONNX Runtime Web does: Chrome 91, Firefox 89, Safari 16.4 or later.
- A panic in the core rejects its generation with a `WebAssembly.RuntimeError` carrying the panic's message, which
  also goes to `console.error`. The core's instance is then replaced, and the next `generateChart` loads the model
  into the new one from `source` again.

Apache-2.0. The model weights carry their own licence (CC BY-NC-SA 4.0).
