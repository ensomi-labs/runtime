// Replays the four recorded release songs through the Wasm core and ONNX Runtime Web's WASM backend in Node,
// with the criteria of the native check (crates/generate-ort/src/bin/parity.rs):
// every graph call within tolerance, identical sampled choices and hit objects, timing points within 1e-6 ms,
// all other .osu text identical.
//
// Usage: node replay.mjs --model-dir <ensomi-r2> --reference <runtime-ref> --assets <resampler tables> --core <wasm> --out <report.json> [song...]
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import * as ort from 'onnxruntime-web/wasm';
import { loadModels, generateChart } from '../src/index.js';
import { loadCore } from '../src/core.js';
import { Recording, difference, firstTextDifference, parseOsu, stripGenerator, timingDifference, withoutTimingPoints } from './recording.mjs';

const args = process.argv.slice(2), option = (name) => { const i = args.indexOf(name); return i < 0 ? undefined : args.splice(i, 2)[1]; };
const modelDir = option('--model-dir'), referenceDir = option('--reference'), assetsDir = option('--assets'), corePath = option('--core'), out = option('--out');
const songs = args.length ? args : ['hesitation-snow', 'saigetsu', 'catch-the-moment', 'adventure'];
const GRAPH_TOLERANCE = 1e-4, BEATTHIS_TOLERANCE = 2e-4, GRID_TOLERANCE_MS = 1e-4, TIMING_TOLERANCE_MS = 1e-6;
const report = { ort: ort.env.versions?.common ?? '1.23.2', provider: 'wasm', threads: 1, graph_tolerance: { default: GRAPH_TOLERANCE, beatthis: BEATTHIS_TOLERANCE }, timing_tolerance_ms: TIMING_TOLERANCE_MS, scope: 'recorded graph inputs, PCM frontends, full Wasm decode with carried state and replayed random draws, exact exports, own PRNG', songs: {}, passed: false };
const save = () => writeFile(out, JSON.stringify(report, null, 2) + '\n');

const core = await loadCore(await readFile(corePath));
const start = performance.now();
const models = await loadModels(async (p) => new Uint8Array(await readFile(join(modelDir, 'onnx', p))), { ort, core: await readFile(corePath), backend: 'wasm', wasm: { numThreads: 1 }, runtimeSource: async (p) => new Uint8Array(await readFile(join(assetsDir, p))) });
report.load_seconds = (performance.now() - start) / 1000;

async function networkReplay(recording) {
    const result = {};
    for (const call of recording.index.graphs) {
        const feeds = Object.fromEntries(await Promise.all(Object.entries(call.inputs).map(async ([name, id]) => [name, await recording.tensor(id, models.ort)])));
        const t = performance.now(), output = await models.sessions[call.name].run(feeds), seconds = (performance.now() - t) / 1000;
        const entry = result[call.name] ??= { calls: 0, seconds: 0, max_abs: 0, outputs: {} };
        entry.calls++;
        entry.seconds += seconds;
        for (const [key, value] of Object.entries(output)) {
            const expected = await recording.get(call.outputs[key]), diff = difference(value.data, expected), old = entry.outputs[key];
            if (!old || diff.max_abs > old.max_abs)
                entry.outputs[key] = { ...diff, scope: call.scope, step: call.step };
            entry.max_abs = Math.max(entry.max_abs, diff.max_abs);
            if (diff.max_abs > (call.name === 'beatthis' ? BEATTHIS_TOLERANCE : GRAPH_TOLERANCE) || !diff.nonfinite_equal || !diff.same_length)
                throw new Error(JSON.stringify({ stage: 1, graph: call.name, output: key, scope: call.scope, step: call.step, ...diff }));
        }
    }
    return result;
}

