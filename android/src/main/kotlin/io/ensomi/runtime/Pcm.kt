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

internal fun FloatArray.toF32le(): ByteArray {
    val bytes = ByteArray(Math.multiplyExact(size, Float.SIZE_BYTES))
    ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN).asFloatBuffer().put(this)
    return bytes
}
