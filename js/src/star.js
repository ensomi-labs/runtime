import { loadCore, callCore } from './core.js';

/** osu!mania star rating (20241007), four keys at clock rate 1; also works for imported charts.
 * Loads the package's Wasm core if no `loadModels` has. */
export async function starRating(chart) {
    const core = await loadCore();
    return callCore(() => core.starRating(JSON.stringify(chart)));
}
