use async_trait::async_trait;
use footos_shared::{
    PermissionService, Permission, PermissionStatus,
    PlatformError, PlatformResult,
};

/// Desktop permission service - mostly no-op since desktop doesn't need runtime permissions
pub struct DesktopPermissionService;

impl DesktopPermissionService {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DesktopPermissionService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PermissionService for DesktopPermissionService {
    async fn check_permission(&self, _permission: Permission) -> PlatformResult<PermissionStatus> {
        // Desktop platforms generally don't require explicit permission requests
        // The OS handles file access permissions through the file picker dialogs
        Ok(PermissionStatus::Granted)
    }

    async fn request_permission(&self, _permission: Permission) -> PlatformResult<PermissionStatus> {
        // No explicit permission request needed on desktop
        Ok(PermissionStatus::Granted)
    }

    async fn open_app_settings(&self) -> PlatformResult<()> {
        // On desktop, we can open system preferences
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .args(["x-apple.systempreferences:com.apple.preference.security?Privacy"])
                .spawn()
                .map_err(|e| PlatformError::Platform(e.to_string()))?;
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("start")
                .args(["ms-settings:privacy"])
                .spawn()
                .map_err(|e| PlatformError::Platform(e.to_string()))?;
        }
        #[cfg(target_os = "linux")]
        {
            // Linux doesn't have a unified settings app
            return Err(PlatformError::NotSupported(
                "Opening app settings is not supported on Linux".to_string()
            ));
        }
        Ok(())
    }
}
