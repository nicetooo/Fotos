use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::error::PlatformResult;
use crate::services::ImportProgress;

/// Event types that can be emitted
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum AppEvent {
    /// Import progress update
    ImportProgress(ImportProgress),
    /// Import was cancelled
    ImportCancelled,
    /// Thumbnail regeneration progress
    ThumbnailProgress { current: u32, total: u32 },
    /// Error occurred
    Error { message: String },
}

/// Platform-agnostic event service trait
#[async_trait]
pub trait EventService: Send + Sync {
    /// Emit an event to the frontend
    async fn emit(&self, event: AppEvent) -> PlatformResult<()>;

    /// Emit a named event with arbitrary payload
    async fn emit_raw(&self, event_name: &str, payload: &str) -> PlatformResult<()>;
}

/// No-op event service for testing or headless mode
pub struct NoOpEventService;

#[async_trait]
impl EventService for NoOpEventService {
    async fn emit(&self, _event: AppEvent) -> PlatformResult<()> {
        Ok(())
    }

    async fn emit_raw(&self, _event_name: &str, _payload: &str) -> PlatformResult<()> {
        Ok(())
    }
}
