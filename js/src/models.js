import { callCore, coreInstance, loadCore, readFile } from './core.js';

/** The package's manifest, at its root beside `runtime.json` (docs/model-package.md). */
const PACKAGE = 'ensomi-package.json';

/** What a source rejects with for a file it does not have: an error with `code` 'ENOENT', as Node's `readFile` gives. */
function missingFile(path) {
    return Object.assign(new Error(`Missing model file: ${path}`), { code: 'ENOENT' });
}

/** Reads files relative to a base URL: with `fetch`, or from the file system for a `file:` URL in Node. */
export function urlSource(baseUrl) {
    const base = baseUrl.replace(/\/$/, '') + '/';
    return async (path) => {
        const url = new URL(path, base);
        if (url.protocol === 'file:')
            return readFile(url);
        const response = await fetch(url);
        if (response.status === 404)
            throw missingFile(path);
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
            throw missingFile(path);
        return new Uint8Array(await file.arrayBuffer());
    };
}

/** Checks an `ensomi-package.json` and resolves to what it describes; rejects naming what is wrong. */
export async function checkPackage(manifestJson) {
    return (await loadCore()).checkPackage(manifestJson);
}

/** Reads the files a checked manifest lists, each checked against its listed size and SHA-256; any other path is
 * refused. */
function checkedSource(read, info) {
    const listed = new Map(info.files.map(file => [file.path, file]));
    return async (path) => {
        const file = listed.get(path);
        if (!file)
            throw new Error(`${path} is not among the files ${PACKAGE} lists`);
        const bytes = await read(path);
        const sha256 = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), b => b.toString(16).padStart(2, '0')).join('');
        if (bytes.length !== file.size || sha256 !== file.sha256)
            throw new Error(`${path} does not match ${PACKAGE}: ${bytes.length} bytes with SHA-256 ${sha256}, where it lists ${file.size} bytes with ${file.sha256}`);
        return bytes;
    };
}

/** `promise`, its rejection marked as handled: it is awaited later, and a failure that comes first must not also be
 * reported as unhandled. */
function handled(promise) {
    promise.catch(() => undefined);
    return promise;
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

/** The licence notices a `runtime.json` names, as paths relative to the model directory: files to download and show
 * with the model, which `loadModels` reads into `models.notices` but does not need. */
export async function noticeFiles(runtimeConfigJson) {
    return (await loadCore()).WasmModels.noticeFiles(runtimeConfigJson);
}

export class Models {
    constructor(core, wasm, ort, sessions, source, runtimeSource, manifestJson, notices, info) {
        this.core = core;
        this.wasm = wasm;
        this.ort = ort;
        this.sessions = sessions;
        this.source = source;
        this.runtimeSource = runtimeSource;
        this.manifestJson = manifestJson;
        this.notices = notices;
        this.package = info;
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
 * Loads the six graphs into ONNX Runtime Web sessions and the rest of the model package into the core, and the
 * licence notices into `models.notices`. With `ensomi-package.json`, checks it, fetches the files it lists, each checked
 * against its SHA-256, and reads no other; without it (the source rejects with `code` 'ENOENT'), reads `runtime.json`
 * and the files it names.
 * `source` is a base URL or an async function `(relativePath) => Uint8Array`.
 * Options: `ort` (the onnxruntime-web module), `session` (SessionOptions), `backend` ('webgpu' | 'wasm'),
 * `wasm` (ORT's `env.wasm` flags), `runtimeSource` (reads the `resample-*` tables), `core` (the Wasm core's
 * bytes, module or URL).
 */
export async function loadModels(source, options = {}) {
    const fetchFile = typeof source === 'string' ? urlSource(source) : source;
    const core = await loadCore(options.core);
    const info = await fetchFile(PACKAGE).then(bytes => core.checkPackage(new TextDecoder().decode(bytes)), error => {
        if (error?.code !== 'ENOENT')
            throw error;
    });
    // With a manifest, every file it lists is fetched at once and checked; the loading below reads those.
    const checked = info ? checkedSource(fetchFile, info) : fetchFile;
    const fetched = new Map(info?.files.map(file => [file.path, handled(checked(file.path))]));
    const read = path => fetched.get(path) ?? checked(path);
    const manifestBytes = await read(info?.config ?? 'runtime.json');
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
    // The licence notices are shown with the model, not needed to run it. A manifest's notices are among its files;
    // without one, a notice the source lacks (a folder installed without it) is left out.
    const notice = (path, bytes) => ({ path, text: new TextDecoder().decode(bytes) });
    const notices = handled(info
        ? Promise.all(info.notices.map(path => read(path).then(bytes => notice(path, bytes))))
        : Promise.all(core.WasmModels.noticeFiles(manifestJson).map(path => read(path).then(bytes => notice(path, bytes), () => undefined))).then(list => list.filter(Boolean)));
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
            await Promise.all([create, ...reads, ...fetched.values()]);
        }
        catch (error) {
            wasm.free();
            throw error;
        }
        return new Models(core, wasm, ort, sessions, checked, options.runtimeSource ?? defaultRuntimeSource, manifestJson, await notices, info);
    }
    catch (error) {
        await release();
        throw error;
    }
}
