package app.footos.bridge

import android.os.Handler
import android.os.Looper
import android.util.Log
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.appcompat.app.AppCompatActivity
import org.json.JSONObject

/**
 * WebView JavaScript Interface for 足迹相册 (footos) app.
 *
 * This bridge enables communication between the frontend (Svelte/TypeScript) and
 * native Android code (Kotlin). It mirrors the iOS webkit.messageHandlers pattern.
 *
 * Communication patterns:
 * - JS -> Kotlin: window.AndroidWebViewInterface.postMessage(JSON)
 * - Kotlin -> JS: webView.evaluateJavascript() dispatching CustomEvents
 * - Kotlin -> JS: Calling global functions like window.__handleAndroidImportPaths()
 */
class FootosWebViewBridge(
    private val activity: AppCompatActivity,
    private val webView: WebView,
    private val permissionManager: PermissionManager,
    private val photoPickerManager: PhotoPickerManager,
    private val mediaStoreSync: MediaStoreSync
) {

    companion object {
        private const val TAG = "FootosWebViewBridge"
    }

    private val mainHandler = Handler(Looper.getMainLooper())
    private var dbPath: String = ""
    private var thumbDir: String = ""
    @Volatile private var importCancelled = false

    /**
     * Main entry point for JavaScript messages
     * Called from JS: window.AndroidWebViewInterface.postMessage(JSON.stringify({...}))
     */
    @JavascriptInterface
    fun postMessage(message: String) {
        Log.d(TAG, "Received message: $message")

        try {
            val json = JSONObject(message)
            val command = json.optString("command", "")
            val msgDbPath = json.optString("dbPath", "")
            val msgThumbDir = json.optString("thumbDir", "")

            // Update paths if provided
            if (msgDbPath.isNotEmpty()) dbPath = msgDbPath
            if (msgThumbDir.isNotEmpty()) thumbDir = msgThumbDir

            mainHandler.post {
                handleCommand(command)
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to parse message: ${e.message}")
            sendError("Invalid message format")
        }
    }

    /**
     * Handle commands from JavaScript
     */
    private fun handleCommand(command: String) {
        Log.d(TAG, "Handling command: $command")

        when (command) {
            "requestImport" -> handleRequestImport()
            "syncMediaStore" -> handleSyncMediaStore()
            "checkPermission" -> handleCheckPermission()
            "requestPermission" -> handleRequestPermission()
            "cancelImport" -> handleCancelImport()
            "getPhotoCount" -> handleGetPhotoCount()
            "clearData" -> handleClearData()
            else -> {
                Log.w(TAG, "Unknown command: $command")
                sendError("Unknown command: $command")
            }
        }
    }

    // ==================== Command Handlers ====================

    private fun handleRequestImport() {
        Log.d(TAG, "Requesting photo import")

        if (dbPath.isEmpty() || thumbDir.isEmpty()) {
            sendError("dbPath or thumbDir not set")
            return
        }

        importCancelled = false
        sendImportStarted()

        // Use Photo Picker if available (no permission needed)
        if (photoPickerManager.isPhotoPickerAvailable()) {
            photoPickerManager.launchPhotoPicker { paths ->
                if (paths.isEmpty()) {
                    sendImportComplete(0, 0, 0)
                } else {
                    processImportedPaths(paths)
                }
            }
        } else {
            // Fallback: check permission first
            permissionManager.checkAndRequestPermission { granted ->
                if (granted) {
                    photoPickerManager.launchPhotoPicker { paths ->
                        if (paths.isEmpty()) {
                            sendImportComplete(0, 0, 0)
                        } else {
                            processImportedPaths(paths)
                        }
                    }
                } else {
                    sendPermissionDenied("Photo access permission denied")
                }
            }
        }
    }

    private fun handleSyncMediaStore() {
        Log.d(TAG, "Requesting MediaStore sync")

        if (dbPath.isEmpty() || thumbDir.isEmpty()) {
            sendError("dbPath or thumbDir not set")
            return
        }

        // MediaStore sync requires permission (including ACCESS_MEDIA_LOCATION for GPS)
        permissionManager.checkAndRequestPermission { granted ->
            if (granted) {
                importCancelled = false
                sendImportStarted()
                sendPermissionGranted("mediastore")

                mediaStoreSync.syncAllPhotos(
                    dbPath,
                    thumbDir,
                    progressCallback = { current, total ->
                        sendImportProgress(0, 0, 0, current, total, "Syncing...")
                    },
                    completionCallback = { result ->
                        if (result.paths.isNotEmpty()) {
                            // Pass paths to frontend for processing via Tauri
                            callGlobalImportFunction(result.paths)
                        } else {
                            sendImportComplete(result.success, result.failure, result.duplicates)
                        }
                    }
                )
            } else {
                sendPermissionDenied("MediaStore access requires permission")
            }
        }
    }

    private fun handleCheckPermission() {
        val hasPermission = permissionManager.hasMediaPermission()
        sendPermissionStatus(hasPermission)
    }

    private fun handleRequestPermission() {
        permissionManager.checkAndRequestPermission { granted ->
            if (granted) {
                sendPermissionGranted("mediastore")
            } else {
                sendPermissionDenied("Permission denied by user")
            }
        }
    }

    private fun handleCancelImport() {
        importCancelled = true
        mediaStoreSync.cancelSync()
        Log.d(TAG, "Import cancellation requested")
    }

    private fun handleGetPhotoCount() {
        // Return 0 for now - photo count should be managed by frontend via Tauri
        sendEvent("android-photo-count", mapOf("count" to 0))
    }

    private fun handleClearData() {
        // Data clearing should be handled by frontend via Tauri commands
        sendEvent("android-data-cleared", mapOf("success" to true))
    }

    // ==================== Import Processing ====================

    private fun processImportedPaths(paths: List<String>) {
        Log.d(TAG, "Processing ${paths.size} imported paths")

        // Send progress updates and call global function for frontend to handle
        Thread {
            val total = paths.size

            for ((index, path) in paths.withIndex()) {
                if (importCancelled) {
                    Log.d(TAG, "Import cancelled at $index/$total")
                    break
                }

                // Send progress
                sendImportProgress(0, 0, 0, index + 1, total, java.io.File(path).name)
            }

            // Call global function for frontend to process the paths
            callGlobalImportFunction(paths)

            Log.d(TAG, "Paths sent to frontend for processing")
        }.start()
    }

    private fun callGlobalImportFunction(paths: List<String>) {
        val pathsJson = paths.joinToString(",") { "\"${it.replace("\"", "\\\"")}\"" }
        val dbPathEscaped = dbPath.replace("'", "\\'")
        val thumbDirEscaped = thumbDir.replace("'", "\\'")

        val script = """
            (function() {
                if (window.__handleAndroidImportPaths) {
                    window.__handleAndroidImportPaths([$pathsJson], '$dbPathEscaped', '$thumbDirEscaped');
                    return 'called';
                } else {
                    console.error('[足迹相册] __handleAndroidImportPaths not found');
                    return 'not_found';
                }
            })();
        """.trimIndent()

        mainHandler.post {
            webView.evaluateJavascript(script) { result ->
                Log.d(TAG, "Global import function result: $result")

                // Also send completion event
                sendImportComplete(paths.size, 0, 0)
            }
        }
    }

    // ==================== Event Dispatching ====================

    private fun sendEvent(eventName: String, detail: Map<String, Any>) {
        val detailJson = JSONObject(detail).toString()
        val script = "window.dispatchEvent(new CustomEvent('$eventName', { detail: $detailJson }));"

        mainHandler.post {
            webView.evaluateJavascript(script, null)
        }
    }

    private fun sendImportStarted() {
        sendEvent("android-import-started", emptyMap())
    }

    private fun sendImportProgress(
        success: Int,
        failure: Int,
        duplicates: Int,
        current: Int,
        total: Int,
        phase: String
    ) {
        sendEvent("android-import-progress", mapOf(
            "success" to success,
            "failure" to failure,
            "duplicates" to duplicates,
            "current" to current,
            "total" to total,
            "phase" to phase
        ))
    }

    private fun sendImportComplete(success: Int, failure: Int, duplicates: Int) {
        sendEvent("android-import-complete", mapOf(
            "success" to success,
            "failure" to failure,
            "duplicates" to duplicates
        ))
    }

    private fun sendPermissionGranted(type: String) {
        sendEvent("android-permission-granted", mapOf("type" to type))
    }

    private fun sendPermissionDenied(message: String) {
        sendEvent("android-permission-denied", mapOf("message" to message))
    }

    private fun sendPermissionStatus(hasAccess: Boolean) {
        sendEvent("android-permission-status", mapOf("hasAccess" to hasAccess))
    }

    private fun sendError(message: String) {
        sendEvent("android-error", mapOf("message" to message))
    }

    /**
     * Call this after WebView is loaded to notify frontend that bridge is ready
     */
    fun notifyBridgeReady() {
        val script = """
            console.log('[足迹相册] Android bridge initialized');
            window.__ANDROID_BRIDGE_READY__ = true;
            window.dispatchEvent(new CustomEvent('android-bridge-ready', { detail: {} }));
        """.trimIndent()

        mainHandler.post {
            webView.evaluateJavascript(script, null)
        }
    }

    /**
     * Clean up resources
     */
    fun cleanup() {
        photoPickerManager.shutdown()
        mediaStoreSync.shutdown()
    }
}
