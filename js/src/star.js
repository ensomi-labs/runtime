import { loadCoreSync, callCore, readFile } from './core.js';

// Compile during import so calculation is synchronous, while loadModels may still initialize a custom core first.
const url = new URL('../pkg/ensomi_generate_wasm_bg.wasm', import.meta.url);
const module = await WebAssembly.compile(await (url.protocol === 'file:' ? readFile(url) : fetch(url).then(r => r.arrayBuffer())));

/** osu!mania star rating (20241007), four keys at clock rate 1; also works for imported charts. */
export function starRating(chart) {
    const core = loadCoreSync(module);
    return callCore(() => core.starRating(JSON.stringify(chart)));
}
