import { callCore, coreInstance, loadCore } from './core.js';
import { exportOsz } from './export.js';

/** Lets queued tasks run, such as the WebGPU result that a graph run is waiting for. */
const pause = () => new Promise(resolve => {
    const channel = new MessageChannel();
    channel.port1.onmessage = () => {
        channel.port1.close();
        resolve();
    };
    channel.port2.postMessage(null);
});

/**
 * Runs the graph a core step asked for and returns its outputs for the core. While the graph runs (BeatThis on the
 * GPU), `machine.work()` does what needs no outputs, yielding between slices so that the outputs are taken as soon
 * as they arrive.
 */
export async function runGraph(models, machine, step) {
    const feeds = {};
    for (const [name, tensor] of Object.entries(step.inputs))
        feeds[name] = new models.ort.Tensor(tensor.type, tensor.data, tensor.dims);
    let running = true;
    const run = models.sessions[step.graph].run(feeds);
    run.then(() => { running = false; }, () => { running = false; });
    while (running && callCore(() => machine.work()))
        await pause();
    const result = await run;
    const outputs = {};
    for (const [name, tensor] of Object.entries(result))
        outputs[name] = { type: tensor.type, dims: Array.from(tensor.dims), data: tensor.data };
    return outputs;
}

/** PCM as the core takes it, and the resampler tables it needs. */
export async function coreAudio(models, pcm, sampleRate) {
    const audio = pcm instanceof Float32Array || pcm instanceof Float64Array ? { samples: pcm, channels: 1 } : pcm;
    const resamplerFiles = audio.beatthis ? null : models.core.WasmModels.resamplerFiles(sampleRate);
    const resampler = resamplerFiles ? await Promise.all(Array.from(resamplerFiles, name => models.runtimeSource(name))) : undefined;
    // decodeAudio passes its Mel stream as the samples too; the core then takes the stream once.
    const samples = audio.mel && audio.beatthis && audio.samples === audio.mel ? new Float32Array(0) : audio.samples;
    return { samples, channels: audio.channels, mel: audio.mel, beatthis: audio.beatthis, resampler };
}

/** The request's seed as the core takes it: a BigInt, and whether the caller gave one. */
export function coreSeed(request) {
    const seed = request.seed ?? 0;
    if (typeof seed !== 'number' && typeof seed !== 'bigint' || typeof seed === 'number' && (!Number.isSafeInteger(seed) || seed < 0) || typeof seed === 'bigint' && (seed < 0n || seed > 0xffffffffffffffffn))
        throw new Error('Seed must be an unsigned 64-bit integer; use bigint above the safe-number range');
    const { seed: _seed, ...rest } = request;
    return { seed, json: JSON.stringify(rest) };
}

/**
 * What the stages that read only the audio keep of a song, from which any request and seed generate
 * (`ChartScheduler.create`). `bytes()` is the runtime's stored form, for a cache keyed by the audio file's hash.
 */
export class Prepared {
    #bytes;
    constructor(bytes) {
        this.#bytes = bytes;
    }
    bytes() {
        return this.#bytes;
    }
    /** Reads stored bytes; throws for bytes of another format, runtime version or model. */
    static async fromBytes(bytes) {
        const core = await loadCore();
        callCore(() => core.checkPrepared(bytes));
        return new Prepared(bytes);
    }
}

/**
 * Runs the stages that read only the audio: PCM preparation, the spectrogram, BeatThis, the features and the beat
 * grid. `pcm` and `sampleRate` as for `generateChart`. Options: `onProgress({phase})`, `clock`.
 */
export async function prepare(models, pcm, sampleRate, options = {}) {
    await models.reloadCore();
    const audio = await coreAudio(models, pcm, sampleRate);
    const clock = options.clock ?? (() => performance.now());
    const machine = callCore(() => new models.core.WasmPrepare(models.wasm, audio.samples, audio.channels, sampleRate, audio.mel, audio.beatthis, audio.resampler, clock));
    const instance = coreInstance();
    try {
        let outputs;
        for (;;) {
            const step = callCore(() => machine.step(outputs));
            outputs = undefined;
            if (step.kind === 'progress')
                options.onProgress?.({ phase: step.phase });
            else if (step.kind === 'run')
                outputs = await runGraph(models, machine, step);
            else
                return new Prepared(step.bytes);
        }
    }
    finally {
        if (coreInstance() === instance)
            machine.free();
    }
}

/**
 * Generates a chart. `pcm` is mono Float32Array/Float64Array or `{samples, channels, mel?, beatthis?}` with
 * interleaved samples at `sampleRate`; `request` is a GenerateChartRequest.
 * Options: `onProgress({phase})`, `audio` ({name, bytes}, adds an .osz), `setting` (difficulty name), `clock`
 * (milliseconds, default `performance.now`), `replay` (a core `ReplayDraws`, moved into the generation, for
 * conformance checks; the result then has `replayConsumed`, the number of draws used).
 */
export async function generateChart(models, pcm, sampleRate, request, options = {}) {
    if (options.random || options.trace)
        throw new Error('The Wasm core takes recorded draws as options.replay and has no trace');
    const { seed, json } = coreSeed(request);
    await models.reloadCore();
    const audio = await coreAudio(models, pcm, sampleRate);
    const clock = options.clock ?? (() => performance.now());
    const generator = callCore(() => new models.core.WasmGenerator(models.wasm, audio.samples, audio.channels, sampleRate, audio.mel, audio.beatthis, audio.resampler, json, BigInt(seed), typeof seed === 'bigint', options.setting, options.audio?.name, clock, options.replay));
    const instance = coreInstance();
    try {
        let outputs;
        for (;;) {
            const step = callCore(() => generator.step(outputs));
            outputs = undefined;
            if (step.kind === 'progress')
                options.onProgress?.({ phase: step.phase });
            else if (step.kind === 'run')
                outputs = await runGraph(models, generator, step);
            else
                return chartResult(step, seed, options);
        }
    }
    finally {
        if (coreInstance() === instance)
            generator.free();
    }
}

/** The result of a finished core generation, with the `.osz` when `options.audio` is given. */
export function chartResult(step, seed, options) {
    const { chart, osu, receipt, replay_consumed: replayConsumed } = step;
    const osz = options.audio ? exportOsz(osu, options.audio, receipt.receipt_json) : undefined;
    return { chart, receipt: { model_name: receipt.model_name, model_version: receipt.model_version, seed, star: receipt.star, receipt_json: receipt.receipt_json }, osu, ...(osz ? { osz } : {}), ...(options.replay ? { replayConsumed } : {}) };
}
