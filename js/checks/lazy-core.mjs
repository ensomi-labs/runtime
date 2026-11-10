// Model-free regression for bundled consumers that supply their own core. The package copy has no default Wasm.
// Optional --package <directory> checks another revision's JS with the same built core.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { cp, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseOsu } from './recording.mjs';

const source = process.argv.includes('--package') ? resolve(process.argv[process.argv.indexOf('--package') + 1]) : fileURLToPath(new URL('../', import.meta.url));
const directory = new URL('../../conformance/star/', import.meta.url);
const vectors = JSON.parse(await readFile(new URL('vectors.json', directory), 'utf8'));
const charts = await Promise.all(vectors.map(async vector => {
    const objects = vector.objects ?? parseOsu(await readFile(new URL(vector.osu, directory), 'utf8'));
    return { name: vector.name, expected: vector.star_rating, chart: { duration_ms: Math.max(0, ...objects.map(o => o.end_ms ?? o.start_ms)), objects } };
}));
const temporary = await mkdtemp(join(tmpdir(), 'ensomi-lazy-core-'));
try {
    for (const path of ['src', 'pkg', 'package.json'])
        await cp(join(source, path), join(temporary, path), { recursive: true });
    await symlink(fileURLToPath(new URL('../node_modules/', import.meta.url)), join(temporary, 'node_modules'), 'dir');
    await cp(join(temporary, 'pkg/ensomi_generate_wasm_bg.wasm'), join(temporary, 'supplied-core.wasm'));
    await rm(join(temporary, 'pkg/ensomi_generate_wasm_bg.wasm'));
    await writeFile(join(temporary, 'inputs.json'), JSON.stringify({ charts, manifest: await readFile(new URL('../../conformance/package/ensomi-package.json', import.meta.url), 'utf8') }));
    await writeFile(join(temporary, 'check.mjs'), `
        import assert from 'node:assert/strict';
        import fs from 'node:fs/promises';

        const bytes = await fs.readFile(new URL('./supplied-core.wasm', import.meta.url));
        const { charts, manifest } = JSON.parse(await fs.readFile(new URL('./inputs.json', import.meta.url), 'utf8'));
        await assert.rejects(fs.access(new URL('./pkg/ensomi_generate_wasm_bg.wasm', import.meta.url)), { code: 'ENOENT' });

        // Only package evaluation is guarded: setup is finished, and explicit core initialization follows it.
        const guarded = [[globalThis, 'fetch'], [fs, 'readFile'], ...['compile', 'compileStreaming', 'instantiate', 'instantiateStreaming', 'Module', 'Instance'].map(key => [WebAssembly, key])];
        const originals = guarded.map(([object, key]) => object[key]);
        for (const [object, key] of guarded)
            object[key] = function () { throw new Error('Package import attempted ' + key); };
        let runtime;
        try { runtime = await import('./src/index.js'); }
        finally { guarded.forEach(([object, key], i) => { object[key] = originals[i]; }); }

        // loadModels initializes the supplied core before reading the manifest. Stop at that boundary so CI
        // needs no model files or inference mock, then exercise public APIs on the real selected Wasm core.
        const stop = new Error('model read reached');
        await assert.rejects(runtime.loadModels(async path => {
            assert.equal(path, 'ensomi-package.json');
            throw stop;
        }, { core: bytes }), error => error === stop);
        const info = await runtime.checkPackage(manifest);
        assert.equal(info.model.id, 'ensomi-r2');
        for (const { name, expected, chart } of charts) {
            const rating = runtime.starRating(chart);
            assert.ok(rating instanceof Promise);
            const actual = await rating;
            if (expected === 0) assert.equal(actual, 0, name);
            else assert.ok(Math.abs(actual / expected - 1) <= 1e-9, name + ': ' + actual + ' vs ' + expected);
        }
        await assert.rejects(runtime.starRating({ duration_ms: 0, objects: [{ lane: 4, start_ms: 0 }] }), /lanes/);
        console.log(JSON.stringify({ passed: true, default_core_missing: true, import_io_and_wasm_work: false, supplied_core_vectors: charts.length }));
    `);
    const result = spawnSync(process.execPath, [join(temporary, 'check.mjs')], { encoding: 'utf8', timeout: 30000 });
    process.stdout.write(result.stdout ?? '');
    process.stderr.write(result.stderr ?? '');
    assert.equal(result.status, 0, result.error?.message ?? 'lazy-core regression failed');
}
finally {
    await rm(temporary, { recursive: true, force: true });
}
