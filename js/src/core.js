import init, * as bindings from '../pkg/ensomi_generate_wasm.js';

let loaded;

/**
 * Instantiates the Wasm core once. `wasm` is the module's bytes, a `WebAssembly.Module` or a URL; the
 * default is the `.wasm` file beside the bindings, which `fetch` can read in a browser but not in Node.
 */
export function loadCore(wasm) {
    loaded ??= init({ module_or_path: wasm ?? new URL('../pkg/ensomi_generate_wasm_bg.wasm', import.meta.url) }).then(() => bindings);
    return loaded;
}
