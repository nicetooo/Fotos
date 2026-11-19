package app.footos.viewer

import android.content.Context
import android.net.Uri
import java.io.File

/**
 * A motion photo (Pixel "Top Shot", Samsung "Motion photo") is a JPEG with an
 * MP4 clip appended to it. This pulls the clip out into the cache so it can
 * be played.
 */
object MotionPhoto {
    private const val MAX_BYTES = 64L * 1024 * 1024

    /** Path of the clip inside `path`, or null if there is none. Blocking. */
    fun clipFor(context: Context, path: String): String? {
        return try {
            val dir = File(context.cacheDir, "motion_clips").apply { mkdirs() }
            val out = File(dir, "${path.hashCode().toUInt().toString(16)}.mp4")
            if (out.length() > 1024) return out.absolutePath

            val bytes = if (path.startsWith("content://")) {
                context.contentResolver.openInputStream(Uri.parse(path))?.use { it.readBytes() }
            } else {
                File(path).takeIf { it.length() in 1..MAX_BYTES }?.readBytes()
            } ?: return null
            if (bytes.size > MAX_BYTES) return null

            // An MP4 starts with a box whose type is "ftyp", four bytes after
            // its own length. The clip is at the end: take the last one.
            val start = lastIndexOf(bytes, byteArrayOf(0x66, 0x74, 0x79, 0x70)) - 4
            if (start <= 0) return null
            out.outputStream().use { it.write(bytes, start, bytes.size - start) }
            // Keep the cache small: the clips are quick to make again.
            dir.listFiles()?.sortedByDescending { it.lastModified() }?.drop(20)?.forEach { it.delete() }
            out.absolutePath
        } catch (e: Throwable) {
            null
        }
    }

    private fun lastIndexOf(data: ByteArray, needle: ByteArray): Int {
        var i = data.size - needle.size
        while (i >= 0) {
            var match = true
            for (j in needle.indices) {
                if (data[i + j] != needle[j]) { match = false; break }
            }
            if (match) return i
            i--
        }
        return -1
    }
}
