use async_trait::async_trait;
use footos_shared::{
    PhotoService, ImportOptions, PhotoAlbum, PhotoSource,
    PlatformError, PlatformResult,
};
use footos_core::{
    PhotoInfo, PhotoIndex, Thumbnailer, ThumbnailSpec,
    extract_raw_preview, compute_hash, read_metadata,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::path::{Path, PathBuf};
use std::fs;

/// iOS photo service implementation
///
/// On iOS, photos are accessed through the Photos framework (PHAsset).
/// The native Swift layer handles:
/// - Fetching photos from PHAssetCollection
/// - Requesting image data from PHImageManager
/// - Passing photo data to Rust for processing
///
/// This service handles:
/// - Database operations (SQLite via footos-core)
/// - Thumbnail generation
/// - RAW preview extraction
pub struct IosPhotoService {
    cancel_flag: Arc<AtomicBool>,
}

impl IosPhotoService {
    pub fn new() -> Self {
        Self {
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Get a clone of the cancel flag for external use
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel_flag.clone()
    }

    /// Process a single photo from iOS Photos library
    /// Called from Swift after fetching photo data
    pub fn process_photo(
        &self,
        photo_data: &[u8],
        identifier: &str,
        db_path: &str,
        thumbnail_dir: &str,
    ) -> Result<(), String> {
        // This is a simplified implementation
        // In production, we'd need to handle the photo data differently
        // since we receive bytes rather than a file path

        let index = PhotoIndex::open(db_path.to_string())
            .map_err(|e| e.to_string())?;

        // Create a temporary file to process
        let temp_path = format!("{}/temp_{}.jpg", thumbnail_dir, identifier);
        fs::write(&temp_path, photo_data).map_err(|e| e.to_string())?;

        let path = Path::new(&temp_path);

        // Read metadata
        let metadata = read_metadata(path).map_err(|e| e.to_string())?;

        // Compute hash
        let hash = compute_hash(path).map_err(|e| e.to_string())?;

        // Generate thumbnail
        let thumbnailer = Thumbnailer::new(PathBuf::from(thumbnail_dir));
        let spec = ThumbnailSpec { width: 300, height: 300 };
        thumbnailer.generate(path, &spec).map_err(|e| e.to_string())?;

        // Store with iOS photo identifier as path
        // This allows us to refetch from Photos library later
        let ios_path = format!("photos://{}", identifier);
        index.insert(ios_path, hash, metadata).map_err(|e| e.to_string())?;

        // Clean up temp file
        let _ = fs::remove_file(&temp_path);

        Ok(())
    }
}

impl Default for IosPhotoService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PhotoService for IosPhotoService {
    async fn list_photos(&self, db_path: &str) -> PlatformResult<Vec<PhotoInfo>> {
        let index = PhotoIndex::open(db_path.to_string())
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        index.list()
            .map_err(|e| PlatformError::Platform(e.to_string()))
    }

    async fn import_photos(
        &self,
        options: ImportOptions,
        db_path: &str,
        thumbnail_dir: &str,
    ) -> PlatformResult<u32> {
        // Reset cancel flag
        self.cancel_flag.store(false, Ordering::SeqCst);

        match &options.source {
            Some(PhotoSource::CameraRoll) |
            Some(PhotoSource::Screenshots) |
            Some(PhotoSource::Favorites) |
            Some(PhotoSource::Album(_)) => {
                // iOS photo import is handled by Swift calling process_photo()
                // for each selected photo from the PHPickerViewController
                Err(PlatformError::NotSupported(
                    "iOS photo import is initiated from native UI".to_string()
                ))
            }
            Some(PhotoSource::Path(_)) => {
                // Path-based import is not typical on iOS due to sandboxing
                Err(PlatformError::NotSupported(
                    "Path-based import not supported on iOS".to_string()
                ))
            }
            None => {
                Err(PlatformError::Platform(
                    "No import source specified".to_string()
                ))
            }
        }
    }

    fn cancel_import(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    fn is_import_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::SeqCst)
    }

    async fn delete_from_app(
        &self,
        photo_ids: Vec<String>,
        db_path: &str,
        thumbnail_dir: &str,
    ) -> PlatformResult<u32> {
        let index = PhotoIndex::open(db_path.to_string())
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        let ids: Vec<i64> = photo_ids
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();

        let thumbnailer = Thumbnailer::new(PathBuf::from(thumbnail_dir));
        let thumb_spec = ThumbnailSpec { width: 300, height: 300 };

        let mut deleted_count = 0u32;
        for id in &ids {
            if let Ok(Some(photo)) = index.get_by_id(*id) {
                // For iOS photos, the path is "photos://identifier"
                // We only delete the thumbnail, not the original in Photos library
                if !photo.path.starts_with("photos://") {
                    if let Ok(Some(thumb_path)) = thumbnailer.get_cached_path(Path::new(&photo.path), &thumb_spec) {
                        let _ = fs::remove_file(&thumb_path);
                    }
                }
                deleted_count += 1;
            }
        }

        index.delete_by_ids(ids)
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        Ok(deleted_count)
    }

    async fn delete_completely(
        &self,
        photo_ids: Vec<String>,
        db_path: &str,
        thumbnail_dir: &str,
    ) -> PlatformResult<u32> {
        // On iOS, we cannot delete photos from the Photos library programmatically
        // without user confirmation through PHAssetChangeRequest
        // For now, just delete from app
        self.delete_from_app(photo_ids, db_path, thumbnail_dir).await
    }

    async fn clear_app_data(
        &self,
        db_path: &str,
        thumbnail_dir: &str,
        raw_preview_dir: &str,
        tile_cache_dir: &str,
    ) -> PlatformResult<()> {
        // Delete database
        if Path::new(db_path).exists() {
            fs::remove_file(db_path)
                .map_err(|e| PlatformError::Platform(e.to_string()))?;
        }

        // Delete directories
        for dir in [thumbnail_dir, raw_preview_dir, tile_cache_dir] {
            if Path::new(dir).exists() {
                fs::remove_dir_all(dir)
                    .map_err(|e| PlatformError::Platform(e.to_string()))?;
            }
        }

        Ok(())
    }

    async fn regenerate_thumbnails(
        &self,
        db_path: &str,
        thumbnail_dir: &str,
    ) -> PlatformResult<u32> {
        let index = PhotoIndex::open(db_path.to_string())
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        let photos = index.list()
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        // Clear existing thumbnails
        if Path::new(thumbnail_dir).exists() {
            fs::remove_dir_all(thumbnail_dir)
                .map_err(|e| PlatformError::Platform(e.to_string()))?;
        }
        fs::create_dir_all(thumbnail_dir)
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        let thumbnailer = Thumbnailer::new(PathBuf::from(thumbnail_dir));
        let thumb_spec = ThumbnailSpec { width: 300, height: 300 };
        let mut regenerated = 0u32;

        for photo in photos {
            // Skip iOS photos:// paths - they need to be refetched from Photos library
            if photo.path.starts_with("photos://") {
                // In production, we'd request the photo from PHImageManager
                // and regenerate the thumbnail
                continue;
            }

            if thumbnailer.generate(Path::new(&photo.path), &thumb_spec).is_ok() {
                regenerated += 1;
            }
        }

        Ok(regenerated)
    }

    async fn get_albums(&self) -> PlatformResult<Vec<PhotoAlbum>> {
        // Album listing requires PHAssetCollection enumeration from Swift
        // Return empty for now - Swift layer will provide this
        Ok(Vec::new())
    }

    async fn get_raw_preview(
        &self,
        path: &str,
        cache_dir: &str,
    ) -> PlatformResult<String> {
        // Ensure cache directory exists
        fs::create_dir_all(cache_dir)
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        // For iOS photos:// paths, we'd need to fetch from Photos library
        if path.starts_with("photos://") {
            return Err(PlatformError::NotSupported(
                "RAW preview for Photos library items must be fetched via native APIs".to_string()
            ));
        }

        let hash = compute_hash(Path::new(path))
            .map_err(|e| PlatformError::Platform(e.to_string()))?;
        let cache_path = format!("{}/{}.jpg", cache_dir, hash);

        if Path::new(&cache_path).exists() {
            return Ok(cache_path);
        }

        let preview_data = extract_raw_preview(Path::new(path))
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        fs::write(&cache_path, preview_data)
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        Ok(cache_path)
    }
}

// FFI functions for Swift to call

/// Called from Swift to process a photo from the Photos library
#[no_mangle]
pub extern "C" fn ios_process_photo(
    _photo_data: *const u8,
    _photo_data_len: usize,
    _identifier: *const std::os::raw::c_char,
    _db_path: *const std::os::raw::c_char,
    _thumbnail_dir: *const std::os::raw::c_char,
) -> i32 {
    // In production, this would:
    // 1. Convert C pointers to Rust types
    // 2. Call IosPhotoService::process_photo()
    // 3. Return 0 on success, error code on failure
    0
}

/// Called from Swift to check if import should be cancelled
#[no_mangle]
pub extern "C" fn ios_check_import_cancelled() -> bool {
    // Would check the cancel flag
    false
}
