package io.ensomi.runtime

import android.content.Context
import java.io.File
import java.io.InputStream
import java.io.RandomAccessFile

/** ONNX Runtime's MIT licence and third-party notices, for the app's acknowledgements. */
val onnxRuntimeLicenseAssets: List<String> = listOf(
    "licenses/onnxruntime/LICENSE",
    "licenses/onnxruntime/ThirdPartyNotices.txt",
)

internal fun bundledResamplerDirectory(context: Context): String {
    val assets = context.assets
    val version = assets.open("resampler.sha256").bufferedReader().use { it.readText().trim() }
    return installResamplerTables(
        File(context.noBackupFilesDir, "ensomi-runtime"),
        version,
        checkNotNull(assets.list("resampler")).toList(),
    ) { name -> assets.open("resampler/$name") }.absolutePath
}

/**
 * Serialize threads and processes, then atomically publish a complete, content-keyed directory.
 * Interrupted copies are retried. Keep older versions: an open generator may still read them.
 */
@Synchronized
internal fun installResamplerTables(
    root: File,
    version: String,
    names: List<String>,
    open: (String) -> InputStream,
): File {
    check(root.isDirectory || root.mkdirs()) { "Cannot create $root" }
    RandomAccessFile(File(root, ".tables.lock"), "rw").use { lockFile ->
        lockFile.channel.lock().use {
            val directory = File(root, "resampler-$version")
            if (File(directory, ".complete").isFile) return directory
            val staging = File(root, ".resampler-$version.tmp")
            staging.deleteRecursively()
            check(staging.mkdir()) { "Cannot create $staging" }
            try {
                for (name in names) {
                    open(name).use { input ->
                        File(staging, name).outputStream().use { output -> input.copyTo(output) }
                    }
                }
                File(staging, ".complete").writeText(version)
                // Also recover an incomplete directory left by the original, in-place copier.
                check(!directory.exists() || directory.deleteRecursively()) { "Cannot replace $directory" }
                check(staging.renameTo(directory)) { "Cannot publish $directory" }
                return directory
            } finally {
                staging.deleteRecursively()
            }
        }
    }
}
