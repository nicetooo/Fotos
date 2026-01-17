package app.footos.bridge

import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import java.io.File
import java.io.FileOutputStream
import java.util.concurrent.Executors

/**
 * Manages photo selection using SAF (Storage Access Framework) OpenDocument.
 *
 * NOTE: We intentionally do NOT use Photo Picker because it strips GPS/location
 * data for privacy. Instead, we use ACTION_OPEN_DOCUMENT which provides direct
 * file access and preserves all EXIF metadata including GPS coordinates.
 *
 * This requires READ_MEDIA_IMAGES + ACCESS_MEDIA_LOCATION permissions.
 */
class PhotoPickerManager(private val context: Context) {

    companion object {
        private const val TAG = "PhotoPickerManager"
    }

    private var photosCallback: ((List<String>) -> Unit)? = null
    private val mainHandler = Handler(Looper.getMainLooper())
    private val executor = Executors.newSingleThreadExecutor()

    // OpenDocument launcher (SAF) - preserves all metadata including GPS
    private var openDocumentLauncher: ActivityResultLauncher<Array<String>>? = null

    // GetMultipleContents fallback for older Android
    private var getContentLauncher: ActivityResultLauncher<String>? = null

    /**
     * Register activity result launchers (must be called in Activity.onCreate)
     */
    fun registerLaunchers(activity: AppCompatActivity) {
        // OpenDocument launcher (SAF) - preserves GPS metadata
        openDocumentLauncher = activity.registerForActivityResult(
            ActivityResultContracts.OpenMultipleDocuments()
        ) { uris ->
            handleSelectedUris(uris)
        }

        // Fallback content picker for older devices
        getContentLauncher = activity.registerForActivityResult(
            ActivityResultContracts.GetMultipleContents()
        ) { uris ->
            handleSelectedUris(uris)
        }

        Log.d(TAG, "Launchers registered")
    }

    /**
     * Launch document picker to select images (preserves GPS data)
     * @param callback Called with list of file paths (copied to app cache)
     */
    fun launchPhotoPicker(callback: (List<String>) -> Unit) {
        photosCallback = callback

        // Use OpenDocument (SAF) which preserves all metadata including GPS
        Log.d(TAG, "Using OpenDocument (SAF) - preserves GPS metadata")
        openDocumentLauncher?.launch(arrayOf("image/*")) ?: run {
            Log.e(TAG, "OpenDocument launcher not registered!")
            // Try fallback
            getContentLauncher?.launch("image/*") ?: run {
                Log.e(TAG, "GetContent launcher also not registered!")
                callback(emptyList())
            }
        }
    }

    /**
     * Check if Photo Picker is available (we don't use it, but keep for API compatibility)
     */
    fun isPhotoPickerAvailable(): Boolean {
        // Always return false since we use SAF instead of Photo Picker
        // Photo Picker strips GPS data which we need
        return false
    }

    /**
     * Handle URIs returned from picker
     * Copies selected photos to app cache for processing
     */
    private fun handleSelectedUris(uris: List<Uri>) {
        Log.d(TAG, "Selected ${uris.size} photos")

        if (uris.isEmpty()) {
            mainHandler.post {
                photosCallback?.invoke(emptyList())
                photosCallback = null
            }
            return
        }

        // Copy photos to app cache in background
        executor.execute {
            val paths = mutableListOf<String>()
            val cacheDir = File(context.cacheDir, "photo_imports")
            cacheDir.mkdirs()

            for ((index, uri) in uris.withIndex()) {
                try {
                    val fileName = "import_${System.currentTimeMillis()}_$index.jpg"
                    val destFile = File(cacheDir, fileName)

                    // Use setRequireOriginal to get GPS/location data (Android 10+)
                    val originalUri = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                        MediaStore.setRequireOriginal(uri)
                    } else {
                        uri
                    }

                    context.contentResolver.openInputStream(originalUri)?.use { input ->
                        FileOutputStream(destFile).use { output ->
                            input.copyTo(output)
                        }
                    }

                    paths.add(destFile.absolutePath)
                    Log.d(TAG, "Copied: $uri -> ${destFile.absolutePath}")
                } catch (e: Exception) {
                    Log.e(TAG, "Failed to copy $uri: ${e.message}")
                    // Fallback: try without setRequireOriginal
                    try {
                        val fileName = "import_${System.currentTimeMillis()}_${index}_fallback.jpg"
                        val destFile = File(cacheDir, fileName)
                        context.contentResolver.openInputStream(uri)?.use { input ->
                            FileOutputStream(destFile).use { output ->
                                input.copyTo(output)
                            }
                        }
                        paths.add(destFile.absolutePath)
                        Log.d(TAG, "Copied (fallback): $uri -> ${destFile.absolutePath}")
                    } catch (e2: Exception) {
                        Log.e(TAG, "Fallback copy also failed for $uri: ${e2.message}")
                    }
                }
            }

            Log.d(TAG, "Copied ${paths.size}/${uris.size} photos successfully")

            // Callback on main thread
            mainHandler.post {
                photosCallback?.invoke(paths)
                photosCallback = null
            }
        }
    }

    /**
     * Clean up cached import files
     */
    fun cleanupImportCache() {
        executor.execute {
            val cacheDir = File(context.cacheDir, "photo_imports")
            if (cacheDir.exists()) {
                cacheDir.deleteRecursively()
                Log.d(TAG, "Import cache cleaned")
            }
        }
    }

    /**
     * Shutdown executor when done
     */
    fun shutdown() {
        executor.shutdown()
    }
}
