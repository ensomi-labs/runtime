// Runs the TypeScript runtime and the Wasm core in Node on the same PCM, requests and onnxruntime-web WASM backend,
// and compares charts, .osu text and receipts. Only the runtime name and wall-clock timings may differ.
//
// Run with cwd ensomi-web/packages/runtime and tsx, which reads the TypeScript runtime's sources (read-only):
//   node --import tsx <this file> --model-dir <ensomi-r2> --reference <runtime-ref> --assets assets --core <wasm> --out <report.json>
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import * as ort from 'onnxruntime-web/wasm';
import { loadModels as loadWasm, generateChart as generateWasm } from '../src/index.js';
import { loadModels as loadTs, generateChart as generateTs } from '../../../ensomi-web/packages/runtime/src/index.ts';
import { Recording } from './recording.mjs';

const args = process.argv.slice(2), option = (name) => { const i = args.indexOf(name); return i < 0 ? undefined : args.splice(i, 2)[1]; };
const modelDir = option('--model-dir'), referenceDir = option('--reference'), assetsDir = option('--assets'), corePath = option('--core'), out = option('--out');
const songs = args.length ? args : ['hesitation-snow', 'saigetsu', 'catch-the-moment', 'adventure'];
const seeds = [0, 1, 2];
const read = async (p) => new Uint8Array(await readFile(join(modelDir, 'onnx', p)));
const runtimeSource = async (p) => new Uint8Array(await readFile(join(assetsDir, p)));
const options = { ort, backend: 'wasm', wasm: { numThreads: 1 }, runtimeSource };

const report = { ort: '1.23.2', backend: 'wasm', threads: 1, node: process.version, seeds, cases: [], passed: false };
const save = () => writeFile(out, JSON.stringify(report, null, 2) + '\n');

let t = performance.now();
const tsModels = await loadTs(read, options);
report.ts_load_seconds = (performance.now() - t) / 1000;
t = performance.now();
const wasmModels = await loadWasm(read, { ...options, core: await readFile(corePath) });
report.wasm_load_seconds = (performance.now() - t) / 1000;
report.capabilities_equal = JSON.stringify(tsModels.capabilities) === JSON.stringify(wasmModels.capabilities);

const normalizeOsu = (text) => text.replace('// @ensomi/runtime ', '// ensomi-runtime ');
function normalizeReceipt(json) {
    return json
        .replace(/,\n  "seconds": \{[\s\S]*?\n  \}\n\}$/, '\n}')
        .replace(/"wall_seconds": [^\n]*/g, '"wall_seconds": 0')
        .replace('"name": "@ensomi/runtime"', '"name": "ensomi-runtime"');
}
function firstDifference(a, b) {
    const x = a.split('\n'), y = b.split('\n');
    for (let i = 0; i < Math.max(x.length, y.length); i++)
        if (x[i] !== y[i])
            return { line: i + 1, ts: x[i] ?? null, wasm: y[i] ?? null };
    return null;
}

async function compare(label, pcm, sampleRate, request) {
    let start = performance.now();
    const ts = await generateTs(tsModels, pcm, sampleRate, request);
    const tsWall = (performance.now() - start) / 1000;
    start = performance.now();
    const wasm = await generateWasm(wasmModels, pcm, sampleRate, request);
    const wasmWall = (performance.now() - start) / 1000;
    const tsDetail = JSON.parse(ts.receipt.receipt_json), wasmDetail = JSON.parse(wasm.receipt.receipt_json);
    const receiptDifference = firstDifference(normalizeReceipt(ts.receipt.receipt_json), normalizeReceipt(wasm.receipt.receipt_json));
    const osuDifference = firstDifference(normalizeOsu(ts.osu), wasm.osu);
    const chartEqual = JSON.stringify(ts.chart) === JSON.stringify(wasm.chart);
    const fields = ['model_name', 'model_version', 'seed', 'star'].every(k => ts.receipt[k] === wasm.receipt[k]);
    const entry = {
        ...label, objects: ts.chart.objects.length, chart_equal: chartEqual, osu_equal: osuDifference === null, receipt_equal: receiptDifference === null && fields,
        first_osu_difference: osuDifference, first_receipt_difference: receiptDifference,
        runtime_names: [tsDetail.runtime.name, wasmDetail.runtime.name],
        ts: { wall_seconds: tsWall, seconds: tsDetail.seconds }, wasm: { wall_seconds: wasmWall, seconds: wasmDetail.seconds },
    };
    entry.passed = entry.chart_equal && entry.osu_equal && entry.receipt_equal;
    console.log(label.song, label.name, entry.passed ? 'equal' : 'DIFFERENT', `${entry.objects} objects, ts ${tsWall.toFixed(1)} s, wasm ${wasmWall.toFixed(1)} s`);
    report.cases.push(entry);
    await save();
}

try {
    for (const song of songs) {
        const recording = await Recording.open(join(referenceDir, 'bulk', song));
        const audio = await recording.event('audio');
        const receipt = JSON.parse(await readFile(join(modelDir, 'examples', song, 'receipt.json'), 'utf8'));
        const pcm = { samples: audio.pcm, channels: recording.shape(recording.index.events.find(e => e.name === 'audio').arrays.pcm)[1] ?? 1, mel: audio.mel_pcm }, sampleRate = Number(audio.sample_rate[0]);
        await recording.close();
        const base = { star: receipt.star, title: receipt.metadata.title, artist: receipt.metadata.artist, audio: { asset_id: song } };
        for (const seed of seeds)
            await compare({ song, name: `seed ${seed}`, seed }, pcm, sampleRate, { ...base, seed });
        if (song === 'hesitation-snow') {
            const controls = { long_notes: 2, sections: [{ start_ms: 20000, end_ms: 40000, star: 4.5 }, { start_ms: 60000, end_ms: 80000, pattern: { kind: 2, strength: 0.7 } }, { start_ms: 100000, end_ms: 120000, pattern: { kind: 5, strength: 0.4 } }] };
            await compare({ song, name: 'controls', seed: 1 }, pcm, sampleRate, { ...base, seed: 1, controls });
        }
    }
    report.passed = report.capabilities_equal && report.cases.every(c => c.passed);
    if (!report.passed)
        process.exitCode = 1;
    const mean = (xs) => xs.reduce((a, b) => a + b, 0) / xs.length;
    report.mean_wall_seconds = { ts: mean(report.cases.map(c => c.ts.wall_seconds)), wasm: mean(report.cases.map(c => c.wasm.wall_seconds)) };
}
catch (error) {
    report.failure = String(error);
    process.exitCode = 1;
    console.error(error);
}
finally {
    await tsModels.dispose();
    await wasmModels.dispose();
    await save();
}
