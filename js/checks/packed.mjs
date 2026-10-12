// Generates one chart with an installed @ensomi/runtime as a client would: the package's own Wasm core and resampler
// tables, a model directory read through a file: URL, and a release recording's PCM averaged to mono float32 at its
// recorded rate. The hit objects must equal the chart the native driver made from the same input
// (conformance/generate/native-charts).
//
// Run where the packed tarball is installed, with this file and recording.mjs copied there (RELEASING.md):
//   node packed.mjs --model-dir <ensomi-r2/onnx> --recording <runtime-ref/bulk/song> --chart <native-charts/song>
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { loadModels, generateChart } from '@ensomi/runtime';
import { Recording, parseOsu } from './recording.mjs';

const args = process.argv.slice(2), option = (name) => args[args.indexOf(name) + 1];
const fixture = JSON.parse(await readFile(`${option('--chart')}.json`, 'utf8'));
const expected = parseOsu(await readFile(`${option('--chart')}.osu`, 'utf8'));

const recording = await Recording.open(option('--recording'));
const audio = await recording.event('audio');
const channels = recording.shape(recording.index.events.find(e => e.name === 'audio').arrays.pcm)[1] ?? 1;
await recording.close();
const pcm = new Float32Array(audio.pcm.length / channels);
for (let i = 0; i < pcm.length; i++) {
    let sum = 0;
    for (let c = 0; c < channels; c++)
        sum += audio.pcm[i * channels + c];
    pcm[i] = sum / channels;
}

const models = await loadModels(pathToFileURL(resolve(option('--model-dir'))).href, { wasm: { numThreads: 1 } });
const start = performance.now();
const result = await generateChart(models, pcm, Number(audio.sample_rate[0]), { ...fixture.request, seed: fixture.seed });
const seconds = (performance.now() - start) / 1000;
await models.dispose();
const same = JSON.stringify(result.chart.objects) === JSON.stringify(expected);
console.log(JSON.stringify({ sample_rate: Number(audio.sample_rate[0]), objects: result.chart.objects.length, expected: expected.length, same_hit_objects: same, seconds }));
if (!same)
    process.exitCode = 1;
