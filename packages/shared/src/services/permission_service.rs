use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::error::PlatformResult;

/// Permission types that may be required
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    /// Read photos from device library
    PhotoLibraryRead,
    /// Write/delete photos in device library
    PhotoLibraryWrite,
    /// Read from external storage
    StorageRead,
    /// Write to external storage
    StorageWrite,
    /// Access camera
    Camera,
    /// Access location (for geotagging)
    Location,
}

/// Permission status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionStatus {
    /// Permission granted
    Granted,
    /// Permission denied
    Denied,
    /// Permission not determined (never asked)
    NotDetermined,
    /// Permission restricted by system policy
    Restricted,
    /// Limited access (iOS 14+ partial photo access)
    Limited,
}

impl PermissionStatus {
    pub fn is_granted(&self) -> bool {
        matches!(self, PermissionStatus::Granted | PermissionStatus::Limited)
    }

    pub fn can_request(&self) -> bool {
        matches!(self, PermissionStatus::NotDetermined)
    }
}

/// Platform-agnostic permission service trait
#[async_trait]
pub trait PermissionService: Send + Sync {
    /// Check current status of a permission
    async fn check_permission(&self, permission: Permission) -> PlatformResult<PermissionStatus>;

    /// Request a permission from the user
    async fn request_permission(&self, permission: Permission) -> PlatformResult<PermissionStatus>;

    /// Check multiple permissions at once
    async fn check_permissions(
        &self,
        permissions: &[Permission],
    ) -> PlatformResult<Vec<(Permission, PermissionStatus)>> {
        let mut results = Vec::with_capacity(permissions.len());
        for &perm in permissions {
            let status = self.check_permission(perm).await?;
            results.push((perm, status));
        }
        Ok(results)
    }

    /// Request multiple permissions at once
    async fn request_permissions(
        &self,
        permissions: &[Permission],
    ) -> PlatformResult<Vec<(Permission, PermissionStatus)>> {
        let mut results = Vec::with_capacity(permissions.len());
        for &perm in permissions {
            let status = self.request_permission(perm).await?;
            results.push((perm, status));
        }
        Ok(results)
    }

    /// Open app settings (for when permission is denied)
    async fn open_app_settings(&self) -> PlatformResult<()>;
}
