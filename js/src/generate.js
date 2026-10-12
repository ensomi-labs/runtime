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
 * Generates a chart. `pcm` is mono Float32Array/Float64Array or `{samples, channels, mel?, beatthis?}` with
 * interleaved samples at `sampleRate`; `request` is a GenerateChartRequest.
 * Options: `onProgress({phase})`, `audio` ({name, bytes}, adds an .osz), `setting` (difficulty name), `clock`
 * (milliseconds, default `performance.now`), `replay` (a core `ReplayDraws`, moved into the generation, for
 * conformance checks; the result then has `replayConsumed`, the number of draws used).
 */
export async function generateChart(models, pcm, sampleRate, request, options = {}) {
    if (options.random || options.trace)
        throw new Error('The Wasm core takes recorded draws as options.replay and has no trace');
    const audio = pcm instanceof Float32Array || pcm instanceof Float64Array ? { samples: pcm, channels: 1 } : pcm;
    const seed = request.seed ?? 0;
    if (typeof seed !== 'number' && typeof seed !== 'bigint' || typeof seed === 'number' && (!Number.isSafeInteger(seed) || seed < 0) || typeof seed === 'bigint' && (seed < 0n || seed > 0xffffffffffffffffn))
        throw new Error('Seed must be an unsigned 64-bit integer; use bigint above the safe-number range');
    const { core, wasm } = models;
    const resamplerFiles = audio.beatthis ? null : core.WasmModels.resamplerFiles(sampleRate);
    const resampler = resamplerFiles ? await Promise.all(Array.from(resamplerFiles, name => models.runtimeSource(name))) : undefined;
    const { seed: _seed, ...rest } = request;
    const clock = options.clock ?? (() => performance.now());
    const generator = new core.WasmGenerator(wasm, audio.samples, audio.channels, sampleRate, audio.mel, audio.beatthis, resampler, JSON.stringify(rest), BigInt(seed), typeof seed === 'bigint', options.setting, options.audio?.name, clock, options.replay);
    try {
        let outputs;
        for (;;) {
            const step = generator.step(outputs);
            outputs = undefined;
            if (step.kind === 'progress') {
                options.onProgress?.({ phase: step.phase });
            }
            else if (step.kind === 'run') {
                const feeds = {};
                for (const [name, tensor] of Object.entries(step.inputs))
                    feeds[name] = new models.ort.Tensor(tensor.type, tensor.data, tensor.dims);
                // While the graph runs (BeatThis on the GPU), the core does what needs no outputs, yielding between
                // slices so that the outputs are taken as soon as they arrive.
                let running = true;
                const run = models.sessions[step.graph].run(feeds);
                run.then(() => { running = false; }, () => { running = false; });
                while (running && generator.work())
                    await pause();
                const result = await run;
                outputs = {};
                for (const [name, tensor] of Object.entries(result))
                    outputs[name] = { type: tensor.type, dims: Array.from(tensor.dims), data: tensor.data };
            }
            else {
                const { chart, osu, receipt, replay_consumed: replayConsumed } = step;
                const osz = options.audio ? exportOsz(osu, options.audio, receipt.receipt_json) : undefined;
                return { chart, receipt: { model_name: receipt.model_name, model_version: receipt.model_version, seed, star: receipt.star, receipt_json: receipt.receipt_json }, osu, ...(osz ? { osz } : {}), ...(options.replay ? { replayConsumed } : {}) };
            }
        }
    }
    finally {
        generator.free();
    }
}