try {
    for (const song of songs) {
        const row = report.songs[song] = {};
        console.log(`${song}: stage 1 graph replay`);
        const recording = await Recording.open(join(referenceDir, 'bulk', song));
        try {
            row.stage1 = { passed: true, networks: await networkReplay(recording) };
            console.log(song, 'stage 1', Object.fromEntries(Object.entries(row.stage1.networks).map(([name, v]) => [name, v.max_abs])));
            await save();
            const audio = await recording.event('audio'), expectedRows = await recording.event('system/rows'), expectedPrior = await recording.event('system/prior');
            const receipt = JSON.parse(await readFile(join(modelDir, 'examples', song, 'receipt.json'), 'utf8')), expectedOsu = await readFile(join(modelDir, 'examples', song, 'generated.osu'), 'utf8'), summary = JSON.parse(await readFile(join(referenceDir, song, 'summary.json'), 'utf8'));
            const request = { star: receipt.star, seed: receipt.seed, title: receipt.metadata.title, artist: receipt.metadata.artist, audio: { asset_id: song } };
            const pcm = { samples: audio.pcm, channels: recording.shape(recording.index.events.find(e => e.name === 'audio').arrays.pcm)[1] ?? 1, mel: audio.mel_pcm }, sampleRate = Number(audio.sample_rate[0]);
            const audioName = summary.audio.split('/').pop();
            const replay = new core.ReplayDraws();
            for (const draw of recording.index.draws)
                replay.add(draw.name, draw.kind, Float64Array.from(await recording.get(draw.values), Number), draw.chosen);
            const recorded = replay.recorded();
            console.log(`${song}: stages 2-5, replaying draws`);
            const phases = [];
            const result = await generateChart(models, pcm, sampleRate, request, { replay, setting: receipt.setting, audio: { name: audioName, bytes: new Uint8Array(0) }, onProgress: p => phases.push(p.phase) });
            if (result.replayConsumed !== recorded)
                throw new Error(`Unconsumed draws: ${result.replayConsumed}/${recorded}`);
            const detail = JSON.parse(result.receipt.receipt_json);
            const grid = detail.rows.segments;
            const gridError = Math.max(...grid.map((s, i) => Math.abs(s[0] - Number(expectedRows.grid_offsets[i])))), sameSegments = grid.length === expectedRows.grid_offsets.length;
            row.stage2 = { passed: sameSegments && gridError <= GRID_TOLERANCE_MS, max_offset_error_ms: gridError, segments: grid };
            row.stage3 = { passed: detail.rows.K === expectedRows.head_ms.length, heads: detail.rows.K, recorded_heads: expectedRows.head_ms.length };
            const priorError = Math.max(...['held', 'heads', 'busy_free', 'quad', 'log_r'].map(k => Math.abs(detail.prior.predicted[k] - Number(expectedPrior[k][0]))));
            row.stage4 = { passed: priorError === 0, prior_max_error: priorError, draws_consumed: result.replayConsumed, draws_recorded: recorded, passes: detail.controls.difficulty.passes };
            row.replay_seconds = detail.seconds;
            row.phases = phases;
            const expectedObjects = parseOsu(expectedOsu);
            if (JSON.stringify(result.chart.objects) !== JSON.stringify(expectedObjects)) {
                const index = result.chart.objects.findIndex((o, i) => JSON.stringify(o) !== JSON.stringify(expectedObjects[i]));
                throw new Error(JSON.stringify({ stage: 5, song, step: 'hitobjects', index, actual: result.chart.objects[index], reference: expectedObjects[index] }));
            }
            const textDifference = firstTextDifference(stripGenerator(result.osu), stripGenerator(expectedOsu));
            const timing = timingDifference(result.osu, expectedOsu), restDifference = firstTextDifference(stripGenerator(withoutTimingPoints(result.osu)), stripGenerator(withoutTimingPoints(expectedOsu)));
            row.stage5 = { passed: timing.same_rows && timing.max_abs_ms <= TIMING_TOLERANCE_MS && restDifference === null, hitobjects_identical: true, objects: result.chart.objects.length, timing_points: timing.rows, timing_max_abs_ms: timing.max_abs_ms, rest_identical: restDifference === null, osu_identical_except_generator: textDifference === null, first_text_difference: textDifference, osz_bytes: result.osz.length };
            console.log(song, 'replay', row.stage5);
            console.log(`${song}: own PRNG run`);
            const wall = performance.now();
            const own = await generateChart(models, pcm, sampleRate, request, { setting: receipt.setting });
            const ownDetail = JSON.parse(own.receipt.receipt_json);
            if (!ownDetail.export.replay_finalized)
                throw new Error(`Own PRNG left an open hold: ${song}`);
            row.prng = { passed: true, algorithm: ownDetail.runtime.random_algorithm, seed: request.seed, objects: own.chart.objects.length, heads: ownDetail.rows.K, fallbacks: ownDetail.fallback_count, seconds: ownDetail.seconds, wall_seconds: (performance.now() - wall) / 1000 };
        }
        finally {
            await recording.close();
            await save();
        }
    }
    report.passed = songs.every(song => [1, 2, 3, 4, 5].every(n => report.songs[song][`stage${n}`]?.passed) && report.songs[song].prng?.passed);
    if (!report.passed)
        process.exitCode = 1;
}
catch (error) {
    report.failure = String(error);
    process.exitCode = 1;
    console.error(error);
}
finally {
    await models.dispose();
    await save();
}
