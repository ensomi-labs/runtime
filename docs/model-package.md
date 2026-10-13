# Model package

A released model is a directory: its manifest `ensomi-package.json`, the runtime config `runtime.json` and the files
that config names. The runtime is the only party that interprets the manifest. ensomi-model's exporter writes it, and
the apps and ensomi.app download exactly the files it lists through this runtime. The published R2 package is the
`onnx/` directory of `sed-i/ensomi-r2`; its manifest is [`conformance/package/ensomi-package.json`](../conformance/package/ensomi-package.json).

## Format `ensomi-package/1`

```json
{
  "format": "ensomi-package/1",
  "model": {"id": "ensomi-r2", "name": "Ensomi R2", "version": "r2-1.0"},
  "license": "CC-BY-NC-SA-4.0 AND MIT",
  "notices": ["BeatThis-LICENSE.txt"],
  "pipeline": {"id": "r2", "version": 1},
  "min_runtime": "0.3.0",
  "config": "runtime.json",
  "files": [
    {"path": "BeatThis-LICENSE.txt", "size": 1113, "sha256": "909ab6549794a18e9bb243aacfadda4a5f308436fc2846f350755c53c4f06ae1"}
  ]
}
```

| Field | Meaning |
| --- | --- |
| `format` | Exactly `ensomi-package/1`. |
| `model` | The model's `id`, display `name` and `version`. |
| `license` | The package's licence as an SPDX expression. |
| `notices` | Licence notices the app shows with the model; each is one of `files`. |
| `pipeline` | The host pipeline the package runs on, by `id` and integer `version`. |
| `min_runtime` | The lowest runtime version that reads the package, `major.minor.patch`. |
| `config` | The runtime config's path; one of `files`. |
| `files` | Every file the package needs except the manifest, each with its `path`, byte `size` and lowercase hex `sha256`: the config, graphs, arrays, scope columns and notices. For R2, 30 files and 120,377,244 bytes. |

Paths are relative to the manifest and `/`-separated, with no empty, `.` or `..` part, no leading `/` and no `\`.
`files` is sorted by path (byte order) and lists each path once.

A reader ignores fields it does not know. A later field a reader must understand (such as `graphs`, `capabilities`,
`schedule` or `host`) comes with a `min_runtime` at or above the first runtime that reads it, so an older runtime
refuses the package instead of misreading it.

`model` and `pipeline` must name what this runtime's built-in pipeline implements: `ensomi-r2` (Ensomi R2, `r2-1.0`) on
`r2` version 1 (`crates/generate/src/capabilities.rs`).

## Checks

The manifest check (`ensomi_generate::package::check`; `checkPackage` in Swift and JavaScript) reads the manifest
alone. It refuses a manifest whose format is not `ensomi-package/1`, whose `min_runtime` is above this runtime's
version (naming both), whose `model` or `pipeline` this runtime does not run, which lacks a field above, whose paths
break the rule above or whose hashes are not 64 lowercase hex digits, or whose notices or config are not among its
files. It returns the model, licence, notices, files and their total size, and the config's path.

Once the config is read, a second check (`ensomi_generate::package::check_config`) refuses a manifest that leaves out
a file the config names (its graphs, the arrays, state files and scope columns `Models::new` reads, and its notices),
naming the first: "`<path>` is named by runtime.json but not listed in ensomi-package.json". Swift `open` and
JavaScript `loadModels` both run it before reading any other file.

| When | Who | Checks |
| --- | --- | --- |
| Download | the app's downloader with `checkPackage`; JavaScript `loadModels` | the manifest; then every listed file, fetched and compared with its size and SHA-256. `loadModels` fetches the config first and runs the config check on it before fetching the other files; it reads no file the manifest does not list, and a mismatch fails loading, naming the file. |
| Open | Swift `ChartGenerator.open` (`crates/generate-ort/src/package.rs`) | the manifest; every listed file present at its listed size; then the config it names and the config check. Hashes are not computed at open. |

## Without a manifest

A directory without `ensomi-package.json` (a model installed by a 0.2 client, or a picked folder without it) opens as
in runtime 0.2: `runtime.json` and the files it names, with no hashes. `modelFiles` and `noticeFiles` (Swift) and
`noticeFiles` (JavaScript) list those files from `runtime.json`. In JavaScript a source signals that it has no manifest
by rejecting with an error whose `code` is `'ENOENT'`, as Node's `readFile` does; `urlSource` does so for an HTTP 404
and `localFilesSource` for a file not picked.
