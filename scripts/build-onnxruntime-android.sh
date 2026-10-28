#!/usr/bin/env bash
# Builds ONNX Runtime 1.23.2's static Android libraries with NDK 30, cached under CARGO_TARGET_DIR.
# The ort rc.11 Android download contains GNU libstdc++ references and does not load on Android.
# Needs Python 3 and git; installs pinned CMake/Ninja into a private venv, never the system Python.
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p "${CARGO_TARGET_DIR:-target}"
target_dir=$(cd "${CARGO_TARGET_DIR:-target}" && pwd)
work="$target_dir/onnxruntime-android"
tools="$target_dir/android-tools"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/30.0.16248370"

if [[ ! -x "$tools/bin/cmake" || ! -x "$tools/bin/ninja" ]]; then
  python3 -m venv "$tools"
  "$tools/bin/pip" install cmake==3.31.10 ninja==1.13.0
fi
export PATH="$tools/bin:$PATH"

# v1.23.2, the ONNX Runtime version selected by ort-sys 2.0.0-rc.11's download table.
revision=a83fc4d58cb48eb68890dd689f94f28288cf2278
if [[ ! -d "$work/source/.git" ]]; then
  git init "$work/source"
  git -C "$work/source" remote add origin https://github.com/microsoft/onnxruntime.git
  git -C "$work/source" fetch --depth 1 origin "$revision"
  git -C "$work/source" checkout --detach FETCH_HEAD
fi
[[ $(git -C "$work/source" rev-parse HEAD) == "$revision" ]]

python "$work/source/tools/ci_build/build.py" \
  --build_dir "$work/build" --config Release --update --build --skip_tests --skip_submodule_sync \
  --cmake_generator Ninja --parallel 4 --compile_no_warning_as_error \
  --android --android_sdk_path "$ANDROID_HOME" --android_ndk_path "$ANDROID_NDK_HOME" \
  --android_abi arm64-v8a --android_api 24 --android_cpp_shared \
  --cmake_extra_defines onnxruntime_BUILD_UNIT_TESTS=OFF FETCHCONTENT_TRY_FIND_PACKAGE_MODE=NEVER
# re2 is EXCLUDE_FROM_ALL upstream; the static-only build has no final link target to pull it in.
cmake --build "$work/build/Release" --target re2 --parallel 4
