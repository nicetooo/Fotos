use serde::{Deserialize, Serialize};

/// Represents the current platform type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlatformType {
    Desktop,
    Android,
    Ios,
}

impl PlatformType {
    /// Get the current platform at compile time
    #[cfg(all(not(target_os = "android"), not(target_os = "ios")))]
    pub fn current() -> Self {
        PlatformType::Desktop
    }

    #[cfg(target_os = "android")]
    pub fn current() -> Self {
        PlatformType::Android
    }

    #[cfg(target_os = "ios")]
    pub fn current() -> Self {
        PlatformType::Ios
    }

    pub fn is_mobile(&self) -> bool {
        matches!(self, PlatformType::Android | PlatformType::Ios)
    }

    pub fn is_desktop(&self) -> bool {
        matches!(self, PlatformType::Desktop)
    }
}

/// Platform capabilities - what features are available
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    /// Can access arbitrary filesystem paths
    pub arbitrary_file_access: bool,
    /// Has native photo library integration
    pub native_photo_library: bool,
    /// Requires runtime permission requests
    pub runtime_permissions: bool,
    /// Supports file/folder picker dialogs
    pub file_picker: bool,
    /// Supports revealing files in native file manager
    pub reveal_in_file_manager: bool,
    /// Supports external storage (SD cards, USB drives)
    pub external_storage: bool,
}

impl PlatformCapabilities {
    pub fn desktop() -> Self {
        Self {
            arbitrary_file_access: true,
            native_photo_library: false,
            runtime_permissions: false,
            file_picker: true,
            reveal_in_file_manager: true,
            external_storage: true,
        }
    }

    pub fn android() -> Self {
        Self {
            arbitrary_file_access: false, // Scoped storage
            native_photo_library: true,   // MediaStore
            runtime_permissions: true,
            file_picker: true,
            reveal_in_file_manager: false,
            external_storage: true,
        }
    }

    pub fn ios() -> Self {
        Self {
            arbitrary_file_access: false, // Sandboxed
            native_photo_library: true,   // PHAsset
            runtime_permissions: true,
            file_picker: true,
            reveal_in_file_manager: false,
            external_storage: false,
        }
    }

    pub fn for_platform(platform: PlatformType) -> Self {
        match platform {
            PlatformType::Desktop => Self::desktop(),
            PlatformType::Android => Self::android(),
            PlatformType::Ios => Self::ios(),
        }
    }
}

/// Platform context containing runtime information
#[derive(Debug, Clone)]
pub struct PlatformContext {
    pub platform_type: PlatformType,
    pub capabilities: PlatformCapabilities,
    pub app_data_dir: String,
    pub cache_dir: String,
}

impl PlatformContext {
    pub fn new(
        platform_type: PlatformType,
        app_data_dir: String,
        cache_dir: String,
    ) -> Self {
        Self {
            capabilities: PlatformCapabilities::for_platform(platform_type),
            platform_type,
            app_data_dir,
            cache_dir,
        }
    }

    /// Get the database path
    pub fn db_path(&self) -> String {
        format!("{}/fotos.db", self.app_data_dir)
    }

    /// Get the thumbnails directory
    pub fn thumbnails_dir(&self) -> String {
        format!("{}/thumbnails", self.app_data_dir)
    }

    /// Get the raw previews directory
    pub fn raw_previews_dir(&self) -> String {
        format!("{}/raw_previews", self.app_data_dir)
    }

    /// Get the map tiles cache directory
    pub fn map_tiles_dir(&self) -> String {
        format!("{}/map_tiles", self.cache_dir)
    }
}
