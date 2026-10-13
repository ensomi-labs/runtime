// Loads the released model with an installed @ensomi/runtime as a client would, with and without ensomi-package.json:
// over HTTP from a server in this process (fetch, and a 404 for a file it does not have) and from a picked folder.
// With the published package's manifest beside the released files, loading fetches exactly the files it lists, checks
// each one's SHA-256, and takes models.package and models.notices from it; one changed byte in a listed file fails
// loading, naming the file; without the manifest the model loads from runtime.json as before.
//
// Run where the packed tarball is installed, with this file copied there (RELEASING.md):
//   node package.mjs --model-dir <ensomi-r2/onnx> --manifest <conformance/package/ensomi-package.json>
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { loadModels, localFilesSource } from '@ensomi/runtime';

const args = process.argv.slice(2), option = (name) => args[args.indexOf(name) + 1];
const modelDir = resolve(option('--model-dir'));
const manifest = await readFile(resolve(option('--manifest')));
const listed = JSON.parse(manifest);
const changed = 'scopes-S.bin';

// /release/ serves the released directory, /package/ the same with the manifest, /changed/ that with one bit of
// `changed` flipped. Every path served is recorded under its root.
const served = { release: [], package: [], changed: [] };
const server = createServer(async (request, response) => {
    const [, root, path] = request.url.split('/');
    let bytes = path === 'ensomi-package.json' ? (root === 'release' ? undefined : manifest) : await readFile(join(modelDir, path)).catch(() => undefined);
    if (bytes && root === 'changed' && path === changed) {
        bytes = Buffer.from(bytes);
        bytes[0] ^= 1;
    }
    served[root]?.push(path);
    response.writeHead(bytes ? 200 : 404).end(bytes);
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const base = `http://127.0.0.1:${server.address().port}`;

const options = { backend: 'wasm', wasm: { numThreads: 1 } };
async function load(source) {
    const models = await loadModels(source, options);
    const summary = { graphs: Object.keys(models.sessions).length, notices: models.notices.map(n => n.path), package: models.package && { files: models.package.files.length, total_bytes: models.package.totalBytes } };
    await models.dispose();
    return summary;
}

const report = {
    with_manifest: await load(`${base}/package/`),
    without_manifest: await load(`${base}/release/`),
    changed_file: await loadModels(`${base}/changed/`, options).then(() => 'loaded', error => error.message),
    picked_folder_without_manifest: await load(localFilesSource(await Promise.all(listed.files.map(async f => new File([await readFile(join(modelDir, f.path))], f.path))))),
};
server.close();
report.package_requests_are_the_listed_files = JSON.stringify(served.package.toSorted()) === JSON.stringify(['ensomi-package.json', ...listed.files.map(f => f.path)].toSorted());
console.log(JSON.stringify(report));

const total = listed.files.reduce((sum, f) => sum + f.size, 0);
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const passed = same(report.with_manifest, { graphs: 6, notices: listed.notices, package: { files: listed.files.length, total_bytes: total } })
    && same(report.without_manifest, { graphs: 6, notices: listed.notices })
    && report.changed_file.startsWith(`${changed} does not match ensomi-package.json`)
    && same(report.picked_folder_without_manifest, { graphs: 6, notices: listed.notices })
    && report.package_requests_are_the_listed_files;
console.log(JSON.stringify({ passed }));
if (!passed)
    process.exitCode = 1;
