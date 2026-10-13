# Model package manifest

`ensomi-package.json` is the manifest of the published R2 package (`sed-i/ensomi-r2`, `onnx/`, `r2-1.0`), as
ensomi-model's exporter wrote it: 30 files, 120,377,244 bytes, every SHA-256 equal to the release tag's `SHA256SUMS`.
The format is in [`docs/model-package.md`](../../docs/model-package.md).

The core's check reads it as it is and with one thing changed (`crates/generate/tests/package.rs`). With the released
files beside it, where bings-mac keeps them, `open` accepts it natively and in Swift, and `js/checks/package.mjs`
loads it through the packed npm package, checking every file's SHA-256 ([`RELEASING.md`](../../RELEASING.md) steps 6
and 7).
