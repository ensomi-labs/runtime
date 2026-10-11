import { loadCore } from './core.js';

export function urlSource(baseUrl) {
    const base = baseUrl.replace(/\/$/, '') + '/';
    return async (path) => {
        const response = await fetch(new URL(path, base));
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

export class Models {
    constructor(core, wasm, ort, sessions, source, runtimeSource) {
        this.core = core;
        this.wasm = wasm;
        this.ort = ort;
        this.sessions = sessions;
        this.source = source;
        this.runtimeSource = runtimeSource;
        this.capabilities = JSON.parse(wasm.capabilities());
    }
    async dispose() {
        for (const session of Object.values(this.sessions))
            await session.release();
        this.wasm.free();
    }
}

/** Where the soxr tables for raw PCM at other rates than 22050 Hz are read from: `assets/` beside `src/`. */
function defaultRuntimeSource(path) {
    return urlSource(new URL('../assets/', import.meta.url).href)(path);
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
        const paths = core.WasmModels.requiredFiles(manifestJson);
        const bytes = await Promise.all(paths.map(path => read(path)));
        const wasm = new core.WasmModels(manifestJson, Object.fromEntries(paths.map((path, i) => [path, bytes[i]])));
        try {
            await Promise.all([create, ...reads]);
        }
        catch (error) {
            wasm.free();
            throw error;
        }
        return new Models(core, wasm, ort, sessions, read, options.runtimeSource ?? defaultRuntimeSource);
    }
    catch (error) {
        await release();
        throw error;
    }
}
