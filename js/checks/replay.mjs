// Replays the four recorded release songs through the Wasm core and ONNX Runtime Web's WASM backend in Node,
// with the criteria of the native check (crates/generate-ort/src/bin/parity.rs):
// every graph call within tolerance, identical sampled choices and hit objects, timing points within 1e-6 ms,
// all other .osu text identical.
//
// With `--check off`, the chart comes from the JavaScript ChartScheduler playing from 0, a page per head row with
// R2's difficulty check off: the draws used must be the release's up to pass 1, the pages must make the chart, and
// the chart must be the release's where only one pass ran, else the one pass 1's recorded decisions make. The graphs
// are not replayed again.
//
// Usage: node replay.mjs --model-dir <ensomi-r2> --reference <runtime-ref> --assets <resampler tables> --core <wasm> --out <report.json>
//        [--check off] [song...]
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import * as ort from 'onnxruntime-web/wasm';
import { loadModels, generateChart, ChartScheduler } from '../src/index.js';
import { loadCore } from '../src/core.js';
import { Recording, chartFromDecisions, difference, firstTextDifference, pagesMakeChart, parseOsu, stripGenerator, timingDifference, withoutTimingPoints } from './recording.mjs';

const args = process.argv.slice(2), option = (name) => { const i = args.indexOf(name); return i < 0 ? undefined : args.splice(i, 2)[1]; };
const modelDir = option('--model-dir'), referenceDir = option('--reference'), assetsDir = option('--assets'), corePath = option('--core'), out = option('--out');
const checkOff = option('--check') === 'off';
const songs = args.length ? args : ['hesitation-snow', 'saigetsu', 'catch-the-moment', 'adventure'];
const GRAPH_TOLERANCE = 1e-4, BEATTHIS_TOLERANCE = 2e-4, GRID_TOLERANCE_MS = 1e-4, TIMING_TOLERANCE_MS = 1e-6;
const report = { ort: ort.env.versions?.common ?? '1.23.2', provider: 'wasm', threads: 1, graph_tolerance: { default: GRAPH_TOLERANCE, beatthis: BEATTHIS_TOLERANCE }, timing_tolerance_ms: TIMING_TOLERANCE_MS, scope: 'recorded graph inputs, PCM frontends, full Wasm decode with carried state and replayed random draws, exact exports, own PRNG', check: checkOff ? 'off: ChartScheduler from 0, one pass committed per head row; graphs not replayed again' : 'on', songs: {}, passed: false };

