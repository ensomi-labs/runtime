#!/usr/bin/env bash
# Builds build/apple/EnsomiRuntimeFFI.xcframework (macOS arm64, iOS arm64, iOS simulator arm64) and
# regenerates the Swift sources of the swift/ package. Run on a Mac with Xcode and the three Rust
# targets installed; honours CARGO_TARGET_DIR.
set -euo pipefail

cd "$(dirname "$0")/.."
target_dir="${CARGO_TARGET_DIR:-target}"
lib=ensomi_runtime_ffi
export MACOSX_DEPLOYMENT_TARGET=14.0
export IPHONEOS_DEPLOYMENT_TARGET=17.0

targets=(aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim)
for target in "${targets[@]}"; do
  cargo build --release -p ensomi-runtime-ffi --target "$target"
done

work=build/apple/work
rm -rf "$work" build/apple/EnsomiRuntimeFFI.xcframework
mkdir -p "$work/headers"

# The bindings are generated from the metadata embedded in the macOS library.
cargo run --release -p ensomi-runtime-ffi --features bindgen --bin uniffi-bindgen -- generate \
  --library "$target_dir/aarch64-apple-darwin/release/lib$lib.dylib" \
  --language swift --out-dir "$work"

cp "$work/${lib}FFI.h" "$work/headers/"
cp "$work/${lib}FFI.modulemap" "$work/headers/module.modulemap"
cp "$work/$lib.swift" swift/Sources/EnsomiRuntime/

args=()
for target in "${targets[@]}"; do
  args+=(-library "$target_dir/$target/release/lib$lib.a" -headers "$work/headers")
done
xcodebuild -create-xcframework "${args[@]}" -output build/apple/EnsomiRuntimeFFI.xcframework
