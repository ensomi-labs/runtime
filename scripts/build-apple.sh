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
platforms=(macos ios ios-simulator)
versions=("$MACOSX_DEPLOYMENT_TARGET" "$IPHONEOS_DEPLOYMENT_TARGET" "$IPHONEOS_DEPLOYMENT_TARGET")
for target in "${targets[@]}"; do
  cargo build --release -p ensomi-runtime-ffi --target "$target"
done

work=build/apple/work
# Headers sit in a directory named after the module, where clang looks for its module map: an app that
# links another binary package with a top-level module.modulemap would otherwise get two files at
# include/module.modulemap.
headers="$work/headers/${lib}FFI"
rm -rf "$work" build/apple/EnsomiRuntimeFFI.xcframework
mkdir -p "$headers"

# The bindings are generated from the metadata embedded in the macOS library.
cargo run --release -p ensomi-runtime-ffi --features bindgen --bin uniffi-bindgen -- generate \
  --library "$target_dir/aarch64-apple-darwin/release/lib$lib.dylib" \
  --language swift --out-dir "$work"

cp "$work/${lib}FFI.h" "$headers/"
cp "$work/${lib}FFI.modulemap" "$headers/module.modulemap"
cp "$work/$lib.swift" swift/Sources/EnsomiRuntime/

# Every Rust static library carries Rust's std, whose one unmangled symbol, rust_eh_personality, clashes
# with any other Rust library linked into the app (Sonalign). Pre-link each archive into one object that
# exports only the FFI functions.
printf '_uniffi_%s_*\n_ffi_%s_*\n' "$lib" "$lib" > "$work/exported-symbols"
args=()
for i in "${!targets[@]}"; do
  target=${targets[$i]}
  mkdir -p "$work/$target"
  xcrun ld -r -arch arm64 -platform_version "${platforms[$i]}" "${versions[$i]}" "${versions[$i]}" \
    -exported_symbols_list "$work/exported-symbols" \
    -force_load "$target_dir/$target/release/lib$lib.a" -o "$work/$target/$lib.o"
  libtool -static -o "$work/$target/lib$lib.a" "$work/$target/$lib.o"
  args+=(-library "$work/$target/lib$lib.a" -headers "$work/headers")
done
xcodebuild -create-xcframework "${args[@]}" -output build/apple/EnsomiRuntimeFFI.xcframework
