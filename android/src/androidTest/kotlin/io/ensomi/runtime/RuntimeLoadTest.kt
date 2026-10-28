package io.ensomi.runtime

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.concurrent.ExecutionException
import java.util.concurrent.TimeUnit
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class RuntimeLoadTest {
    @Test
    fun loadsBindingsAndCallsRust() {
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

    @Test
    fun copiesBundledTablesOnceAndIncludesNotices() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
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
    fun generationThreadReturnsValuesAndErrors() {
        val caller = Thread.currentThread()
        assertEquals(42, onGenerationThread {
            assertNotSame(caller, Thread.currentThread())
            42
        }.get(10, TimeUnit.SECONDS))
        val failure = IllegalStateException("generation failed")
        try {
            onGenerationThread<Int> { throw failure }.get(10, TimeUnit.SECONDS)
            fail("Expected the worker's error")
        } catch (error: ExecutionException) {
            assertSame(failure, error.cause)
        }
    }
}
