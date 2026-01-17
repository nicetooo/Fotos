package app.footos.bridge

import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import java.io.File
import java.io.FileOutputStream
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Handles MediaStore queries for full photo library sync.
 *
 * Unlike Photo Picker, MediaStore sync preserves GPS/location data when:
 * 1. ACCESS_MEDIA_LOCATION permission is granted
 * 2. MediaStore.setRequireOriginal() is used when reading files
 */
class MediaStoreSync(private val context: Context) {

    companion object {
        private const val TAG = "MediaStoreSync"
    }

    data class MediaStorePhoto(
        val id: Long,
        val contentUri: Uri,
        val displayName: String,
        val dateTaken: Long,
        val size: Long
    )

    private val mainHandler = Handler(Looper.getMainLooper())
    private val executor = Executors.newSingleThreadExecutor()
    private val isCancelled = AtomicBoolean(false)

    /**
     * Query all photos from MediaStore
     */
    fun queryAllPhotos(): List<MediaStorePhoto> {
        val photos = mutableListOf<MediaStorePhoto>()

        val collection = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL)
        } else {
            MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        }

        val projection = arrayOf(
            MediaStore.Images.Media._ID,
            MediaStore.Images.Media.DISPLAY_NAME,
            MediaStore.Images.Media.DATE_TAKEN,
            MediaStore.Images.Media.SIZE
        )

        val sortOrder = "${MediaStore.Images.Media.DATE_TAKEN} DESC"

        try {
            context.contentResolver.query(
                collection,
                projection,
                null,
                null,
                sortOrder
            )?.use { cursor ->
                val idColumn = cursor.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
                val nameColumn = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.DISPLAY_NAME)
                val dateColumn = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.DATE_TAKEN)
                val sizeColumn = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.SIZE)

                while (cursor.moveToNext()) {
                    val id = cursor.getLong(idColumn)
                    val name = cursor.getString(nameColumn) ?: "unknown"
                    val dateTaken = cursor.getLong(dateColumn)
                    val size = cursor.getLong(sizeColumn)

                    val contentUri = Uri.withAppendedPath(collection, id.toString())

                    photos.add(
                        MediaStorePhoto(
                            id = id,
                            contentUri = contentUri,
                            displayName = name,
                            dateTaken = dateTaken,
                            size = size
                        )
                    )
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to query MediaStore: ${e.message}")
        }

        Log.d(TAG, "Found ${photos.size} photos in MediaStore")
        return photos
    }

    /**
     * Sync all photos from MediaStore - exports to cache and returns paths
     * Uses setRequireOriginal() to preserve GPS/location data
     *
     * @param progressCallback Called with (current, total) progress
     * @param completionCallback Called with list of exported file paths when done
     */
    fun syncAllPhotos(
        dbPath: String,
        thumbDir: String,
        progressCallback: (Int, Int) -> Unit,
        completionCallback: (SyncResult) -> Unit
    ) {
        isCancelled.set(false)

        executor.execute {
            val photos = queryAllPhotos()
            val total = photos.size

            if (photos.isEmpty()) {
                mainHandler.post {
                    completionCallback(SyncResult(0, 0, 0))
                }
                return@execute
            }

            val cacheDir = File(context.cacheDir, "mediastore_sync")
            cacheDir.mkdirs()

            val exportedPaths = mutableListOf<String>()
            var failure = 0

            for ((index, photo) in photos.withIndex()) {
                if (isCancelled.get()) {
                    Log.d(TAG, "Sync cancelled at $index/$total")
                    break
                }

                try {
                    // Use setRequireOriginal to get GPS/location data (Android 10+)
                    val originalUri = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                        MediaStore.setRequireOriginal(photo.contentUri)
                    } else {
                        photo.contentUri
                    }

                    // Export photo to cache file
                    val extension = photo.displayName.substringAfterLast('.', "jpg")
                    val fileName = "sync_${photo.id}.$extension"
                    val destFile = File(cacheDir, fileName)

                    context.contentResolver.openInputStream(originalUri)?.use { input ->
                        FileOutputStream(destFile).use { output ->
                            input.copyTo(output)
                        }
                    }

                    exportedPaths.add(destFile.absolutePath)
                    Log.d(TAG, "Exported: ${photo.displayName} -> ${destFile.absolutePath}")

                    // Report progress
                    val currentIndex = index + 1
                    mainHandler.post {
                        progressCallback(currentIndex, total)
                    }

                } catch (e: Exception) {
                    Log.e(TAG, "Failed to export ${photo.displayName}: ${e.message}")
                    failure++
                }
            }

            Log.d(TAG, "Export complete: ${exportedPaths.size} files, $failure failures")

            // Return result - success count is number of exported files
            // The actual import will be done by frontend via Tauri
            mainHandler.post {
                completionCallback(SyncResult(exportedPaths.size, 0, failure, exportedPaths))
            }
        }
    }

    /**
     * Cancel ongoing sync
     */
    fun cancelSync() {
        isCancelled.set(true)
        Log.d(TAG, "Sync cancellation requested")
    }

    /**
     * Get count of photos in MediaStore
     */
    fun getPhotoCount(): Int {
        val collection = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL)
        } else {
            MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        }

        return try {
            context.contentResolver.query(
                collection,
                arrayOf(MediaStore.Images.Media._ID),
                null,
                null,
                null
            )?.use { cursor ->
                cursor.count
            } ?: 0
        } catch (e: Exception) {
            Log.e(TAG, "Failed to get photo count: ${e.message}")
            0
        }
    }

    /**
     * Shutdown executor when done
     */
    fun shutdown() {
        executor.shutdown()
    }

    data class SyncResult(
        val success: Int,
        val duplicates: Int,
        val failure: Int,
        val paths: List<String> = emptyList()
    )
}
