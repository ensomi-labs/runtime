// Shared Python vectors, through the public synchronous wrapper and its Wasm binding. No model needed.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { starRating } from '../src/index.js';
import { parseOsu } from './recording.mjs';

// Importing the synchronous calculator must leave loadModels free to select its supplied core.
const indexUrl = new URL('../src/index.js', import.meta.url).href;
const custom = spawnSync(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import { loadModels } from ${JSON.stringify(indexUrl)};
    await assert.rejects(loadModels(() => { throw new Error('unexpected model read'); }, { core: new Uint8Array([0]) }), WebAssembly.CompileError);
`], { encoding: 'utf8' });
assert.equal(custom.status, 0, custom.stderr);

const directory = new URL('../../conformance/star/', import.meta.url);
const vectors = JSON.parse(await readFile(new URL('vectors.json', directory), 'utf8'));
for (const vector of vectors) {
    const objects = vector.objects ?? parseOsu(await readFile(new URL(vector.osu, directory), 'utf8'));
    const actual = starRating({ duration_ms: Math.max(0, ...objects.map(o => o.end_ms ?? o.start_ms)), objects });
    assert.equal(typeof actual, 'number');
    if (vector.star_rating === 0) assert.equal(actual, 0, vector.name);
    else assert.ok(Math.abs(actual / vector.star_rating - 1) <= 1e-9, `${vector.name}: ${actual} vs ${vector.star_rating}`);
}
assert.throws(() => starRating({ duration_ms: 1000, objects: [{ lane: 4, start_ms: 0 }] }), /lanes/);
console.log(JSON.stringify({ passed: true, star_vectors: vectors.length }));
