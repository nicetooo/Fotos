package app.footos

import android.os.Bundle
import android.util.Log
import android.webkit.WebView
import androidx.activity.result.contract.ActivityResultContracts
import app.footos.bridge.FootosWebViewBridge
import app.footos.bridge.MediaStoreSync
import app.footos.bridge.PermissionManager
import app.footos.bridge.PhotoPickerManager

/**
 * Main Activity for 足迹相册 (footos) Android app.
 * Extends Tauri's TauriActivity for WebView integration.
 */
class MainActivity : TauriActivity() {

    companion object {
        private const val TAG = "MainActivity"
    }

    private var bridge: FootosWebViewBridge? = null
    private lateinit var permissionManager: PermissionManager
    private lateinit var photoPickerManager: PhotoPickerManager
    private lateinit var mediaStoreSync: MediaStoreSync

    // Permission request launcher
    private val permissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { permissions ->
        val allGranted = permissions.values.all { it }
        Log.d(TAG, "Permission result: allGranted=$allGranted")
    }

    // Photo Picker launcher (Android 13+)
    private val photoPickerLauncher = registerForActivityResult(
        ActivityResultContracts.PickMultipleVisualMedia(100)
    ) { uris ->
        Log.d(TAG, "Photo Picker returned ${uris.size} items")
    }

    // GetContent fallback launcher (Android 8-12)
    private val getContentLauncher = registerForActivityResult(
        ActivityResultContracts.GetMultipleContents()
    ) { uris ->
        Log.d(TAG, "GetContent returned ${uris.size} items")
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Log.d(TAG, "MainActivity onCreate")

        // Initialize managers
        permissionManager = PermissionManager(this)
        permissionManager.registerLauncher(this)

        photoPickerManager = PhotoPickerManager(this)
        photoPickerManager.registerLaunchers(this)

        mediaStoreSync = MediaStoreSync(this)
    }

    override fun onWebViewCreate(webView: WebView) {
        super.onWebViewCreate(webView)
        Log.d(TAG, "WebView created, attaching bridge")

        // Create bridge with all managers
        bridge = FootosWebViewBridge(
            activity = this,
            webView = webView,
            permissionManager = permissionManager,
            photoPickerManager = photoPickerManager,
            mediaStoreSync = mediaStoreSync
        )

        // Add JavaScript interface
        webView.addJavascriptInterface(bridge!!, "AndroidWebViewInterface")

        // Notify frontend that bridge is ready
        webView.post {
            bridge?.notifyBridgeReady()
        }

        Log.d(TAG, "Bridge attached and ready")
    }

    override fun onDestroy() {
        super.onDestroy()

        // Cleanup resources
        bridge?.cleanup()
        if (::photoPickerManager.isInitialized) {
            photoPickerManager.shutdown()
        }
        if (::mediaStoreSync.isInitialized) {
            mediaStoreSync.shutdown()
        }

        Log.d(TAG, "MainActivity destroyed")
    }
}
