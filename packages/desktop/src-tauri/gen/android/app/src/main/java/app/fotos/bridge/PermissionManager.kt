package app.fotos.bridge

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
     * Get required permissions based on API level
     */
    fun getRequiredPermissions(): Array<String> {
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            // Android 13+ (API 33+): need READ_MEDIA_IMAGES + ACCESS_MEDIA_LOCATION for GPS
            arrayOf(
                Manifest.permission.READ_MEDIA_IMAGES,
                Manifest.permission.ACCESS_MEDIA_LOCATION
            )
        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            // Android 10-12 (API 29-32): need READ_EXTERNAL_STORAGE + ACCESS_MEDIA_LOCATION for GPS
            arrayOf(
                Manifest.permission.READ_EXTERNAL_STORAGE,
                Manifest.permission.ACCESS_MEDIA_LOCATION
            )
        } else {
            // Android 8-9 (API 26-28): only READ_EXTERNAL_STORAGE needed
            arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
        }
    }

    /**
     * Check if app has photo library access permission
     */
    fun hasMediaPermission(): Boolean {
        val permissions = getRequiredPermissions()
        return permissions.all { permission ->
            ContextCompat.checkSelfPermission(context, permission) ==
                PackageManager.PERMISSION_GRANTED
        }
    }

    /**
     * Register permission request launcher (must be called in Activity.onCreate)
     */
    fun registerLauncher(activity: AppCompatActivity) {
        permissionLauncher = activity.registerForActivityResult(
            ActivityResultContracts.RequestMultiplePermissions()
        ) { permissions ->
            val allGranted = permissions.values.all { it }
            Log.d(TAG, "Permission result: allGranted=$allGranted, permissions=$permissions")
            permissionCallback?.invoke(allGranted)
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
