import { callCore, coreInstance } from './core.js';
import { Prepared, chartResult, coreAudio, coreSeed, runGraph } from './generate.js';

/**
 * Generation for play, from a song's PCM or its `Prepared`. The caller reports play (`start`, `heard`, `stop`) and
 * runs `work()`, which runs the graphs, hands each committed page to `options.onPage`, and resolves when play may
 * start (`{kind: 'ready', joinMs}`) or nothing is left to do (`{kind: 'done', result}`, with the chart once
 * generation has finished). While play is far enough ahead it waits for the next `heard`, `start` or `stop`.
 */
export class ChartScheduler {
    #models;
    #core;
    #instance;
    #seed;
    #options;
    #wake;

    constructor(models, core, seed, options) {
        this.#models = models;
        this.#core = core;
        this.#instance = coreInstance();
        this.#seed = seed;
        this.#options = options;
    }

    /**
     * `source` is a `Prepared` or `{pcm, sampleRate}` as for `generateChart`. Options: `mode` ('ahead-of-play', the
     * default, or 'whole-chart'), `judge` ('A' to 'E', default 'C': the rules whose settle lead the lead takes),
     * `renderLeadMs` (how far ahead of the heard time the playfield draws notes) and `marginMs` (room for the
     * variance of the generation's speed), `onPage(page)`, `setting`, `audio`, `clock`, `replay` as for `generateChart`.
     */
    static async create(models, source, request, options = {}) {
        const { seed, json } = coreSeed(request);
        await models.reloadCore();
        const prepared = source instanceof Prepared ? source.bytes() : undefined;
        const audio = prepared ? {} : await coreAudio(models, source.pcm, source.sampleRate);
        const clock = options.clock ?? (() => performance.now());
        const core = callCore(() => new models.core.WasmScheduler(models.wasm, prepared, audio.samples, audio.channels ?? 1, source.sampleRate ?? 0, audio.mel, audio.beatthis, audio.resampler, json, BigInt(seed), typeof seed === 'bigint', options.setting, options.audio?.name, clock, options.replay, options.mode ?? 'ahead-of-play', options.judge ?? 'C', options.renderLeadMs ?? 0, options.marginMs ?? 0));
        return new ChartScheduler(models, core, seed, options);
    }

    /** Play is to start at `atMs`, 0 or a position; the pages that follow are for a new session, from the join point. */
    start(atMs) {
        callCore(() => this.#core.start(atMs));
        this.#wake?.();
    }

    /** Each frame during play: the chart time heard and the clock that `performance.now` reads, in milliseconds. */
    heard(chartMs, wallMs) {
        callCore(() => this.#core.heard(chartMs, wallMs));
        this.#wake?.();
    }

    stop() {
        callCore(() => this.#core.stop());
        this.#wake?.();
    }

    async work() {
        for (;;) {
            const action = callCore(() => this.#core.next());
            if (action.kind === 'run') {
                const outputs = await runGraph(this.#models, this.#core, action);
                callCore(() => this.#core.outputs(outputs));
            }
            else if (action.kind === 'commit')
                this.#options.onPage?.(action.page);
            else if (action.kind === 'ready')
                return { kind: 'ready', joinMs: action.join_ms };
            else if (action.kind === 'wait') {
                await new Promise(resolve => { this.#wake = resolve; });
                this.#wake = undefined;
            }
            else
                return { kind: 'done', ...(action.chart ? { result: chartResult(action, this.#seed, this.#options) } : {}) };
        }
    }

    /** Committed chart milliseconds per wall millisecond of work so far. */
    get rate() {
        return callCore(() => this.#core.rate());
    }

    /** The song's `Prepared`, to store, once the audio stages have run. */
    prepared() {
        const bytes = callCore(() => this.#core.prepared());
        return bytes ? new Prepared(bytes) : undefined;
    }

    free() {
        if (coreInstance() === this.#instance)
            this.#core.free();
    }
}