/** A chart through the scheduler, playing from 0 with nothing heard, so that it generates to the end; and its pages. */
async function streamed(source, request, options) {
    const pages = [];
    const scheduler = await ChartScheduler.create(models, source, request, { ...options, onPage: page => pages.push(page) });
    try {
        scheduler.start(0);
        const ready = await scheduler.work(), done = await scheduler.work();
        if (ready.kind !== 'ready' || ready.joinMs !== 0 || done.kind !== 'done')
            throw new Error(`scheduler: ${JSON.stringify([ready, done.kind])}`);
        return { result: done.result, pages, prepared: scheduler.prepared() };
    }
    finally {
        scheduler.free();
    }
}
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
            if (!checkOff) {
                row.stage1 = { passed: true, networks: await networkReplay(recording) };
                console.log(song, 'stage 1', Object.fromEntries(Object.entries(row.stage1.networks).map(([name, v]) => [name, v.max_abs])));
                await save();
            }
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
            const options = { replay, setting: receipt.setting, audio: { name: audioName, bytes: new Uint8Array(0) } };
            let result, prepared;
            if (checkOff) {
                const run = await streamed({ pcm, sampleRate }, request, options);
                ({ result, prepared } = run);
                row.pages = pagesMakeChart(run.pages, result.chart, expectedRows.head_ms.length);
            }
            else
                result = await generateChart(models, pcm, sampleRate, request, { ...options, onProgress: p => phases.push(p.phase) });
            // With the check off, pass 2's draws are left.
            const pass2 = recording.index.draws.filter(d => d.name.startsWith('r2_pass_2/')).length;
            const expectedDraws = checkOff ? recorded - pass2 : recorded;
            if (result.replayConsumed !== expectedDraws)
                throw new Error(`Unconsumed draws: ${result.replayConsumed}/${expectedDraws}`);
            const detail = JSON.parse(result.receipt.receipt_json);
            const grid = detail.rows.segments;
            const gridError = Math.max(...grid.map((s, i) => Math.abs(s[0] - Number(expectedRows.grid_offsets[i])))), sameSegments = grid.length === expectedRows.grid_offsets.length;
            row.stage2 = { passed: sameSegments && gridError <= GRID_TOLERANCE_MS, max_offset_error_ms: gridError, segments: grid };
            row.stage3 = { passed: detail.rows.K === expectedRows.head_ms.length, heads: detail.rows.K, recorded_heads: expectedRows.head_ms.length };
            const priorError = Math.max(...['held', 'heads', 'busy_free', 'quad', 'log_r'].map(k => Math.abs(detail.prior.predicted[k] - Number(expectedPrior[k][0]))));
            row.stage4 = { passed: priorError === 0, prior_max_error: priorError, draws_consumed: result.replayConsumed, draws_recorded: recorded, passes: detail.controls.difficulty.passes };
            if (checkOff) {
                // One pass, and the check would have arranged again exactly where the release did.
                const wouldRepass = detail.controls.difficulty.would_repass;
                Object.assign(row.stage4, { would_repass: wouldRepass, release_passes: pass2 > 0 ? 2 : 1 });
                row.stage4.passed &&= detail.controls.difficulty.passes.length === 1 && wouldRepass === pass2 > 0;
            }
            row.replay_seconds = detail.seconds;
            row.phases = phases;
            const expectedObjects = parseOsu(expectedOsu);
            if (checkOff && pass2 > 0) {
                // The release's chart is pass 2's: the chart must be the one pass 1's recorded decisions make.
                const actions = [], gaps = [];
                for (let k = 0; k <= expectedRows.head_ms.length; k++) {
                    const chosen = await recording.event(`r2_pass_1/${k}/chosen`);
                    actions.push(Array.from(chosen.codes, Number));
                    gaps.push(Array.from(chosen.releases, Number));
                }
                const songMs = 1000 * audio.pcm.length / pcm.channels / sampleRate;
                const pass1 = chartFromDecisions(Array.from(expectedRows.head_ms, Number), songMs, actions, gaps);
                const same = JSON.stringify(result.chart.objects) === JSON.stringify(pass1);
                row.stage5 = { passed: same, compared: 'with the chart of the recorded pass-1 decisions', objects: result.chart.objects.length, same_as_release: JSON.stringify(result.chart.objects) === JSON.stringify(expectedObjects) };
                if (!same)
                    throw new Error(JSON.stringify({ stage: 5, song, step: 'pass-1 hitobjects', objects: result.chart.objects.length, expected: pass1.length }));
            }
            else if (JSON.stringify(result.chart.objects) !== JSON.stringify(expectedObjects)) {
                const index = result.chart.objects.findIndex((o, i) => JSON.stringify(o) !== JSON.stringify(expectedObjects[i]));
                throw new Error(JSON.stringify({ stage: 5, song, step: 'hitobjects', index, actual: result.chart.objects[index], reference: expectedObjects[index] }));
            }
            else {
                const textDifference = firstTextDifference(stripGenerator(result.osu), stripGenerator(expectedOsu));
                const timing = timingDifference(result.osu, expectedOsu), restDifference = firstTextDifference(stripGenerator(withoutTimingPoints(result.osu)), stripGenerator(withoutTimingPoints(expectedOsu)));
                row.stage5 = { passed: timing.same_rows && timing.max_abs_ms <= TIMING_TOLERANCE_MS && restDifference === null, hitobjects_identical: true, objects: result.chart.objects.length, timing_points: timing.rows, timing_max_abs_ms: timing.max_abs_ms, rest_identical: restDifference === null, osu_identical_except_generator: textDifference === null, first_text_difference: textDifference, osz_bytes: result.osz.length };
            }
            console.log(song, 'replay', row.stage5);
            // The generator's own PRNG, as it runs in production; with the check off, from the stored audio stages.
            console.log(`${song}: own PRNG run`);
            const wall = performance.now();
            let own, ownPages;
            if (checkOff) {
                const run = await streamed(prepared, request, { setting: receipt.setting });
                own = run.result;
                ownPages = pagesMakeChart(run.pages, own.chart, JSON.parse(own.receipt.receipt_json).rows.K);
            }
            else
                own = await generateChart(models, pcm, sampleRate, request, { setting: receipt.setting });
            const ownDetail = JSON.parse(own.receipt.receipt_json);
            if (!ownDetail.export.replay_finalized)
                throw new Error(`Own PRNG left an open hold: ${song}`);
            row.prng = { passed: ownPages?.passed ?? true, algorithm: ownDetail.runtime.random_algorithm, seed: request.seed, objects: own.chart.objects.length, heads: ownDetail.rows.K, fallbacks: ownDetail.fallback_count, seconds: ownDetail.seconds, wall_seconds: (performance.now() - wall) / 1000, ...(checkOff ? { pages: ownPages, would_repass: ownDetail.controls.difficulty.would_repass } : {}) };
        }
        finally {
            await recording.close();
            await save();
        }
    }
    const stages = checkOff ? [2, 3, 4, 5] : [1, 2, 3, 4, 5];
    report.passed = songs.every(song => stages.every(n => report.songs[song][`stage${n}`]?.passed) && report.songs[song].prng?.passed && (!checkOff || report.songs[song].pages?.passed));
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
