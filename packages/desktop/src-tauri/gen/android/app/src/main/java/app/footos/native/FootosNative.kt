package app.footos.native

import android.util.Log

/**
 * JNI interface to Rust native library (footos_platform_android)
 *
 * This object provides the bridge between Kotlin and the Rust photo processing library.
 * All methods are called from Kotlin and executed in Rust via JNI.
 */
object FootosNative {

    private const val TAG = "FootosNative"
    private var initialized = false

    init {
        try {
            System.loadLibrary("footos_platform_android")
            Log.d(TAG, "Native library loaded successfully")
        } catch (e: UnsatisfiedLinkError) {
            Log.e(TAG, "Failed to load native library: ${e.message}")
        }
    }

    /**
     * Initialize the Rust library with app directories
     * @param filesDir App internal files directory (for database)
     * @param cacheDir App cache directory (for temp files)
     */
    fun initialize(filesDir: String, cacheDir: String) {
        if (initialized) {
            Log.d(TAG, "Already initialized")
            return
        }
        try {
            init(filesDir, cacheDir)
            initialized = true
            Log.d(TAG, "Rust library initialized: filesDir=$filesDir, cacheDir=$cacheDir")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to initialize Rust library: ${e.message}")
        }
    }

    /**
     * Process a single photo from byte array
     * @param photoData Raw photo data bytes
     * @param contentUri Original content:// URI (stored as path in database)
     * @param dbPath Path to SQLite database
     * @param thumbDir Directory for thumbnails
     * @return 0 on success, negative error code on failure
     */
    fun processPhotoData(
        photoData: ByteArray,
        contentUri: String,
        dbPath: String,
        thumbDir: String
    ): Int {
        return try {
            processPhoto(photoData, contentUri, dbPath, thumbDir)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to process photo: ${e.message}")
            -100
        }
    }

    /**
     * Check if import has been cancelled
     * @return true if import should be cancelled
     */
    fun checkImportCancelled(): Boolean {
        return try {
            isImportCancelled()
        } catch (e: Exception) {
            false
        }
    }

    /**
     * Request cancellation of current import
     */
    fun requestCancelImport() {
        try {
            cancelImport()
        } catch (e: Exception) {
            Log.e(TAG, "Failed to cancel import: ${e.message}")
        }
    }

    /**
     * Get total photo count in database
     * @param dbPath Path to SQLite database
     * @return Photo count, or negative error code
     */
    fun getTotalPhotoCount(dbPath: String): Long {
        return try {
            getPhotoCount(dbPath)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to get photo count: ${e.message}")
            -1
        }
    }

    /**
     * Clear all app data (database and cache directories)
     * @param dbPath Path to SQLite database
     * @param thumbDir Thumbnails directory
     * @param rawPreviewDir RAW preview directory
     * @param tileCacheDir Map tile cache directory
     * @return 0 on success, negative error code on failure
     */
    fun clearAllData(
        dbPath: String,
        thumbDir: String,
        rawPreviewDir: String,
        tileCacheDir: String
    ): Int {
        return try {
            clearAppData(dbPath, thumbDir, rawPreviewDir, tileCacheDir)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to clear app data: ${e.message}")
            -100
        }
    }

    // ==================== JNI External Methods ====================

    /**
     * Initialize the Rust library
     * Kotlin signature: FootosNative.init(filesDir: String, cacheDir: String)
     */
    private external fun init(filesDir: String, cacheDir: String)

    /**
     * Process a photo from byte array
     * Returns 0 on success, negative on error
     */
    private external fun processPhoto(
        photoData: ByteArray,
        contentUri: String,
        dbPath: String,
        thumbDir: String
    ): Int

    /**
     * Check if import is cancelled
     */
    private external fun isImportCancelled(): Boolean

    /**
     * Cancel current import
     */
    private external fun cancelImport()

    /**
     * Get photo count in database
     */
    private external fun getPhotoCount(dbPath: String): Long

    /**
     * Clear all app data
     */
    private external fun clearAppData(
        dbPath: String,
        thumbDir: String,
        rawPreviewDir: String,
        tileCacheDir: String
    ): Int
}
