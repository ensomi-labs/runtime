package io.ensomi.runtime

import android.content.Context
import java.io.File
import java.util.concurrent.Executors
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/**
 * A reusable worker for blocking FFI calls, with an explicit 8 MiB native stack.
 * Close it after all callers have completed (cancel and join them first), or use Kotlin's `use`.
 *
 * Cancellation skips queued work, but does not interrupt a running native call. To stop that call,
 * signal its Cancellation.cancel() or ChartScheduler.stop() from another thread. The coroutine
 * waits for the call and cleanup to finish before completing cancellation. Progress callbacks run
 * on the worker too; dispatch UI updates to the main thread.
 */
class RuntimeWorker : AutoCloseable {
    private val executor = Executors.newSingleThreadExecutor { task ->
        Thread(null, task, "Ensomi runtime", 8L shl 20)
    }
    private val dispatcher = executor.asCoroutineDispatcher()

    /**
     * Runs FFI work off the caller's thread. Suspensions resume on this worker unless the block
     * explicitly changes dispatcher. Use this for scheduler work as well as prepare/generate.
     * Acquire and close native objects inside the block with `use`; return data, not native handles,
     * since cancellation can discard the returned value on the way back to the caller.
     */
    suspend fun <T> run(block: suspend () -> T): T {
        check(!executor.isShutdown) { "RuntimeWorker is closed" }
        return withContext(dispatcher) { block() }
    }

    /**
     * Opens a model on the worker, installing the bundled tables when resamplerDirectory is unset.
     * Keeps it open across suspensions and any number of calls in [block], then closes it, including
     * on failure or cancellation. The generator must not escape the block. The caller owns any
     * Prepared, ChartScheduler, Cancellation or GameplaySession it creates: close those with `use`
     * inside the block as well. Return ordinary Kotlin data such as ChartResult.
     */
    suspend fun <T> withGenerator(
        context: Context,
        modelDirectory: File,
        options: GeneratorOptions = GeneratorOptions(ExecutionProvider.CPU),
        block: suspend ChartGenerator.() -> T,
    ): T = withResource({
        ChartGenerator.open(
            modelDirectory.absolutePath,
            options.copy(resamplerDirectory = options.resamplerDirectory ?: bundledResamplerDirectory(context)),
        )
    }, block)

    internal suspend fun <R : AutoCloseable, T> withResource(
        open: () -> R,
        block: suspend R.() -> T,
    ): T = run {
        open().use { resource ->
            // Enter the cancellable scope only after establishing cleanup, even if open was slow.
            // Its children also finish before the resource closes.
            coroutineScope {
                ensureActive()
                resource.block()
            }
        }
    }

    /** Releases the worker thread after callers finish. Does not cancel native work. Idempotent. */
    override fun close() = dispatcher.close()
}
