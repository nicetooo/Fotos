package app.footos.viewer

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.graphics.Matrix
import android.media.ExifInterface
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.LruCache
import android.widget.ImageView
import java.io.File
import java.util.concurrent.Executors
import kotlin.math.ceil
import kotlin.math.max

/**
 * Decodes local image files off the main thread, downsampled to what the
 * screen can show, with a small memory cache. No third-party image library.
 */
object ImageLoader {
    private val pool = Executors.newFixedThreadPool(3)
    private val main = Handler(Looper.getMainLooper())

    private val cache = object : LruCache<String, Bitmap>(
        (Runtime.getRuntime().maxMemory() / 1024 / 5).toInt()
    ) {
        override fun sizeOf(key: String, value: Bitmap): Int = value.byteCount / 1024
    }

    /**
     * Load `path` into `target`, at most about `maxSide` pixels on its long
     * side. A later call for the same view supersedes this one.
     * `onDone` runs on the main thread once this load has finished, with
     * whether a bitmap was set; it does not run if the load was superseded.
     */
    fun load(path: String, maxSide: Int, target: ImageView, onDone: ((Boolean) -> Unit)? = null) {
        val key = "$path@$maxSide"
        target.setTag(TAG_KEY, key)
        if (path.isEmpty()) {
            onDone?.invoke(false)
            return
        }
        cache.get(key)?.let {
            target.setImageBitmap(it)
            onDone?.invoke(true)
            return
        }
        pool.execute {
            val bitmap = try {
                decode(path, maxSide)
            } catch (e: Throwable) {
                null
            }
            if (bitmap != null) cache.put(key, bitmap)
            main.post {
                if (target.getTag(TAG_KEY) == key) {
                    if (bitmap != null) target.setImageBitmap(bitmap)
                    onDone?.invoke(bitmap != null)
                }
            }
        }
    }

    /** Forget what was asked for this view, so a pending load does not land on it. */
    fun cancel(target: ImageView) {
        target.setTag(TAG_KEY, null)
    }

    private const val TAG_KEY = 0x7f0f0001

    private fun sampleFor(width: Int, height: Int, maxSide: Int): Int =
        max(1, ceil(max(width, height) / maxSide.toFloat()).toInt())

    private fun decode(path: String, maxSide: Int): Bitmap? {
        val file = File(path)
        if (!file.exists()) return null

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            // Handles orientation and HEIF.
            return ImageDecoder.decodeBitmap(ImageDecoder.createSource(file)) { decoder, info, _ ->
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                decoder.setTargetSampleSize(sampleFor(info.size.width, info.size.height, maxSide))
            }
        }

        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(path, bounds)
        if (bounds.outWidth <= 0) return null
        val options = BitmapFactory.Options().apply {
            inSampleSize = Integer.highestOneBit(sampleFor(bounds.outWidth, bounds.outHeight, maxSide))
        }
        val raw = BitmapFactory.decodeFile(path, options) ?: return null
        val degrees = when (
            ExifInterface(path).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)
        ) {
            ExifInterface.ORIENTATION_ROTATE_90 -> 90f
            ExifInterface.ORIENTATION_ROTATE_180 -> 180f
            ExifInterface.ORIENTATION_ROTATE_270 -> 270f
            else -> 0f
        }
        if (degrees == 0f) return raw
        val matrix = Matrix().apply { postRotate(degrees) }
        return Bitmap.createBitmap(raw, 0, 0, raw.width, raw.height, matrix, true)
    }
}
