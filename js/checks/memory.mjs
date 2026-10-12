// Memory of two generations in a row through the Wasm core and ONNX Runtime Web's WASM backend in Node: the core's
// linear memory (which never shrinks) after loading and after each generation, and the process's resident size
// sampled during each generation. The input is a recording's PCM with its channels averaged to mono float32: by
// default at the recorded rate without frontend streams, as a native client passes it; with --browser, as the
// browser's decodeAudio gives it, the recorded 24000 Hz Mel stream as samples and Mel stream, and a 22050 Hz
// BeatThis stream (here picked from the PCM by nearest sample). The request is the release example's star, seed,
// title and artist.
//
// Usage: node --expose-gc js/checks/memory.mjs --model-dir <ensomi-r2> --reference <runtime-ref> --core <wasm> [--browser] [--out <report.json>] [song]
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import * as ort from 'onnxruntime-web/wasm';
import init from '../pkg/ensomi_generate_wasm.js';
import { loadModels, generateChart } from '../src/index.js';
import { Recording } from './recording.mjs';

const args = process.argv.slice(2), option = (name) => { const i = args.indexOf(name); return i < 0 ? undefined : args.splice(i, 2)[1]; };
const modelDir = option('--model-dir'), referenceDir = option('--reference'), corePath = option('--core'), out = option('--out');
const browser = args.includes('--browser') && args.splice(args.indexOf('--browser'), 1).length > 0;
const song = args[0] ?? 'adventure';
const mib = (bytes) => Math.round(bytes / 1048576 * 10) / 10;
const coreMemory = async () => mib((await init()).memory.buffer.byteLength);

/** The recorded PCM with its channels averaged to mono float32, and its sample rate. */
async function monoPcm() {
    const recording = await Recording.open(join(referenceDir, 'bulk', song));
    const audio = await recording.event('audio');
    const channels = recording.shape(recording.index.events.find(e => e.name === 'audio').arrays.pcm)[1] ?? 1;
    const pcm = new Float32Array(audio.pcm.length / channels);
    for (let i = 0; i < pcm.length; i++) {
        let sum = 0;
        for (let c = 0; c < channels; c++)
            sum += Number(audio.pcm[i * channels + c]);
        pcm[i] = sum / channels;
    }
    await recording.close();
    const rate = Number(audio.sample_rate[0]);
    if (!browser)
        return [pcm, rate];
    const beatthis = Float32Array.from({ length: Math.floor(pcm.length * 22050 / rate) }, (_, i) => pcm[Math.floor(i * rate / 22050)]);
    return [{ samples: audio.mel_pcm, channels: 1, mel: audio.mel_pcm, beatthis }, 24000];
}
const [pcm, sampleRate] = await monoPcm();
const receipt = JSON.parse(await readFile(join(modelDir, 'examples', song, 'receipt.json'), 'utf8'));
const request = { star: receipt.star, seed: receipt.seed, title: receipt.metadata.title, artist: receipt.metadata.artist };

const models = await loadModels(async (p) => new Uint8Array(await readFile(join(modelDir, 'onnx', p))), { ort, core: await readFile(corePath), backend: 'wasm', wasm: { numThreads: 1 } });
globalThis.gc?.();
const samples = pcm.samples ?? pcm;
const report = { song, input: browser ? 'browser' : 'native', sample_rate: sampleRate, seconds: samples.length / sampleRate, pcm_mib: mib(samples.byteLength + (pcm.beatthis?.byteLength ?? 0)), loaded: { core_mib: await coreMemory(), rss_mib: mib(process.memoryUsage.rss()) }, generations: [] };
const charts = [];
for (let run = 0; run < 2; run++) {
    let peak = process.memoryUsage.rss();
    const sampler = setInterval(() => { peak = Math.max(peak, process.memoryUsage.rss()); }, 20);
    const start = performance.now();
    const result = await generateChart(models, pcm, sampleRate, request, { onProgress: () => { peak = Math.max(peak, process.memoryUsage.rss()); } });
    clearInterval(sampler);
    peak = Math.max(peak, process.memoryUsage.rss());
    charts.push(JSON.stringify(result.chart));
    globalThis.gc?.();
    report.generations.push({ wall_seconds: (performance.now() - start) / 1000, objects: result.chart.objects.length, core_mib: await coreMemory(), rss_peak_mib: mib(peak), rss_after_mib: mib(process.memoryUsage.rss()) });
}
report.same_chart = charts[0] === charts[1];
await models.dispose();
console.log(JSON.stringify(report, null, 2));
if (out)
    await writeFile(out, JSON.stringify(report, null, 2) + '\n');
