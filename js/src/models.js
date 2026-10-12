import { callCore, coreInstance, loadCore, readFile } from './core.js';

/** Reads files relative to a base URL: with `fetch`, or from the file system for a `file:` URL in Node. */
export function urlSource(baseUrl) {
    const base = baseUrl.replace(/\/$/, '') + '/';
    return async (path) => {
        const url = new URL(path, base);
        if (url.protocol === 'file:')
            return readFile(url);
        const response = await fetch(url);
        if (!response.ok)
            throw new Error(`Model download failed: ${path} (${response.status})`);
        return new Uint8Array(await response.arrayBuffer());
    };
}

export function localFilesSource(files) {
    const entries = new Map(Array.from(files, f => [f.name, f]));
    return async (path) => {
        const file = entries.get(path) ?? entries.get(path.split('/').pop());
        if (!file)
            throw new Error(`Missing model file: ${path}`);
        return new Uint8Array(await file.arrayBuffer());
    };
}

/** Graphs that use WebGPU when it is available. The other graphs run once per beat or head row on small
 * inputs, where WebGPU's per-call transfers cost more than computing on the CPU. */
const GPU_GRAPHS = new Set(['beatthis']);

/** The model in the core: `runtime.json` and the files it names besides the graphs. */
async function coreModels(core, manifestJson, read) {
    const paths = core.WasmModels.requiredFiles(manifestJson);
    const bytes = await Promise.all(paths.map(path => read(path)));
    return callCore(() => new core.WasmModels(manifestJson, Object.fromEntries(paths.map((path, i) => [path, bytes[i]]))));
}

export class Models {
    constructor(core, wasm, ort, sessions, source, runtimeSource, manifestJson) {
        this.core = core;
        this.wasm = wasm;
        this.ort = ort;
        this.sessions = sessions;
        this.source = source;
        this.runtimeSource = runtimeSource;
        this.manifestJson = manifestJson;
        this.instance = coreInstance();
        this.capabilities = JSON.parse(wasm.capabilities());
    }
    /** Loads the model into the core again if a trap replaced the core's instance since (see `callCore`). */
    async reloadCore() {
        if (this.instance === coreInstance())
            return;
        this.wasm = await coreModels(this.core, this.manifestJson, this.source);
        this.instance = coreInstance();
    }
    async dispose() {
        for (const session of Object.values(this.sessions))
            await session.release();
        if (this.instance === coreInstance())
            this.wasm.free();
    }
}

/** Where the soxr tables for raw PCM at other rates than 22050 Hz are read from: the package's `resampler/`. */
function defaultRuntimeSource(path) {
    return urlSource(new URL('../resampler/', import.meta.url).href)(path);
}

/**
 * Loads the six graphs into ONNX Runtime Web sessions and the rest of the model package into the core.
 * `source` is a base URL or an async function `(relativePath) => Uint8Array`.
 * Options: `ort` (the onnxruntime-web module), `session` (SessionOptions), `backend` ('webgpu' | 'wasm'),
 * `wasm` (ORT's `env.wasm` flags), `runtimeSource` (reads the `resample-*` tables), `core` (the Wasm core's
 * bytes, module or URL).
 */
export async function loadModels(source, options = {}) {
    const read = typeof source === 'string' ? urlSource(source) : source;
    const core = await loadCore(options.core);
    const manifestBytes = await read('runtime.json');
    const manifestJson = new TextDecoder().decode(manifestBytes);
    const manifest = JSON.parse(manifestJson);
    if (manifest.format !== 'ensomi-onnx-runtime-v1')
        throw new Error(`Unsupported runtime format: ${manifest.format}`);
    const backend = options.backend ?? (typeof navigator !== 'undefined' && 'gpu' in navigator ? 'webgpu' : 'wasm');
    const ort = options.ort ?? (backend === 'webgpu' ? await import('onnxruntime-web/webgpu') : await import('onnxruntime-web/wasm'));
    if (options.wasm)
        Object.assign(ort.env.wasm, options.wasm);
    const sessions = {};
    // All files download at once; sessions are created in order, each as soon as its file has arrived.
    const graphs = Object.entries(manifest.graphs), reads = graphs.map(([, graph]) => read(graph.file));
    const create = (async () => {
        for (const [i, [name]] of graphs.entries())
            sessions[name] = await ort.InferenceSession.create(await reads[i], options.session ?? { executionProviders: backend === 'webgpu' && GPU_GRAPHS.has(name) ? ['webgpu', 'wasm'] : ['wasm'] });
    })();
    const release = async () => {
        await create.catch(() => undefined);
        for (const session of Object.values(sessions))
            await session.release();
    };
    try {
        const wasm = await coreModels(core, manifestJson, read);
        try {
            await Promise.all([create, ...reads]);
        }
        catch (error) {
            wasm.free();
            throw error;
        }
        return new Models(core, wasm, ort, sessions, read, options.runtimeSource ?? defaultRuntimeSource, manifestJson);
    }
    catch (error) {
        await release();
        throw error;
    }
}
