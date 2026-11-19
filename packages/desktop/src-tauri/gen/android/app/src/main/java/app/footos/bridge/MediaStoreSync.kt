package app.footos.bridge

import android.content.ContentUris
import android.content.Context
import android.graphics.Bitmap
import android.media.ExifInterface
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import android.util.Size
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.FileOutputStream
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.Callable
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Brings the system photo library (photos and videos) into the app's library
 * by reference.
 *
 * Nothing is copied. Each item becomes a row whose path is its MediaStore
 * `content://` URI, plus a small thumbnail file; the original is read from
 * the system when it is looked at. Position and capture time come from the
 * original's own metadata, which needs ACCESS_MEDIA_LOCATION and
 * `MediaStore.setRequireOriginal`.
 */
class MediaStoreSync(private val context: Context) {

    companion object {
        private const val TAG = "MediaStoreSync"
        private const val THUMB_SIDE = 384
        private const val BATCH = 48
        /** Older versions exported a copy of every original here. */
        const val LEGACY_EXPORT_DIR = "mediastore_sync"
    }

    private class Asset(
        val uri: Uri,
        val isVideo: Boolean,
        val id: Long,
        val name: String,
        val dateTakenMs: Long,
        val dateModifiedS: Long,
        val size: Long,
        val width: Int,
        val height: Int,
        val durationMs: Long,
    ) {
        val path: String get() = uri.toString()
    }

    private val mainHandler = Handler(Looper.getMainLooper())
    private val executor = Executors.newSingleThreadExecutor()
    private val workers = Executors.newFixedThreadPool(4)
    private val isCancelled = AtomicBoolean(false)
    private val running = AtomicBoolean(false)

