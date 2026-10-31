#!/usr/bin/env bash
# Builds build/android/ensomi-runtime.aar (arm64-v8a) and Kotlin sources beside it, on a Mac.
# Needs cargo-ndk 4.1.2, the aarch64-linux-android Rust target, Android SDK 37 / build-tools 37.0.0,
# NDK 30.0.16248370, Python 3, git and Android Studio's bundled JDK (or JAVA_HOME).
# Gradle uses the checked-in wrapper. ONNX Runtime is built from pinned source and cached in target/.
# Honours CARGO_TARGET_DIR. The AAR carries the resampler tables and the licence notices of ONNX Runtime and Sonalign;
# the consuming app declares JNA 5.19.1@aar and kotlinx-coroutines-core 1.11.0. No model is needed.
set -euo pipefail

cd "$(dirname "$0")/.."
target_dir="${CARGO_TARGET_DIR:-target}"
lib=ensomi_runtime_ffi
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/30.0.16248370"
export JAVA_HOME="${JAVA_HOME:-/Applications/Android Studio.app/Contents/jbr/Contents/Home}"

scripts/build-onnxruntime-android.sh
ort_dir=$(cd "$target_dir/onnxruntime-android/build" && pwd)
work=build/android
mkdir -p "$work"
rm -rf "$work/jni" "$work/kotlin" "$work/assets"
# Set this explicitly even though NDK 30's linker defaults to 16 KiB pages.
ORT_LIB_LOCATION="$ort_dir" \
CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,--no-undefined" \
  cargo ndk -t arm64-v8a -P 24 --link-libcxx-shared -o "$work/jni" \
  build --locked --release -p ensomi-runtime-ffi --features sync
# Sonalign also declares a cdylib, which cargo-ndk copies here. The runtime links it statically and never loads this one.
rm -f "$work/jni/arm64-v8a/"libsonalign*.so

# Check every native library, including the NDK's shared C++ runtime.
readelf="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-readelf"
for so in "$work/jni/arm64-v8a/"*.so; do
  echo "$so"
  "$readelf" -lW "$so" | awk '
    $1 == "LOAD" { print; count++; if ($NF != "0x4000") bad = 1 }
    END { if (!count || bad) exit 1 }
  '
done

cargo run --locked --release -p ensomi-runtime-ffi --features bindgen --bin uniffi-bindgen -- generate \
  --library "$target_dir/aarch64-linux-android/release/lib$lib.so" \
  --language kotlin --out-dir "$work/kotlin" --no-format
python3 scripts/fix-kotlin-errors.py "$work/kotlin/io/ensomi/runtime/$lib.kt"

mkdir -p "$work/assets/licenses"
cp -R assets/resampler "$work/assets/"
cp -R licenses/onnxruntime licenses/sonalign "$work/assets/licenses/"
# The convenience helper reuses its private copy until the bundled tables change.
shasum -a 256 assets/resampler/* | shasum -a 256 | awk '{print $1}' > "$work/assets/resampler.sha256"

android/gradlew -p android --no-daemon assembleRelease
cp "$work/gradle/outputs/aar/ensomi-runtime-release.aar" "$work/ensomi-runtime.aar"
shasum -a 256 "$work/ensomi-runtime.aar"
