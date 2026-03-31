package org.cleanflash.android.container

import android.content.Context
import android.util.Log
import org.apache.commons.compress.archivers.tar.TarArchiveEntry
import org.apache.commons.compress.archivers.tar.TarArchiveInputStream
import com.github.luben.zstd.ZstdInputStream
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.InputStream

/**
 * Utility for extracting tar archives with zstd or plain compression.
 * Follows Winlator's TarCompressorUtils pattern.
 */
object TarExtractor {
    private const val TAG = "TarExtractor"
    private const val BUFFER_SIZE = 8192

    enum class Compression { NONE, ZSTD }

    /**
     * Extract a tar archive from APK assets to a destination directory.
     *
     * @param context Android context for asset access
     * @param assetName Name of the asset file (e.g., "rootfs.tar.zst")
     * @param destination Target directory to extract into
     * @param compression Compression format of the archive
     * @param onProgress Optional callback with (entriesExtracted, currentFileName)
     * @return true if extraction succeeded
     */
    fun extractFromAssets(
        context: Context,
        assetName: String,
        destination: File,
        compression: Compression = Compression.ZSTD,
        onProgress: ((Int, String) -> Unit)? = null
    ): Boolean {
        return try {
            context.assets.open(assetName).use { input ->
                extract(input, destination, compression, onProgress)
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to extract asset $assetName", e)
            false
        }
    }

    /**
     * Extract a tar archive from a file on disk.
     */
    fun extractFromFile(
        source: File,
        destination: File,
        compression: Compression = Compression.ZSTD,
        onProgress: ((Int, String) -> Unit)? = null
    ): Boolean {
        if (!source.isFile) return false
        return try {
            BufferedInputStream(FileInputStream(source), BUFFER_SIZE).use { input ->
                extract(input, destination, compression, onProgress)
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to extract file ${source.path}", e)
            false
        }
    }

    private fun extract(
        source: InputStream,
        destination: File,
        compression: Compression,
        onProgress: ((Int, String) -> Unit)?
    ): Boolean {
        val decompressedStream = when (compression) {
            Compression.ZSTD -> ZstdInputStream(BufferedInputStream(source, BUFFER_SIZE))
            Compression.NONE -> BufferedInputStream(source, BUFFER_SIZE)
        }

        var entriesExtracted = 0
        TarArchiveInputStream(decompressedStream).use { tar ->
            var entry: TarArchiveEntry? = tar.nextTarEntry
            while (entry != null) {
                if (!tar.canReadEntryData(entry)) {
                    entry = tar.nextTarEntry
                    continue
                }

                val outFile = File(destination, entry.name)

                // Path traversal protection: ensure extracted file stays under destination
                if (!outFile.canonicalPath.startsWith(destination.canonicalPath + File.separator) &&
                    outFile.canonicalPath != destination.canonicalPath) {
                    Log.w(TAG, "Skipping entry with path traversal: ${entry.name}")
                    entry = tar.nextTarEntry
                    continue
                }

                if (entry.isDirectory) {
                    outFile.mkdirs()
                } else if (entry.isSymbolicLink) {
                    outFile.parentFile?.mkdirs()
                    // Delete existing file/symlink before creating new one
                    outFile.delete()
                    try {
                        Os.symlink(entry.linkName, outFile.absolutePath)
                    } catch (e: Exception) {
                        // Fallback: on some Android versions Os may not be available
                        Runtime.getRuntime().exec(
                            arrayOf("ln", "-sf", entry.linkName, outFile.absolutePath)
                        ).waitFor()
                    }
                } else {
                    outFile.parentFile?.mkdirs()
                    BufferedOutputStream(FileOutputStream(outFile), BUFFER_SIZE).use { output ->
                        val buffer = ByteArray(BUFFER_SIZE)
                        var bytesRead: Int
                        while (tar.read(buffer).also { bytesRead = it } != -1) {
                            output.write(buffer, 0, bytesRead)
                        }
                    }
                }

                // Preserve executable permission bits
                if (entry.mode and 0b001_001_001 != 0) {
                    outFile.setExecutable(true, false)
                }
                if (entry.mode and 0b010_010_010 != 0) {
                    outFile.setWritable(true, false)
                }
                outFile.setReadable(true, false)

                entriesExtracted++
                onProgress?.invoke(entriesExtracted, entry.name)
                entry = tar.nextTarEntry
            }
        }

        Log.i(TAG, "Extracted $entriesExtracted entries to ${destination.path}")
        return true
    }

    /**
     * Android OS symlink helper.
     */
    private object Os {
        fun symlink(target: String, path: String) {
            android.system.Os.symlink(target, path)
        }
    }
}