    private fun query(video: Boolean): List<Asset> {
        val collection = when {
            video -> MediaStore.Video.Media.EXTERNAL_CONTENT_URI
            else -> MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        }
        val columns = mutableListOf(
            MediaStore.MediaColumns._ID,
            MediaStore.MediaColumns.DISPLAY_NAME,
            MediaStore.MediaColumns.DATE_MODIFIED,
            MediaStore.MediaColumns.SIZE,
            MediaStore.MediaColumns.WIDTH,
            MediaStore.MediaColumns.HEIGHT,
            // Same column name ("datetaken") in the image and video tables.
            MediaStore.Images.Media.DATE_TAKEN,
        )
        if (video) columns.add(MediaStore.Video.Media.DURATION)

        val out = ArrayList<Asset>()
        try {
            context.contentResolver.query(collection, columns.toTypedArray(), null, null, null)?.use { c ->
                val id = c.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
                val name = c.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)
                val modified = c.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_MODIFIED)
                val size = c.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)
                val width = c.getColumnIndexOrThrow(MediaStore.MediaColumns.WIDTH)
                val height = c.getColumnIndexOrThrow(MediaStore.MediaColumns.HEIGHT)
                val taken = c.getColumnIndexOrThrow(MediaStore.Images.Media.DATE_TAKEN)
                val duration = if (video) c.getColumnIndexOrThrow(MediaStore.Video.Media.DURATION) else -1
                while (c.moveToNext()) {
                    val rowId = c.getLong(id)
                    out.add(
                        Asset(
                            uri = ContentUris.withAppendedId(collection, rowId),
                            isVideo = video,
                            id = rowId,
                            name = c.getString(name) ?: "",
                            dateTakenMs = c.getLong(taken),
                            dateModifiedS = c.getLong(modified),
                            size = c.getLong(size),
                            width = c.getInt(width),
                            height = c.getInt(height),
                            durationMs = if (duration >= 0) c.getLong(duration) else 0,
                        )
                    )
                }
            }
        } catch (e: Exception) {
            // No permission for this kind of media, or the provider is unavailable.
            Log.w(TAG, "query(video=$video) failed: ${e.message}")
        }
        return out
    }

    /**
     * Describe every library item that is not in `known`, in batches, and
     * report the full list of current items at the end so the caller can
     * drop what is gone.
     *
     * @param onBatch (items, done, total) on the main thread; `items` are rows
     *   ready for the library, `done`/`total` count the new items.
     * @param onDone (currentPaths, failures) on the main thread. `currentPaths`
     *   is null when the run was cancelled or the library could not be read.
     */
    fun syncLibrary(
        thumbDir: String,
        known: Set<String>,
        onBatch: (JSONArray, Int, Int) -> Unit,
        onDone: (List<String>?, Int) -> Unit,
    ) {
        if (!running.compareAndSet(false, true)) {
            Log.d(TAG, "sync already running")
            return
        }
        isCancelled.set(false)
        executor.execute {
            var failures = 0
            var current: List<String>? = null
            try {
                val assets = query(video = false) + query(video = true)
                val fresh = assets.filter { it.path !in known }
                Log.d(TAG, "library: ${assets.size} items, ${fresh.size} new")
                File(thumbDir).mkdirs()

                var done = 0
                for (chunk in fresh.chunked(BATCH)) {
                    if (isCancelled.get()) break
                    val rows = workers.invokeAll(chunk.map { asset ->
                        Callable {
                            try {
                                describe(asset, thumbDir)
                            } catch (e: Throwable) {
                                Log.w(TAG, "describe ${asset.name} failed: ${e.message}")
                                null
                            }
                        }
                    }).map { it.get() }
                    val batch = JSONArray()
                    for (row in rows) {
                        if (row == null) failures++ else batch.put(row)
                    }
                    done += chunk.size
                    val doneNow = done
                    mainHandler.post { onBatch(batch, doneNow, fresh.size) }
                }
                // An empty answer with items known is far more likely a
                // permission that went away than an emptied library: do not
                // let the caller remove everything on that basis.
                if (!isCancelled.get() && (assets.isNotEmpty() || known.isEmpty())) {
                    current = assets.map { it.path }
                }
            } catch (e: Throwable) {
                Log.e(TAG, "sync failed: ${e.message}")
            } finally {
                running.set(false)
                val result = current
                val failed = failures
                mainHandler.post { onDone(result, failed) }
            }
        }
    }

    /** The row for one item, after making its thumbnail. */
    private fun describe(asset: Asset, thumbDir: String): JSONObject {
        val thumb = File(thumbDir, "ms_${if (asset.isVideo) "v" else "i"}_${asset.id}.jpg")
        if (thumb.length() < 512) {
            loadThumbnail(asset)?.let { bitmap ->
                FileOutputStream(thumb).use { bitmap.compress(Bitmap.CompressFormat.JPEG, 80, it) }
                bitmap.recycle()
            }
        }

        var lat: Double? = null
        var lon: Double? = null
        var taken: String? = null
        var width = asset.width
        var height = asset.height
        // The plain URI has the position stripped; the "original" form keeps it.
        val original = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            try { MediaStore.setRequireOriginal(asset.uri) } catch (e: Exception) { asset.uri }
        } else {
            asset.uri
        }
        if (asset.isVideo) {
            val retriever = MediaMetadataRetriever()
            try {
                retriever.setDataSource(context, original)
                retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_LOCATION)?.let { text ->
                    // ISO 6709, e.g. "+35.6895+139.6917/"
                    Regex("""([+-]\d+(?:\.\d+)?)([+-]\d+(?:\.\d+)?)""").find(text)?.let { m ->
                        lat = m.groupValues[1].toDoubleOrNull()
                        lon = m.groupValues[2].toDoubleOrNull()
                    }
                }
            } catch (e: Exception) {
                // No position for this one.
            } finally {
                try { retriever.release() } catch (e: Exception) { }
            }
        } else {
            try {
                (openOrNull(original) ?: openOrNull(asset.uri))?.use { stream ->
                    val exif = ExifInterface(stream)
                    val position = FloatArray(2)
                    if (exif.getLatLong(position)) {
                        lat = position[0].toDouble()
                        lon = position[1].toDouble()
                    }
                    // Camera clock reading, "2024:05:01 10:20:30".
                    exif.getAttribute(ExifInterface.TAG_DATETIME_ORIGINAL)?.let { raw ->
                        if (raw.length >= 19 && raw[0].isDigit() && !raw.startsWith("0000")) {
                            taken = raw.substring(0, 10).replace(':', '-') + raw.substring(10, 19)
                        }
                    }
                    if (width <= 0 || height <= 0) {
                        width = exif.getAttributeInt(ExifInterface.TAG_IMAGE_WIDTH, 0)
                        height = exif.getAttributeInt(ExifInterface.TAG_IMAGE_LENGTH, 0)
                    }
                }
            } catch (e: Exception) {
                // Unreadable metadata: the item still goes in, without a position.
            }
        }
        if (taken == null) {
            val ms = if (asset.dateTakenMs > 0) asset.dateTakenMs else asset.dateModifiedS * 1000
            if (ms > 0) {
                taken = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.US).format(Date(ms))
            }
        }
        if (lat == 0.0 && lon == 0.0) {
            lat = null
            lon = null
        }

        return JSONObject().apply {
            put("path", asset.path)
            if (thumb.length() >= 512) put("thumb_path", thumb.absolutePath)
            taken?.let { put("date_taken", it) }
            lat?.let { put("lat", it) }
            lon?.let { put("lon", it) }
            put("width", maxOf(0, width))
            put("height", maxOf(0, height))
            put("file_size", maxOf(0L, asset.size))
            put("media_type", if (asset.isVideo) "video" else if (isMotionPhotoName(asset.name)) "live_photo" else "photo")
            if (asset.isVideo && asset.durationMs > 0) put("duration_ms", asset.durationMs)
        }
    }

    private fun openOrNull(uri: Uri) = try {
        context.contentResolver.openInputStream(uri)
    } catch (e: Exception) {
        null
    }

    private fun loadThumbnail(asset: Asset): Bitmap? {
        return try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                context.contentResolver.loadThumbnail(asset.uri, Size(THUMB_SIDE, THUMB_SIDE), null)
            } else if (asset.isVideo) {
                @Suppress("DEPRECATION")
                MediaStore.Video.Thumbnails.getThumbnail(
                    context.contentResolver, asset.id, MediaStore.Video.Thumbnails.MINI_KIND, null
                )
            } else {
                @Suppress("DEPRECATION")
                MediaStore.Images.Thumbnails.getThumbnail(
                    context.contentResolver, asset.id, MediaStore.Images.Thumbnails.MINI_KIND, null
                )
            }
        } catch (e: Exception) {
            Log.w(TAG, "thumbnail for ${asset.name} failed: ${e.message}")
            null
        }
    }

    /** Pixel and other cameras mark a photo with an embedded clip in its name. */
    private fun isMotionPhotoName(name: String): Boolean =
        name.contains(".MP.", ignoreCase = true) || name.startsWith("MVIMG", ignoreCase = true)

    /** Remove the copies of originals an older version kept in the cache. */
    fun deleteLegacyExports() {
        executor.execute {
            val dir = File(context.cacheDir, LEGACY_EXPORT_DIR)
            if (dir.exists()) {
                val ok = dir.deleteRecursively()
                Log.d(TAG, "removed legacy export copies: $ok")
            }
        }
    }

    fun cancelSync() {
        isCancelled.set(true)
    }

    fun shutdown() {
        executor.shutdown()
        workers.shutdown()
    }
}
