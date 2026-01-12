use async_trait::async_trait;
use fotos_shared::{
    PermissionService, Permission, PermissionStatus,
    PlatformError, PlatformResult,
};

/// Android permission service implementation
///
/// Android permissions required for photo access (API level dependent):
/// - Android 13+ (API 33+): READ_MEDIA_IMAGES, READ_MEDIA_VIDEO
/// - Android 10-12 (API 29-32): READ_EXTERNAL_STORAGE
/// - Android 9 and below: READ_EXTERNAL_STORAGE, WRITE_EXTERNAL_STORAGE
///
/// The Kotlin layer handles:
/// - Checking permissions via ContextCompat.checkSelfPermission()
/// - Requesting via ActivityCompat.requestPermissions()
/// - Handling ActivityResultContracts
pub struct AndroidPermissionService {
    // In production, would hold JNI references to call back to Android
}

impl AndroidPermissionService {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for AndroidPermissionService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PermissionService for AndroidPermissionService {
    async fn check_permission(&self, permission: Permission) -> PlatformResult<PermissionStatus> {
        // In production, this would call through JNI to:
        // ContextCompat.checkSelfPermission(context, permission)
        match permission {
            Permission::PhotoLibraryRead => {
                // Maps to READ_MEDIA_IMAGES (API 33+) or READ_EXTERNAL_STORAGE
                Ok(PermissionStatus::NotDetermined)
            }
            Permission::PhotoLibraryWrite => {
                // Maps to WRITE_EXTERNAL_STORAGE (older APIs) or MediaStore access
                Ok(PermissionStatus::NotDetermined)
            }
            Permission::StorageRead => {
                // READ_EXTERNAL_STORAGE
                Ok(PermissionStatus::NotDetermined)
            }
            Permission::StorageWrite => {
                // WRITE_EXTERNAL_STORAGE (limited on Android 10+)
                Ok(PermissionStatus::NotDetermined)
            }
            Permission::Camera => {
                // CAMERA permission
                Ok(PermissionStatus::NotDetermined)
            }
            Permission::Location => {
                // ACCESS_FINE_LOCATION or ACCESS_COARSE_LOCATION
                Ok(PermissionStatus::NotDetermined)
            }
        }
    }

    async fn request_permission(&self, permission: Permission) -> PlatformResult<PermissionStatus> {
        // Must be called through Android Activity for permission request UI
        match permission {
            Permission::PhotoLibraryRead |
            Permission::PhotoLibraryWrite |
            Permission::StorageRead |
            Permission::StorageWrite |
            Permission::Camera |
            Permission::Location => {
                Err(PlatformError::NotSupported(
                    "Permission request must be made through Android Activity".to_string()
                ))
            }
        }
    }

    async fn open_app_settings(&self) -> PlatformResult<()> {
        // Would launch Intent with Settings.ACTION_APPLICATION_DETAILS_SETTINGS
        Err(PlatformError::NotSupported(
            "Opening app settings must be done through Android Intent".to_string()
        ))
    }
}

/// Android permission constants matching the Manifest.permission strings
pub mod android_permissions {
    pub const READ_EXTERNAL_STORAGE: &str = "android.permission.READ_EXTERNAL_STORAGE";
    pub const WRITE_EXTERNAL_STORAGE: &str = "android.permission.WRITE_EXTERNAL_STORAGE";
    pub const READ_MEDIA_IMAGES: &str = "android.permission.READ_MEDIA_IMAGES";
    pub const READ_MEDIA_VIDEO: &str = "android.permission.READ_MEDIA_VIDEO";
    pub const CAMERA: &str = "android.permission.CAMERA";
    pub const ACCESS_FINE_LOCATION: &str = "android.permission.ACCESS_FINE_LOCATION";
    pub const ACCESS_COARSE_LOCATION: &str = "android.permission.ACCESS_COARSE_LOCATION";
}

/// Convert Permission enum to Android permission string(s)
pub fn get_android_permissions(permission: Permission, api_level: i32) -> Vec<&'static str> {
    match permission {
        Permission::PhotoLibraryRead => {
            if api_level >= 33 {
                vec![android_permissions::READ_MEDIA_IMAGES]
            } else {
                vec![android_permissions::READ_EXTERNAL_STORAGE]
            }
        }
        Permission::PhotoLibraryWrite => {
            if api_level >= 29 {
                // Android 10+ uses MediaStore, no write permission needed
                vec![]
            } else {
                vec![android_permissions::WRITE_EXTERNAL_STORAGE]
            }
        }
        Permission::StorageRead => {
            if api_level >= 33 {
                vec![android_permissions::READ_MEDIA_IMAGES, android_permissions::READ_MEDIA_VIDEO]
            } else {
                vec![android_permissions::READ_EXTERNAL_STORAGE]
            }
        }
        Permission::StorageWrite => {
            if api_level >= 29 {
                vec![]
            } else {
                vec![android_permissions::WRITE_EXTERNAL_STORAGE]
            }
        }
        Permission::Camera => {
            vec![android_permissions::CAMERA]
        }
        Permission::Location => {
            vec![android_permissions::ACCESS_FINE_LOCATION]
        }
    }
}
