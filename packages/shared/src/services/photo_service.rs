use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use fotos_core::PhotoInfo;
use crate::error::PlatformResult;

/// Photo source type for mobile platforms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PhotoSource {
    /// Camera roll / All photos
    CameraRoll,
    /// Screenshots album
    Screenshots,
    /// Favorites album
    Favorites,
    /// Custom album by name
    Album(String),
    /// Filesystem path (desktop)
    Path(String),
}

/// Photo album information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhotoAlbum {
    pub id: String,
    pub name: String,
    pub photo_count: u32,
    pub thumbnail_path: Option<String>,
}

/// Options for photo import
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportOptions {
    /// Source to import from
    pub source: Option<PhotoSource>,
    /// Whether to include subfolders (desktop)
    pub recursive: bool,
    /// File extensions to include
    pub extensions: Option<Vec<String>>,
    /// Maximum number of photos to import (for testing)
    pub limit: Option<u32>,
}

/// Import progress information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportProgress {
    pub current: u32,
    pub total: u32,
    pub current_file: String,
    pub phase: ImportPhase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImportPhase {
    Scanning,
    Processing,
    GeneratingThumbnails,
    Complete,
    Cancelled,
}

/// Platform-agnostic photo service trait
#[async_trait]
pub trait PhotoService: Send + Sync {
    /// Get all photos from the index
    async fn list_photos(&self, db_path: &str) -> PlatformResult<Vec<PhotoInfo>>;

    /// Import photos from a source
    async fn import_photos(
        &self,
        options: ImportOptions,
        db_path: &str,
        thumbnail_dir: &str,
    ) -> PlatformResult<u32>;

    /// Cancel ongoing import
    fn cancel_import(&self);

    /// Check if import was cancelled
    fn is_import_cancelled(&self) -> bool;

    /// Delete photos from app (removes from index and thumbnails)
    async fn delete_from_app(
        &self,
        photo_ids: Vec<String>,
        db_path: &str,
        thumbnail_dir: &str,
    ) -> PlatformResult<u32>;

    /// Delete photos completely (including original files)
    async fn delete_completely(
        &self,
        photo_ids: Vec<String>,
        db_path: &str,
        thumbnail_dir: &str,
    ) -> PlatformResult<u32>;

    /// Clear all app data (database, thumbnails, caches)
    async fn clear_app_data(
        &self,
        db_path: &str,
        thumbnail_dir: &str,
        raw_preview_dir: &str,
        tile_cache_dir: &str,
    ) -> PlatformResult<()>;

    /// Regenerate thumbnails for all photos
    async fn regenerate_thumbnails(
        &self,
        db_path: &str,
        thumbnail_dir: &str,
    ) -> PlatformResult<u32>;

    /// Get available photo albums (mobile only)
    async fn get_albums(&self) -> PlatformResult<Vec<PhotoAlbum>>;

    /// Get RAW preview image
    async fn get_raw_preview(
        &self,
        path: &str,
        cache_dir: &str,
    ) -> PlatformResult<String>;
}
