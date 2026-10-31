package io.ensomi.runtime

import android.system.Os
import android.system.OsConstants
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.sun.jna.Library
import com.sun.jna.Memory
import com.sun.jna.Native
import com.sun.jna.Pointer
import com.sun.jna.ptr.LongByReference
import java.io.File
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.file.Files
import java.security.MessageDigest
import java.util.concurrent.CountDownLatch
import java.util.concurrent.CyclicBarrier
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class RuntimeLoadTest {
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun loadsBindingsThroughWorker() = runBlocking {
        RuntimeWorker().use { worker ->
            worker.run {
                uniffiEnsureInitialized()
                Cancellation().use { cancellation ->
                    assertFalse(cancellation.isCancelled())
                    cancellation.cancel()
                    assertTrue(cancellation.isCancelled())
                }
                assertTrue(noticeFiles("{}").isEmpty())
                try {
                    checkPackage("{}")
                    fail("Expected an invalid package error")
                } catch (error: GeneratorException.Model) {
                    assertTrue(error.message.isNotEmpty())
                }
            }
            // Exercise the public model-opening path too, without downloading a model.
            try {
                worker.withGenerator(context, File(context.cacheDir, "missing-model")) {
                    fail("Expected an invalid model directory error")
                }
                fail("Expected open to fail")
            } catch (error: GeneratorException.Model) {
                assertTrue(error.message.isNotEmpty())
            }
        }
    }

    @Test
    fun syncLocksOnASongThroughTheBindings() = runBlocking {
        val rate = 44_100.0
        val song = syncSong(rate.toInt(), seconds = 30)
        val blockSize = 882
        val offsetMs = 11_000.0
        val clockMs = 5_000_000.0
        val block = FloatArray(blockSize)
        RuntimeWorker().use { worker ->
            worker.run {
                val buildStarted = System.nanoTime()
                SyncReference.fromPcm(song, rate).use { reference ->
                    val buildMs = (System.nanoTime() - buildStarted) / 1e6
                    assertEquals(30_000.0, reference.durationMs(), 0.001)
                    SyncSession(reference, rate).use { session ->
                        var sample = (offsetMs / 1_000 * rate).toInt()
                        var pushes = 0
                        var pushNs = 0L
                        var worstNs = 0L
                        var lock: SyncSnapshot? = null
                        while (lock == null && sample + blockSize <= song.size) {
                            song.copyInto(block, 0, sample, sample + blockSize)
                            val started = System.nanoTime()
                            val snapshot = session.push(block, clockMs + pushes * 20.0)
                            val cost = System.nanoTime() - started
                            pushNs += cost
                            worstNs = maxOf(worstNs, cost)
                            if (snapshot.isFinal) lock = snapshot
                            sample += blockSize
                            pushes += 1
                        }
                        println(
                            "Ensomi sync: reference ${buildMs / 0.5} ms/min; push mean " +
                                "${pushNs / pushes / 1_000} us, worst ${worstNs / 1_000} us, $pushes pushes",
                        )
                        val locked = requireNotNull(lock) { "The session never reached a final lock" }
                        assertEquals(SyncState.LOCKED, locked.state)
                        val error = locked.referenceTimeMs!! - (offsetMs + locked.captureTimeMs!! - clockMs)
                        assertEquals(0.0, error, 3.5)
                    }
                }
            }
            worker.run {
                SyncReference.fromPcm(FloatArray(4_096), 48_000.0).use { reference ->
                    SyncSession(reference, 48_000.0).use { session ->
                        val snapshot = session.push(FloatArray(4_800), 0.0, count = 2_400)
                        assertEquals(SyncState.LISTENING, snapshot.state)
                        assertFalse(snapshot.isFinal)
                        assertNull(snapshot.referenceTimeMs)
                    }
                    try {
                        SyncSession(reference, 1.5).close()
                        fail("Expected a refused sample rate")
                    } catch (error: SyncException.Refused) {
                        assertTrue(error.message.isNotEmpty())
                    }
                }
            }
        }
    }

    /** A tone that jumps at random every 120 ms and decays, so no passage repeats. */
    private fun syncSong(rate: Int, seconds: Int): FloatArray {
        var state = 1
        var phase = 0.0
        var hertz = 440.0
        val notes = rate * 12 / 100
        return FloatArray(rate * seconds) { index ->
            if (index % notes == 0) {
                state = state * 1_664_525 + 1_013_904_223
                hertz = 220.0 * Math.pow(2.0, (state ushr 8) / 16_777_216.0 * 2.0)
            }
            phase += 2 * Math.PI * hertz / rate
            (Math.sin(phase) * 0.3 * (1.0 - (index % notes).toDouble() / notes)).toFloat()
        }
    }

    @Test
    fun workerReusesNativeStackAndPropagatesErrors() = runBlocking {
        val caller = Thread.currentThread()
        val worker = RuntimeWorker()
        val thread = worker.use {
            val thread = worker.run {
                assertNotSame(caller, Thread.currentThread())
                val libc = Native.load("c", Pthreads::class.java)
                // sizeof(pthread_attr_t) in the arm64 Android NDK headers is 56.
                Memory(56).use { attributes ->
                    assertEquals(0, libc.pthread_getattr_np(libc.pthread_self(), attributes))
                    try {
                        val size = LongByReference()
                        assertEquals(0, libc.pthread_attr_getstacksize(attributes, size))
                        println("Ensomi worker stack=${size.value}; page=${Os.sysconf(OsConstants._SC_PAGESIZE)}")
                        assertTrue("Native stack: ${size.value}", size.value >= (8L shl 20))
                    } finally {
                        assertEquals(0, libc.pthread_attr_destroy(attributes))
                    }
                }
                val before = Thread.currentThread()
                delay(1)
                assertSame(before, Thread.currentThread())
                before
            }
            assertSame(thread, worker.run { Thread.currentThread() })
            val failure = IllegalStateException("generation failed")
            try {
                worker.run { throw failure }
                fail("Expected the worker's error")
            } catch (error: IllegalStateException) {
                assertSame(failure, error)
            }
            thread
        }
        thread.join(5_000)
        assertFalse("Worker thread survived close", thread.isAlive)
        worker.close()
        try {
            worker.run { fail("Closed worker ran work") }
            fail("Expected closed worker rejection")
        } catch (error: IllegalStateException) {
            assertEquals("RuntimeWorker is closed", error.message)
        }
    }

    @Test
    fun cancellationDuringOpenClosesResourceBeforeCompletion() = runBlocking {
        RuntimeWorker().use { worker ->
            val entered = CompletableDeferred<Unit>()
            val release = CountDownLatch(1)
            var resource: Cancellation? = null
            val job = launch {
                worker.withResource({
                    Cancellation().also {
                        resource = it
                        entered.complete(Unit)
                        check(release.await(10, TimeUnit.SECONDS))
                    }
                }) { fail("Cancelled acquisition entered the body") }
            }
            entered.await()
            try {
                job.cancel()
                assertFalse("Cancellation must await native work", job.isCompleted)
            } finally {
                release.countDown()
            }
            job.join()
            assertTrue(job.isCancelled)
            try {
                resource!!.isCancelled()
                fail("Resource was not closed")
            } catch (_: IllegalStateException) {
                // UniFFI rejects calls after close.
            }
        }
    }

    @Test
    fun resourceClosesOnFailure() = runBlocking {
        RuntimeWorker().use { worker ->
            var resource: Cancellation? = null
            val failure = IllegalStateException("body failed")
            try {
                worker.withResource({ Cancellation().also { resource = it } }) { throw failure }
                fail("Expected failure")
            } catch (error: IllegalStateException) {
                assertSame(failure, error)
            }
            try {
                resource!!.isCancelled()
                fail("Resource was not closed")
            } catch (_: IllegalStateException) {
            }
        }
    }

    @Test
    fun copiesBundledTablesOnceAndIncludesNotices() {
        val directory = File(bundledResamplerDirectory(context))
        assertTrue(directory.canonicalPath.startsWith(context.noBackupFilesDir.canonicalPath + "/"))
        val names = context.assets.list("resampler")!!
        assertTrue(names.isNotEmpty())
        for (name in names) {
            val expected = context.assets.open("resampler/$name").use { it.readBytes() }
            assertArrayEquals(name, expected, File(directory, name).readBytes())
        }
        val table = File(directory, names.first())
        assertTrue(table.setLastModified(1_000))
        assertEquals(directory.absolutePath, bundledResamplerDirectory(context))
        assertEquals(1_000L, table.lastModified())
        for (name in onnxRuntimeLicenseAssets) {
            assertTrue(context.assets.open(name).use { it.readBytes().isNotEmpty() })
        }
    }

    @Test
    fun tableCopySurvivesRaceAndRefreshesChangedContent() {
        val root = Files.createTempDirectory(context.cacheDir.toPath(), "table-race").toFile()
        val threads = Executors.newFixedThreadPool(8)
        try {
            val contents = "first table".toByteArray()
            val version = hash(contents)
            val copies = AtomicInteger()
            val barrier = CyclicBarrier(8)
            val calls = (1..8).map {
                threads.submit<File> {
                    barrier.await(10, TimeUnit.SECONDS)
                    val directory = installResamplerTables(root, version, listOf("table")) {
                        copies.incrementAndGet()
                        contents.inputStream()
                    }
                    assertArrayEquals(contents, File(directory, "table").readBytes())
                    directory
                }
            }
            val directories = calls.map { it.get(10, TimeUnit.SECONDS) }
            assertEquals(1, directories.toSet().size)
            assertEquals(1, copies.get())
            val changed = "updated table".toByteArray()
            val updated = installResamplerTables(root, hash(changed), listOf("table")) { changed.inputStream() }
            assertNotEquals(directories.first(), updated)
            assertArrayEquals(changed, File(updated, "table").readBytes())
            // A live generator can keep using its previous version.
            assertArrayEquals(contents, File(directories.first(), "table").readBytes())
        } finally {
            threads.shutdownNow()
            threads.awaitTermination(10, TimeUnit.SECONDS)
            root.deleteRecursively()
        }
    }

    @Test
    fun failedTableCopyCanBeRetried() {
        val root = Files.createTempDirectory(context.cacheDir.toPath(), "table-retry").toFile()
        try {
            try {
                installResamplerTables(root, "test-version", listOf("one", "two")) { name ->
                    if (name == "two") throw IOException("interrupted copy")
                    name.byteInputStream()
                }
                fail("Expected copy failure")
            } catch (_: IOException) {
            }
            assertFalse(File(root, "resampler-test-version").exists())
            val directory = installResamplerTables(root, "test-version", listOf("one", "two")) { it.byteInputStream() }
            assertEquals("one", File(directory, "one").readText())
            assertEquals("two", File(directory, "two").readText())
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun floatPcmPreservesBitsInLittleEndianBytes() {
        val bits = intArrayOf(0, Int.MIN_VALUE, 0x3f800000, 0xbf000000.toInt(), 1, 0x7f800000, 0x7fc01234)
        val pcm = FloatArray(bits.size) { Float.fromBits(bits[it]) }
        val bytes = pcm.toF32le()
        assertEquals(bits.size * 4, bytes.size)
        val decoded = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        for (sample in bits) assertEquals(sample, decoded.int)
        assertArrayEquals(byteArrayOf(0, 0, 0x80.toByte(), 0x3f), floatArrayOf(1f).toF32le())
        assertTrue(floatArrayOf().toF32le().isEmpty())
    }

    private fun hash(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    interface Pthreads : Library {
        fun pthread_self(): Long
        fun pthread_getattr_np(thread: Long, attributes: Pointer): Int
        fun pthread_attr_getstacksize(attributes: Pointer, size: LongByReference): Int
        fun pthread_attr_destroy(attributes: Pointer): Int
    }
}
