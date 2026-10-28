package io.ensomi.runtime

import android.content.Context
import java.io.File
import java.util.concurrent.CompletableFuture

/** ONNX Runtime's MIT licence and third-party notices, for the app's acknowledgements. */
val onnxRuntimeLicenseAssets: List<String> = listOf(
    "licenses/onnxruntime/LICENSE",
    "licenses/onnxruntime/ThirdPartyNotices.txt",
)

/** Opens a model directory, filling an unset resamplerDirectory with the AAR's tables. */
fun ChartGenerator.Companion.open(
    context: Context,
    modelDirectory: String,
    options: GeneratorOptions,
): ChartGenerator = open(
    modelDirectory,
    options.copy(resamplerDirectory = options.resamplerDirectory ?: bundledResamplerDirectory(context)),
)

/** Copies once per set of bundled tables; an interrupted copy is retried on the next open. */
@Synchronized
internal fun bundledResamplerDirectory(context: Context): String {
    val assets = context.assets
    val version = assets.open("resampler.sha256").bufferedReader().use { it.readText().trim() }
    val directory = File(context.noBackupFilesDir, "ensomi-runtime/resampler-$version")
    val complete = File(directory, ".complete")
    if (!complete.exists()) {
        check(directory.isDirectory || directory.mkdirs()) { "Cannot create $directory" }
        for (name in checkNotNull(assets.list("resampler"))) {
            assets.open("resampler/$name").use { input ->
                File(directory, name).outputStream().use { output -> input.copyTo(output) }
            }
        }
        complete.writeText(version)
    }
    return directory.absolutePath
}

/**
 * Runs a blocking prepare, generate or scheduler work call on a new thread with an 8 MiB stack.
 * Progress runs on that thread too. Failures complete the future exceptionally. To stop the native
 * work, use its Cancellation or ChartScheduler.stop(); cancelling the future alone does not stop it.
 */
fun <T> onGenerationThread(body: () -> T): CompletableFuture<T> {
    val result = CompletableFuture<T>()
    Thread(null, {
        try {
            result.complete(body())
        } catch (error: Throwable) {
            result.completeExceptionally(error)
        }
    }, "Ensomi generation", 8L shl 20).start()
    return result
}
