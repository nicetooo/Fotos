package app.footos.bridge

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.util.Log
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat

/**
 * Manages Android runtime permissions for photo library access.
 *
 * Permission strategy by Android version:
 * - Android 13+ (API 33+): READ_MEDIA_IMAGES + ACCESS_MEDIA_LOCATION
 * - Android 10-12 (API 29-32): READ_EXTERNAL_STORAGE + ACCESS_MEDIA_LOCATION
 * - Android 8-9 (API 26-28): READ_EXTERNAL_STORAGE
 *
 * ACCESS_MEDIA_LOCATION is required on Android 10+ to read GPS data from photos.
 */
class PermissionManager(private val context: Context) {

    companion object {
        private const val TAG = "PermissionManager"
    }

    private var permissionCallback: ((Boolean) -> Unit)? = null
    private var permissionLauncher: ActivityResultLauncher<Array<String>>? = null

    /**
     * What is asked for. Photos and videos share one system prompt.
     * ACCESS_MEDIA_LOCATION lets positions be read from the originals; the
     * library is usable without it, only nothing lands on the map.
     */
    fun getRequiredPermissions(): Array<String> {
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            arrayOf(
                Manifest.permission.READ_MEDIA_IMAGES,
                Manifest.permission.READ_MEDIA_VIDEO,
                Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED,
                Manifest.permission.ACCESS_MEDIA_LOCATION
            )
        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            arrayOf(
                Manifest.permission.READ_MEDIA_IMAGES,
                Manifest.permission.READ_MEDIA_VIDEO,
                Manifest.permission.ACCESS_MEDIA_LOCATION
            )
        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            arrayOf(
                Manifest.permission.READ_EXTERNAL_STORAGE,
                Manifest.permission.ACCESS_MEDIA_LOCATION
            )
        } else {
            arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
        }
    }

    private fun granted(permission: String): Boolean =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

    /**
     * Something this version asks for has not been granted (videos and
     * "selected photos" were added after people had already granted photos).
     * True only once per install of that fact, so nobody is asked again and
     * again for something they turned down.
     */
    fun shouldAskForAdditions(): Boolean {
        if (getRequiredPermissions().all { granted(it) }) return false
        val prefs = context.getSharedPreferences("footos_permissions", Context.MODE_PRIVATE)
        val key = "asked_additions_v2"
        if (prefs.getBoolean(key, false)) return false
        prefs.edit().putBoolean(key, true).apply()
        return true
    }

    /** Ask for everything in `getRequiredPermissions`, whatever is already granted. */
    fun requestAll(callback: (Boolean) -> Unit) {
        permissionCallback = callback
        permissionLauncher?.launch(getRequiredPermissions()) ?: callback(hasMediaPermission())
    }

    /** The whole library can be read. */
    fun hasFullMediaAccess(): Boolean {
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            granted(Manifest.permission.READ_MEDIA_IMAGES)
        } else {
            granted(Manifest.permission.READ_EXTERNAL_STORAGE)
        }
    }

    /**
     * Some of the library can be read: all of it, or (Android 14+) the items
     * the user picked when choosing "selected photos".
     */
    fun hasMediaPermission(): Boolean {
        if (hasFullMediaAccess()) return true
        return Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE &&
            granted(Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED)
    }

    /**
     * Register permission request launcher (must be called in Activity.onCreate)
     */
    fun registerLauncher(activity: AppCompatActivity) {
        permissionLauncher = activity.registerForActivityResult(
            ActivityResultContracts.RequestMultiplePermissions()
        ) { permissions ->
            // Not "all granted": videos or positions may be refused while
            // photos are allowed, and that is enough to work with.
            val usable = hasMediaPermission()
            Log.d(TAG, "Permission result: usable=$usable, permissions=$permissions")
            permissionCallback?.invoke(usable)
            permissionCallback = null
        }
    }

    /**
     * Check and request permission if needed
     * @param callback Called with true if granted, false if denied
     */
    fun checkAndRequestPermission(callback: (Boolean) -> Unit) {
        if (hasMediaPermission()) {
            Log.d(TAG, "Permission already granted")
            callback(true)
            return
        }

        permissionCallback = callback
        val permissions = getRequiredPermissions()
        Log.d(TAG, "Requesting permissions: ${permissions.joinToString()}")

        permissionLauncher?.launch(permissions) ?: run {
            Log.e(TAG, "Permission launcher not registered! Call registerLauncher() in onCreate()")
            callback(false)
        }
    }

    /**
     * Check if should show permission rationale
     */
    fun shouldShowRationale(activity: AppCompatActivity): Boolean {
        val permissions = getRequiredPermissions()
        return permissions.any { permission ->
            activity.shouldShowRequestPermissionRationale(permission)
        }
    }

    /**
     * Open app settings page for manual permission grant
     */
    fun openAppSettings() {
        val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
            data = Uri.fromParts("package", context.packageName, null)
            flags = Intent.FLAG_ACTIVITY_NEW_TASK
        }
        context.startActivity(intent)
    }

    /**
     * Check if user permanently denied permission (clicked "Don't ask again")
     */
    fun isPermanentlyDenied(activity: AppCompatActivity): Boolean {
        return !hasMediaPermission() && !shouldShowRationale(activity)
    }

    /**
     * Get human-readable permission status
     */
    fun getPermissionStatusString(): String {
        return when {
            hasMediaPermission() -> "granted"
            else -> "denied"
        }
    }

    /**
     * Check if Photo Picker is available (no permission needed)
     * Photo Picker is available on Android 13+ or with Google Play services backport
     */
    fun isPhotoPickerAvailable(): Boolean {
        return Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
    }
}
