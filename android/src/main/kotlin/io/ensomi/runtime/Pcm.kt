package io.ensomi.runtime

import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Blocking; call on RuntimeWorker. The caller closes the returned Prepared with `use`. */
fun ChartGenerator.prepare(
    pcm: FloatArray,
    sampleRate: Double,
    cancellation: Cancellation? = null,
): Prepared = prepareF32le(pcm.toF32le(), sampleRate, cancellation)

/** Blocking; call on RuntimeWorker. No boxed List<Float> crosses the FFI. */
fun ChartGenerator.generate(
    pcm: FloatArray,
    sampleRate: Double,
    request: GenerateChartRequest,
    listener: GenerationListener? = null,
    cancellation: Cancellation? = null,
): ChartResult = generateF32le(pcm.toF32le(), sampleRate, request, listener, cancellation)

/** Call on RuntimeWorker. The caller closes the returned scheduler with `use`. */
fun ChartScheduler.Companion.fromPcm(
    generator: ChartGenerator,
    pcm: FloatArray,
    sampleRate: Double,
    request: GenerateChartRequest,
    mode: ScheduleMode,
    lead: LeadPolicy,
): ChartScheduler = fromPcmF32le(generator, pcm.toF32le(), sampleRate, request, mode, lead)

/**
 * Indexes a whole mono song at the file's sample rate. Blocking; call on RuntimeWorker. The caller closes the reference
 * with `use`; sessions made from it do not need it afterwards.
 */
fun SyncReference.Companion.fromPcm(pcm: FloatArray, sampleRate: Double): SyncReference =
    fromPcmF32le(pcm.toF32le(), sampleRate)

/**
 * Takes the first [count] samples of a mono block whose first sample was captured at [captureTimeMs] on the caller's
 * monotonic clock. Blocking for a few milliseconds at most, on the capture thread; not for an audio callback.
 */
fun SyncSession.push(samples: FloatArray, captureTimeMs: Double, count: Int = samples.size): SyncSnapshot =
    pushF32le(samples.toF32le(count), captureTimeMs)

internal fun FloatArray.toF32le(count: Int = size): ByteArray {
    val bytes = ByteArray(Math.multiplyExact(count, Float.SIZE_BYTES))
    ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN).asFloatBuffer().put(this, 0, count)
    return bytes
}
