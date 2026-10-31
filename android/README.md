# Android build (macOS only for now)

Install the `aarch64-linux-android` Rust target, cargo-ndk 4.1.2, Android SDK 37, build-tools 37.0.0 and NDK
30.0.16248370, with Python 3 and git available. The script defaults to `~/Library/Android/sdk` (`ANDROID_HOME`
overrides it) and Android Studio's bundled JDK (`JAVA_HOME` overrides it). The checked-in wrapper uses
Gradle 9.8.2, AGP 9.4.1 and Kotlin 2.4.21,
with JVM target 17. No model is needed. Run all commands from the repository root.

```sh
scripts/build-android.sh
```

This builds `build/android/ensomi-runtime.aar`, with the bindings' `sync` feature on, and keeps the generated source
in `build/android/kotlin/`.
It honours `CARGO_TARGET_DIR`, builds native code at API 24, rejects unresolved native symbols, and checks
that both native libraries have 16 KiB LOAD alignment. The AAR includes compiled bindings and helpers,
`libensomi_runtime_ffi.so`, `libc++_shared.so`, the resampler assets, `assets/licenses/onnxruntime`
(exposed as `onnxRuntimeLicenseAssets`) and Sonalign's notice at `assets/licenses/sonalign/LICENSE`. It is a local build,
with no Android CI or release asset yet.

`build-onnxruntime-android.sh` builds ONNX Runtime 1.23.2 (the version selected by `ort-sys` rc.11) from a pinned
commit with the same NDK and API level. Pyke's rc.11 Android archive contains GNU libstdc++ references and
fails to load on Android. The source build and its private CMake 3.31.10 / Ninja 1.13.0 venv are cached under
`CARGO_TARGET_DIR` (default: `target/onnxruntime-android/{source,build}` and `target/android-tools`);
deleting `build/android` keeps that cache. This is the CPU execution provider, with ARM KleidiAI kernels;
XNNPACK and NNAPI are disabled. The Rust dependency pins stay unchanged.

Android sessions disable ORT memory patterns while retaining the CPU arena by default. `lowMemory` also disables
the arena, preserving the lower-peak mode. On a Redmi K Pad, disabling patterns alone reduced peak app RSS by
about 0.65 GB across three songs without changing their chart bytes; it avoids the large BeatThis slowdown of
disabling both controls. These are device measurements, not a universal memory bound. See the
[paired measurements and chart gates](../artifacts/android-perf-1/report.md).

BeatThis defaults to at most four threads on Android; the other five graphs use one. Apps can set
`GeneratorOptions.beatthisThreads` for a measured device. Eight threads improved this K Pad's BeatThis time,
but the runtime does not assume that every Android device benefits from eight threads.

In a Kotlin app module (minSdk 33, targetSdk 36, compileSdk 37):

```kotlin
dependencies {
    implementation(files("/absolute/path/to/build/android/ensomi-runtime.aar"))
    implementation("net.java.dev.jna:jna:5.19.1@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.11.0")
}
```

