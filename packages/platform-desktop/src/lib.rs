mod file_service;
mod photo_service;
mod permission_service;
mod event_service;

pub use file_service::DesktopFileService;
pub use photo_service::DesktopPhotoService;
pub use permission_service::DesktopPermissionService;
pub use event_service::TauriEventService;

use footos_shared::{PlatformContext, PlatformType};
use std::sync::Arc;

/// Desktop platform services container
pub struct DesktopPlatform {
    pub context: PlatformContext,
    pub file_service: Arc<DesktopFileService>,
    pub photo_service: Arc<DesktopPhotoService>,
    pub permission_service: Arc<DesktopPermissionService>,
}

impl DesktopPlatform {
    pub fn new(app_data_dir: String, cache_dir: String) -> Self {
        let context = PlatformContext::new(
            PlatformType::Desktop,
            app_data_dir,
            cache_dir,
        );

        Self {
            file_service: Arc::new(DesktopFileService::new()),
            photo_service: Arc::new(DesktopPhotoService::new()),
            permission_service: Arc::new(DesktopPermissionService::new()),
            context,
        }
    }
}
