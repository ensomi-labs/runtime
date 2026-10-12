#!/bin/sh
# Builds the npm package @ensomi/runtime in js/: the Wasm binding of the chart generator into js/pkg, and copies
# of the resampler tables and the licence (all ignored by git). Then `cd js && npm pack` packs it.
# Needs Rust with the wasm32-unknown-unknown target and wasm-bindgen-cli 0.2.129 (cargo install wasm-bindgen-cli --version 0.2.129).
# Run from the repository root. --experimental-reset-state-function exports __wbg_reset_state, with which the JavaScript
# API replaces the core's instance after a trap (js/src/core.js).
set -e
target_dir="${CARGO_TARGET_DIR:-target/generate}"
CARGO_TARGET_DIR="$target_dir" cargo build --release --target wasm32-unknown-unknown -p ensomi-generate-wasm
wasm-bindgen --target web --experimental-reset-state-function --out-dir js/pkg "$target_dir/wasm32-unknown-unknown/release/ensomi_generate_wasm.wasm"
rm -rf js/resampler
cp -R assets/resampler js/resampler
cp LICENSE js/LICENSE
