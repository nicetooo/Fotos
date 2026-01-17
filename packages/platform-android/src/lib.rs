mod file_service;
mod photo_service;
mod permission_service;
mod jni_bridge;

pub use file_service::AndroidFileService;
pub use photo_service::AndroidPhotoService;
pub use permission_service::AndroidPermissionService;

use footos_shared::{PlatformContext, PlatformType};
use std::sync::Arc;

/// Android platform services container
pub struct AndroidPlatform {
    pub context: PlatformContext,
    pub file_service: Arc<AndroidFileService>,
    pub photo_service: Arc<AndroidPhotoService>,
    pub permission_service: Arc<AndroidPermissionService>,
}

impl AndroidPlatform {
    pub fn new(app_data_dir: String, cache_dir: String) -> Self {
        let context = PlatformContext::new(
            PlatformType::Android,
            app_data_dir,
            cache_dir,
        );

        Self {
            file_service: Arc::new(AndroidFileService::new()),
            photo_service: Arc::new(AndroidPhotoService::new()),
            permission_service: Arc::new(AndroidPermissionService::new()),
            context,
        }
    }
}
