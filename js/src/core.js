import init, * as bindings from '../pkg/ensomi_generate_wasm.js';

let loaded, instance = 0;

/** Reads a `file:` URL in Node, whose `fetch` cannot read files. */
export async function readFile(url) {
    const bytes = await process.getBuiltinModule('node:fs/promises').readFile(url);
    return new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

/**
 * Instantiates the Wasm core once. `wasm` is the module's bytes, a `WebAssembly.Module` or a URL; the
 * default is the `.wasm` file beside the bindings, fetched in a browser and read from the file system in Node.
 */
export function loadCore(wasm) {
    const url = new URL('../pkg/ensomi_generate_wasm_bg.wasm', import.meta.url);
    loaded ??= init({ module_or_path: wasm ?? (url.protocol === 'file:' ? readFile(url) : url) }).then(() => bindings);
    return loaded;
}

/** Which instance of the core is current; a trap in `callCore` starts the next. */
export const coreInstance = () => instance;

/**
 * Calls into the core. A trap (a panic, whose message the error carries, or memory or stack exhausted) can leave
 * the instance's memory inconsistent, so the instance is replaced before the error is rethrown; objects of the
 * old instance cannot be used after that.
 */
export function callCore(call) {
    try {
        return call();
    }
    catch (error) {
        if (error instanceof WebAssembly.RuntimeError) {
            bindings.__wbg_reset_state();
            instance++;
        }
        throw error;
    }
}
