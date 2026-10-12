// Time to first playable through the JavaScript API in Node, ONNX Runtime Web's WASM backend on one thread: per song,
// the seconds from the start of a generation to ChartScheduler's first `ready` at position 0, playing while generating
// ('ahead-of-play') against the whole chart, from PCM and from a stored Prepared; then to `done`, with the throughput r
// (committed chart milliseconds per wall millisecond of work) at `ready` and at the end. The input and lead are those
// of crates/ffi/examples/stream.rs: the recorded PCM averaged to mono float32 at its recorded rate, the release
// example's request, judge C's settle lead, the render lead given, no margin. One `prepare` per song first warms the
// graphs and gives the bytes; "from a stored Prepared" starts from those bytes in memory, reading them included.
//
// Usage: node stream.mjs --model-dir <ensomi-r2> --reference <runtime-ref> --assets <resampler tables> --core <wasm>
//        --out <report.json> [--runs 3] [--render-lead-ms 459.4] [song...]
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import * as ort from 'onnxruntime-web/wasm';
import { loadModels, prepare, ChartScheduler, Prepared } from '../src/index.js';
import { Recording } from './recording.mjs';

const args = process.argv.slice(2), option = (name) => { const i = args.indexOf(name); return i < 0 ? undefined : args.splice(i, 2)[1]; };
const modelDir = option('--model-dir'), referenceDir = option('--reference'), assetsDir = option('--assets'), corePath = option('--core'), out = option('--out');
const runs = Number(option('--runs') ?? 3), renderLeadMs = Number(option('--render-lead-ms') ?? 459.4);
const songs = args.length ? args : ['hesitation-snow', 'saigetsu', 'catch-the-moment', 'adventure'];
const models = await loadModels(async (p) => new Uint8Array(await readFile(join(modelDir, 'onnx', p))), { ort, core: await readFile(corePath), backend: 'wasm', wasm: { numThreads: 1 }, runtimeSource: async (p) => new Uint8Array(await readFile(join(assetsDir, p))) });
const report = { what: 'seconds from the start of a generation to the first ready at 0 and to done, @ensomi/runtime in Node, onnxruntime-web WASM backend, one thread', lead: { judge: 'C', render_ms: renderLeadMs, margin_ms: 0 }, songs: {} };
const median = (values) => values.length ? values.toSorted((a, b) => a - b)[Math.floor(values.length / 2)] : null;

async function input(song) {
    const recording = await Recording.open(join(referenceDir, 'bulk', song));
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
    return { pcm, sampleRate: Number(audio.sample_rate[0]) };
}

async function run(source, request, mode, started) {
    let pages = 0;
    const scheduler = await ChartScheduler.create(models, source(), request, { mode, judge: 'C', renderLeadMs, marginMs: 0, onPage: () => pages++ });
    try {
        scheduler.start(0);
        const ready = await scheduler.work();
        const readySeconds = (performance.now() - started) / 1000, rateAtReady = scheduler.rate ?? null, pagesAtReady = pages;
        const done = await scheduler.work();
        const doneSeconds = (performance.now() - started) / 1000;
        if (ready.kind !== 'ready' || ready.joinMs !== 0 || done.kind !== 'done')
            throw new Error(`scheduler: ${JSON.stringify([ready, done.kind])}`);
        const receipt = JSON.parse(done.result.receipt.receipt_json);
        return { ready_seconds: readySeconds, done_seconds: doneSeconds, pages_at_ready: pagesAtReady, pages, rate_at_ready: rateAtReady, rate_at_done: scheduler.rate ?? null, objects: done.result.chart.objects.length, passes: receipt.controls.difficulty.passes.length, seconds: receipt.seconds };
    }
    finally {
        scheduler.free();
    }
}

for (const song of songs) {
    const { pcm, sampleRate } = await input(song);
    const example = JSON.parse(await readFile(join(modelDir, 'examples', song, 'receipt.json'), 'utf8'));
    const request = { star: example.star, seed: example.seed, title: example.metadata.title, artist: example.metadata.artist };
    let started = performance.now();
    const bytes = (await prepare(models, pcm, sampleRate)).bytes();
    const prepareSeconds = (performance.now() - started) / 1000;
    const variants = {};
    for (let r = 0; r < runs; r++) {
        for (const [name, mode, stored] of [['stream_pcm', 'ahead-of-play', false], ['whole_pcm', 'whole-chart', false], ['stream_prepared', 'ahead-of-play', true], ['whole_prepared', 'whole-chart', true]]) {
            started = performance.now();
            const source = stored ? await Prepared.fromBytes(bytes) : { pcm, sampleRate };
            const row = await run(() => source, request, mode, started);
            console.log(`${song} ${name}: ready ${row.ready_seconds.toFixed(3)} s, done ${row.done_seconds.toFixed(3)} s, r at ready ${row.rate_at_ready}`);
            (variants[name] ??= []).push(row);
        }
    }
    const summary = Object.fromEntries(Object.entries(variants).map(([name, rows]) => [name, Object.fromEntries(['ready_seconds', 'done_seconds', 'rate_at_ready', 'rate_at_done'].map(key => [key, median(rows.map(row => row[key]).filter(x => x !== null))]))]));
    report.songs[song] = { sample_rate: sampleRate, song_ms: pcm.length / sampleRate * 1000, prepare_seconds: prepareSeconds, prepared_bytes: bytes.length, median: summary, runs: variants };
    await writeFile(out, JSON.stringify(report, null, 2) + '\n');
}
await models.dispose();
