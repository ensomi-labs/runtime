import { loadCore, callCore } from './core.js';

// Initialize during module import so the public calculator is synchronous.
const core = await loadCore();

/** osu!mania star rating (20241007), four keys at clock rate 1; also works for imported charts. */
export function starRating(chart) {
    return callCore(() => core.starRating(JSON.stringify(chart)));
}
