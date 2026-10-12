# Releasing

A release `X.Y.Z` is the tag `vX.Y.Z` on `main`. It ships two packages built from that commit:

- the Swift package: `EnsomiRuntimeFFI.xcframework.zip` on the GitHub release, pinned by URL and checksum in
  `Package.swift`;
- the npm package `@ensomi/runtime`, published with provenance through npm trusted publishing.

On the tag push, [`release.yml`](.github/workflows/release.yml):

1. checks that the tag is `main`'s head and that `Cargo.toml` and `js/package.json` both carry `X.Y.Z` (the core
   writes its Cargo version into receipts and checks model packages' runtime ranges against it);
2. runs [`ci.yml`](.github/workflows/ci.yml): Rust tests and clippy, the Wasm build and `npm pack`, the
   xcframework and `swift test` on macOS;
3. commits the xcframework's URL and checksum to `Package.swift` on `main`, moves the tag onto that commit
   (SwiftPM reads `Package.swift` at the tag) and creates the GitHub release with the zip;
4. publishes the tarball CI packed: npm dist-tag `latest`, or `next` for a version such as `0.2.0-rc.1`.

## One-time setup

### npm

npm takes a trusted publisher only for a package that exists. `@ensomi/protocol` got there by publishing its
first version (0.1.0) by hand, then configuring the publisher; 0.2.0 went out from GitHub Actions with provenance.
Here a placeholder takes the hand-published slot, so that 0.1.0 is already the CI-built package with provenance.

1. With npm 11.5.1 or later, logged in as an owner of the `@ensomi` scope (`npm login`; `npm whoami` prints
   `s3d-i`):

   ```sh
   cd "$(mktemp -d)"
   cat > package.json <<'EOF'
   {
     "name": "@ensomi/runtime",
     "version": "0.0.0",
     "description": "Placeholder that reserves the name; the first release is 0.1.0.",
     "license": "Apache-2.0",
     "repository": { "type": "git", "url": "git+https://github.com/ensomi-labs/runtime.git", "directory": "js" }
   }
   EOF
   npm publish --access public
   npm deprecate @ensomi/runtime@0.0.0 "Placeholder; use 0.1.0 or later."
   ```

2. Open <https://www.npmjs.com/package/@ensomi/runtime/access> (the package's Settings). Under Trusted Publisher
   choose GitHub Actions, enter the values below and save:

   | Field | Value |
   | --- | --- |
   | Organization or user | `ensomi-labs` |
   | Repository | `runtime` |
   | Workflow filename | `release.yml` |
   | Environment name | `npm` |

   Then, under Publishing access on the same page, choose "Require two-factor authentication and disallow
   tokens": the workflow needs no token.

### GitHub

3. Create the `npm` environment, limited to release tags (without this the first release creates it with no rule):

   ```sh
   gh api -X PUT repos/ensomi-labs/runtime/environments/npm \
     -F 'deployment_branch_policy[protected_branches]=false' -F 'deployment_branch_policy[custom_branch_policies]=true'
   gh api -X POST repos/ensomi-labs/runtime/environments/npm/deployment-branch-policies -f name='v*' -f type=tag
   ```

4. Keep `main` and `v*` tags writable by GitHub Actions: the release pushes the `Package.swift` commit to `main`
   and force-moves the tag. `main` is unprotected and the repository has no rulesets; a rule added later must
   exempt GitHub Actions.

## Before a release (bings-mac)

These need data CI does not have. Run them in `~/ensomi/ensomi-runtime` on the commit to be tagged.

5. Generator replay parity on the four release recordings (4.3 GB in `ensomi-model/artifacts/runtime-ref`); both
   reports end with `"passed": true` ([`conformance/generate`](conformance/generate/README.md) has the criteria):

   ```sh
   CARGO_TARGET_DIR=target/generate cargo run --release -p ensomi-generate-ort --bin parity -- \
     --model-dir ../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
     --reference ../ensomi-model/artifacts/runtime-ref --assets assets/resampler \
     --out conformance/generate/bulk/native-parity.json
   scripts/wasm-build.sh && (cd js && npm install)
   node js/checks/replay.mjs --model-dir ../ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2 \
     --reference ../ensomi-model/artifacts/runtime-ref --assets assets/resampler \
     --core js/pkg/ensomi_generate_wasm_bg.wasm --out conformance/generate/bulk/wasm-parity.json
   ```

6. The Swift generator tests, which CI skips without the model; all pass:

   ```sh
   scripts/build-apple.sh
   ENSOMI_RUNTIME_XCFRAMEWORK=build/apple/EnsomiRuntimeFFI.xcframework swift test
   ```

7. The packed npm package from a clean directory, with only its own Wasm core and resampler tables; it prints
   `"same_hit_objects":true`:

   ```sh
   scripts/wasm-build.sh
   out=$(mktemp -d) && (cd js && npm pack --pack-destination "$out")
   cd "$(mktemp -d)" && npm init -y > /dev/null && npm install "$out"/ensomi-runtime-*.tgz
   cp ~/ensomi/ensomi-runtime/js/checks/packed.mjs ~/ensomi/ensomi-runtime/js/checks/recording.mjs .
   node packed.mjs --model-dir ~/ensomi/ensomi-model/artifacts/release-r2/bulk/hf/ensomi-r2/onnx \
     --recording ~/ensomi/ensomi-model/artifacts/runtime-ref/bulk/hesitation-snow \
     --chart ~/ensomi/ensomi-runtime/conformance/generate/native-charts/hesitation-snow
   ```

## Cutting the release

8. For a version other than the current one: on bings-mac set `version` under `[workspace.package]` in
   `Cargo.toml`, run `cargo update --workspace` (`Cargo.lock`) and `cd js && npm version X.Y.Z
   --no-git-tag-version` (`package.json`, `package-lock.json`); commit and push `main` from the control plane.
   0.1.0 is already set.

9. On the control plane, tag `main`'s head and push the tag:

   ```sh
   git switch main && git pull
   git tag -a vX.Y.Z -m vX.Y.Z && git push origin vX.Y.Z
   gh run list --workflow release.yml --limit 1    # then gh run watch <id>
   ```

10. Afterwards `git pull` and `git fetch --tags --force`: `main` has the workflow's `Package.swift` commit and the
    tag has moved onto it.

If `npm` fails after `github-release` succeeded (for instance before the trusted publisher exists), fix the cause
and re-run that job from the run page; it publishes the same tarball.

## Using a release

- Swift: `.package(url: "https://github.com/ensomi-labs/runtime", from: "0.1.0")` and
  `.product(name: "EnsomiRuntime", package: "runtime")`.
- npm: `npm install @ensomi/runtime`.
