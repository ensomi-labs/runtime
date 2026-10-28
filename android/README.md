# Android build (macOS only for now)

Install the `aarch64-linux-android` Rust target, cargo-ndk 4.1.2, Android SDK 37, build-tools 37.0.0 and NDK
30.0.16248370, with Python 3 and git available. The script defaults to `~/Library/Android/sdk` (`ANDROID_HOME`
overrides it) and Android Studio's bundled JDK (`JAVA_HOME` overrides it). The checked-in wrapper uses
Gradle 9.8.2, AGP 9.4.1 and Kotlin 2.4.21,
with JVM target 17. No model is needed. Run all commands from the repository root.

```sh
scripts/build-android.sh
```

This builds `build/android/ensomi-runtime.aar` and keeps the generated source in `build/android/kotlin/`.
It honours `CARGO_TARGET_DIR`, builds native code at API 24, rejects unresolved native symbols, and checks
that both native libraries have 16 KiB LOAD alignment. The AAR includes compiled bindings and helpers,
`libensomi_runtime_ffi.so`, `libc++_shared.so`, the resampler assets and `assets/licenses/onnxruntime`
(exposed as `onnxRuntimeLicenseAssets`). It is a local build, with no Android CI or release asset yet.

`build-onnxruntime-android.sh` builds ONNX Runtime 1.23.2 (the version selected by `ort-sys` rc.11) from a pinned
commit with the same NDK and API level. Pyke's rc.11 Android archive contains GNU libstdc++ references and
fails to load on Android. The source build and its private CMake 3.31.10 / Ninja 1.13.0 venv are cached under
`CARGO_TARGET_DIR`; deleting `build/android` keeps that cache. The Rust dependency pins stay unchanged.

In a Kotlin app module (minSdk 33, targetSdk 36, compileSdk 37):

```kotlin
dependencies {
    implementation(files("/absolute/path/to/build/android/ensomi-runtime.aar"))
    implementation("net.java.dev.jna:jna:5.19.1@aar")
}
```

Import `io.ensomi.runtime.*`. `ChartGenerator.open(context, modelDirectory, options)` copies the bundled tables
to `noBackupFilesDir` once per table version and fills an unset `resamplerDirectory`. For song PCM use
`prepareF32le`, `generateF32le` and `ChartScheduler.fromPcmF32le`: their `ByteArray` holds mono little-endian
f32 samples, avoiding Kotlin's boxed `List<Float>`. The existing float APIs remain available.
`onGenerationThread { /* blocking FFI call */ }` returns a `CompletableFuture` and runs the call and progress
on a new thread with an 8 MiB stack. Stop native work with `Cancellation.cancel()` or `ChartScheduler.stop()`;
cancelling the future alone does not stop it.

UniFFI stays at 0.31: API checksum comparisons are omitted for its AArch64 Kotlin bug
([upstream fix](https://github.com/mozilla/uniffi-rs/pull/2935)); keep each AAR's classes and native library together.
Its 0.31.2 Android cleaner checks API 34 before using `SystemCleaner`, falling back to JNA on API 33.
The script also fixes the 0.31 error template's duplicate `Throwable.message`: the Rust payload becomes
the override, retaining its name and wire format. The guarded patch is in `scripts/fix-kotlin-errors.py`.

With an arm64 emulator already booted (use `ensomi-36-16k` for the 16 KiB check), run the model-free binding,
asset and worker tests, then shut the emulator down:

```sh
export ANDROID_HOME="$HOME/Library/Android/sdk"
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
android/gradlew -p android --no-daemon connectedAndroidTest
"$ANDROID_HOME/platform-tools/adb" emu kill
```
