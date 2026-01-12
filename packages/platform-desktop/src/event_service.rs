use async_trait::async_trait;
use fotos_shared::{
    EventService, AppEvent,
    PlatformError, PlatformResult,
};
use tauri::{AppHandle, Emitter, Runtime};
use std::sync::Arc;

/// Tauri-based event service for desktop
pub struct TauriEventService<R: Runtime> {
    app_handle: Arc<AppHandle<R>>,
}

impl<R: Runtime> TauriEventService<R> {
    pub fn new(app_handle: AppHandle<R>) -> Self {
        Self {
            app_handle: Arc::new(app_handle),
        }
    }
}

#[async_trait]
impl<R: Runtime + Send + Sync + 'static> EventService for TauriEventService<R> {
    async fn emit(&self, event: AppEvent) -> PlatformResult<()> {
        let event_name = match &event {
            AppEvent::ImportProgress(_) => "import-progress",
            AppEvent::ImportCancelled => "import-cancelled",
            AppEvent::ThumbnailProgress { .. } => "thumbnail-progress",
            AppEvent::Error { .. } => "app-error",
        };

        self.app_handle
            .emit(event_name, &event)
            .map_err(|e| PlatformError::Platform(e.to_string()))
    }

    async fn emit_raw(&self, event_name: &str, payload: &str) -> PlatformResult<()> {
        self.app_handle
            .emit(event_name, payload)
            .map_err(|e| PlatformError::Platform(e.to_string()))
    }
}