The local AAR has no dependency metadata: the app declares both dependencies above. Coroutines is pinned to
[1.11.0](https://github.com/Kotlin/kotlinx.coroutines/releases/tag/1.11.0). Import `io.ensomi.runtime.*`.

`RuntimeWorker` owns a reusable single-thread coroutine dispatcher, constructed with an explicit 8 MiB stack.
It has this public API (default arguments shown):

```kotlin
class RuntimeWorker : AutoCloseable {
    suspend fun <T> run(block: suspend () -> T): T
    suspend fun <T> withGenerator(
        context: Context,
        modelDirectory: File,
        options: GeneratorOptions = GeneratorOptions(ExecutionProvider.CPU),
        block: suspend ChartGenerator.() -> T,
    ): T
    override fun close()
}
```

`withGenerator` opens a model on the worker and closes it after the suspending block finishes, including on
failure or cancellation. It fills an unset `resamplerDirectory` with tables installed under `noBackupFilesDir`.
Each set of tables has a directory keyed by the SHA-256 of the bundled files. A process/thread lock and atomic
directory rename prevent partial copies from reaching callers; failures can be retried. An AAR with changed
tables gets a new directory. Old directories stay available to generators that are still using them.

For example, from an app coroutine:

```kotlin
suspend fun makeChart(context: Context, modelDirectory: File, pcm: FloatArray): ChartResult =
    RuntimeWorker().use { worker ->
        worker.withGenerator(context, modelDirectory) {
            generate(pcm, sampleRate = 48_000.0, request = GenerateChartRequest(star = 4.0))
        }
    }
```

The worker and generator can each live across many calls and suspensions; the example scopes them to one song.
`run` also supports existing generators and scheduler work: create a scheduler inside a worker block, wrap it
in `use`, then call `work()` there. A suspension resumes on the worker unless the app explicitly changes the
dispatcher. Progress callbacks run on this worker too; send UI updates to the main thread.

These blocking overloads convert `FloatArray` directly to little-endian bytes, then call the byte FFI:

```kotlin
fun ChartGenerator.prepare(pcm: FloatArray, sampleRate: Double, cancellation: Cancellation? = null): Prepared
fun ChartGenerator.generate(pcm: FloatArray, sampleRate: Double, request: GenerateChartRequest,
    listener: GenerationListener? = null, cancellation: Cancellation? = null): ChartResult
fun ChartScheduler.Companion.fromPcm(generator: ChartGenerator, pcm: FloatArray, sampleRate: Double,
    request: GenerateChartRequest, mode: ScheduleMode, lead: LeadPolicy): ChartScheduler
```

For an existing little-endian mono f32 `ByteArray`, call the generated `prepareF32le`, `generateF32le` or
`ChartScheduler.fromPcmF32le` inside the same worker block. Neither path builds a boxed `List<Float>`.
Do not mutate the PCM while a call is reading it.

The app closes the worker after all its callers finish (cancel and join them first), usually with `use`.
`withGenerator` owns its generator; do not retain it outside the block. The app owns any `Prepared`,
`ChartScheduler`, `Cancellation` or `GameplaySession` it creates: close those with `use` inside the block too.
Return ordinary data such as `ChartResult` from worker blocks, rather than native handles: coroutine
cancellation can discard a returned value. The generated objects implement `AutoCloseable`.

Coroutine cancellation skips work that has not started. It does not interrupt blocking Rust: a running call
and its resource cleanup finish before the coroutine completes cancellation. To stop native work early, signal
its `Cancellation.cancel()` or `ChartScheduler.stop()` from another thread; do not queue the stop behind the
running call on the same worker. Native cancellation remains cooperative at the runtime's existing checkpoints.

UniFFI stays at 0.31: API checksum comparisons are omitted for its AArch64 Kotlin bug
([upstream fix](https://github.com/mozilla/uniffi-rs/pull/2935)); keep each AAR's classes and native library together.
Its 0.31.2 Android cleaner checks API 34 before using `SystemCleaner`, falling back to JNA on API 33.
The script also fixes the 0.31 error template's duplicate `Throwable.message`: the Rust payload becomes
the override, retaining its name and wire format. The guarded patch is in `scripts/fix-kotlin-errors.py`.

With an arm64 emulator already booted (use `ensomi-36-16k` for the 16 KiB check), run the model-free binding,
asset, sync and worker tests, then shut the emulator down:

```sh
export ANDROID_HOME="$HOME/Library/Android/sdk"
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
android/gradlew -p android --no-daemon connectedAndroidTest
"$ANDROID_HOME/platform-tools/adb" emu kill
```

## Live audio sync

`SyncReference` indexes the song from its decoded PCM; `SyncSession` listens to the microphone and reports where in the
song the room is. Ensomi plays no audio: the position only joins the game to music already playing.

```kotlin
class SyncReference : AutoCloseable {
    companion object {
        fun fromPcmF32le(pcm: ByteArray, sampleRate: Double): SyncReference // throws SyncException
    }
    fun durationMs(): Double
}
class SyncSession(reference: SyncReference, sampleRate: Double) : AutoCloseable { // throws SyncException
    fun pushF32le(pcm: ByteArray, captureTimeMs: Double): SyncSnapshot // throws SyncException
}
data class SyncSnapshot(
    val state: SyncState, // READY, LISTENING, LOCKING, CONFIRMED, LOCKED, DRIFTING, RELOCKING, LOST, FAILED
    val isFinal: Boolean, // The lock to act on.
    val referenceTimeMs: Double?, // Both set or both unset, and provisional until isFinal.
    val captureTimeMs: Double?,
    val confidence: Double, // Spectral agreement from 0 to 1, for display. Nothing gates on it.
    val withholdReason: SyncWithholdReason?,
)
sealed class SyncException : Exception() { class Refused(val detail: String) : SyncException() }

// In Pcm.kt, over the same byte calls:
fun SyncReference.Companion.fromPcm(pcm: FloatArray, sampleRate: Double): SyncReference
fun SyncSession.push(samples: FloatArray, captureTimeMs: Double, count: Int = samples.size): SyncSnapshot
```

Index the same mono PCM the chart is generated from, at the file's rate, so that a reported time is a chart time. Build
the reference on `RuntimeWorker` or another background thread. It takes no `Cancellation`: a build is short (0.1 to 0.25 s
per minute of audio on an arm64 emulator), and coroutine cancellation waits for it.

Push from the thread that reads the microphone, not from an audio callback. Every 100 ms of audio a push runs a search,
and a thread that falls behind is safe as long as `AudioRecord`'s own buffer holds the wait: give it at least 500 ms. The
session converts any rate from 8 to 192 kHz; 48 kHz needs no conversion. A lock takes at least 3.6 s of audio, and longer
when the passage gives weak or ambiguous evidence.

`captureTimeMs` is when the block's first sample was captured, on one monotonic clock in milliseconds. Take it from
`AudioRecord.getTimestamp`. `framesRead` counts the frames read before this block:

```kotlin
record.getTimestamp(stamp, AudioTimestamp.TIMEBASE_MONOTONIC)
val captureNs = stamp.nanoTime + (framesRead - stamp.framePosition) * 1_000_000_000 / rate
val snapshot = session.push(block, captureNs / 1e6, count = read)
framesRead += read
if (snapshot.isFinal) {
    val songMs = snapshot.referenceTimeMs!! + (System.nanoTime() / 1e6 - snapshot.captureTimeMs!!)
}
```

Before a timestamp is available, the time a read returned less the block's duration will do, but it stays late by the
device's input latency, and that error reaches the position. Jitter does not: the session keeps the earliest stamp of
the last five seconds against its sample count, and treats stamps that stay more than two hops late for 500 ms as audio
lost, restarting the search without it. `isFinal` then turns false until the session has locked again.

Stop pushing at the first `isFinal`, as the Swift app does, and project from that one snapshot. A `Refused` block changes
nothing, and the next is taken. Calls to one session take a lock, so they are safe, though they serve one stream.
