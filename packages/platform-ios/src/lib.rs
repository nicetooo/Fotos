mod file_service;
mod photo_service;
mod permission_service;

pub use file_service::IosFileService;
pub use photo_service::IosPhotoService;
pub use permission_service::IosPermissionService;

use footos_shared::{PlatformContext, PlatformType};
use std::sync::Arc;

/// iOS platform services container
pub struct IosPlatform {
    pub context: PlatformContext,
    pub file_service: Arc<IosFileService>,
    pub photo_service: Arc<IosPhotoService>,
    pub permission_service: Arc<IosPermissionService>,
}

impl IosPlatform {
    pub fn new(app_data_dir: String, cache_dir: String) -> Self {
        let context = PlatformContext::new(
            PlatformType::Ios,
            app_data_dir,
            cache_dir,
        );

        Self {
            file_service: Arc::new(IosFileService::new()),
            photo_service: Arc::new(IosPhotoService::new()),
            permission_service: Arc::new(IosPermissionService::new()),
            context,
        }
    }
}
