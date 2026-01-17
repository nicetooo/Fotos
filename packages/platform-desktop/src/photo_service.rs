use async_trait::async_trait;
use footos_shared::{
    PhotoService, ImportOptions, PhotoAlbum, PhotoSource,
    PlatformError, PlatformResult,
};
use footos_core::{
    PhotoInfo, PhotoIndex, Thumbnailer, ThumbnailSpec,
    scan_photos, extract_raw_preview, compute_hash, read_metadata,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::path::{Path, PathBuf};
use std::fs;

/// Desktop photo service implementation
pub struct DesktopPhotoService {
    cancel_flag: Arc<AtomicBool>,
}

impl DesktopPhotoService {
    pub fn new() -> Self {
        Self {
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Get a clone of the cancel flag for external use
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel_flag.clone()
    }
}

impl Default for DesktopPhotoService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PhotoService for DesktopPhotoService {
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

        // Get the source path
        let source_path = match &options.source {
            Some(PhotoSource::Path(path)) => path.clone(),
            _ => return Err(PlatformError::NotSupported(
                "Desktop only supports path-based imports".to_string()
            )),
        };

        // Scan for photos
        let photo_paths = scan_photos(Path::new(&source_path))
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        if photo_paths.is_empty() {
            return Ok(0);
        }

        // Open database
        let index = PhotoIndex::open(db_path.to_string())
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        // Create thumbnailer
        let thumbnailer = Thumbnailer::new(PathBuf::from(thumbnail_dir));
        let thumb_spec = ThumbnailSpec { width: 300, height: 300 };

        let mut imported = 0u32;

        for path in photo_paths {
            // Check for cancellation
            if self.cancel_flag.load(Ordering::SeqCst) {
                return Err(PlatformError::Cancelled);
            }

            let path_str = match path.to_str() {
                Some(s) => s.to_string(),
                None => continue,
            };

            // Skip if already imported
            if index.get_by_path(path_str.clone()).ok().flatten().is_some() {
                continue;
            }

            // Read metadata
            let metadata = match read_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Failed to read metadata for {}: {}", path_str, e);
                    continue;
                }
            };

            // Compute hash
            let hash = match compute_hash(&path) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("Failed to compute hash for {}: {}", path_str, e);
                    continue;
                }
            };

            // Generate thumbnail
            if let Err(e) = thumbnailer.generate(&path, &thumb_spec) {
                eprintln!("Failed to generate thumbnail for {}: {}", path_str, e);
                continue;
            }

            // Insert into database
            if let Err(e) = index.insert(path_str.clone(), hash, metadata) {
                eprintln!("Failed to insert photo {}: {}", path_str, e);
                continue;
            }

            imported += 1;

            // Apply limit if specified
            if let Some(limit) = options.limit {
                if imported >= limit {
                    break;
                }
            }
        }

        Ok(imported)
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

        // Convert string IDs to i64
        let ids: Vec<i64> = photo_ids
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();

        let thumbnailer = Thumbnailer::new(PathBuf::from(thumbnail_dir));
        let thumb_spec = ThumbnailSpec { width: 300, height: 300 };

        // Get photos before deletion to find thumbnail paths
        let mut deleted_count = 0u32;
        for id in &ids {
            if let Ok(Some(photo)) = index.get_by_id(*id) {
                // Delete thumbnail
                if let Ok(Some(thumb_path)) = thumbnailer.get_cached_path(Path::new(&photo.path), &thumb_spec) {
                    let _ = fs::remove_file(&thumb_path);
                }
                deleted_count += 1;
            }
        }

        // Delete from database
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
        let index = PhotoIndex::open(db_path.to_string())
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        // Convert string IDs to i64
        let ids: Vec<i64> = photo_ids
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();

        let thumbnailer = Thumbnailer::new(PathBuf::from(thumbnail_dir));
        let thumb_spec = ThumbnailSpec { width: 300, height: 300 };

        let mut deleted_count = 0u32;
        for id in &ids {
            if let Ok(Some(photo)) = index.get_by_id(*id) {
                // Delete thumbnail
                if let Ok(Some(thumb_path)) = thumbnailer.get_cached_path(Path::new(&photo.path), &thumb_spec) {
                    let _ = fs::remove_file(&thumb_path);
                }

                // Delete original file
                if let Err(e) = fs::remove_file(&photo.path) {
                    eprintln!("Failed to delete original file {}: {}", photo.path, e);
                }

                deleted_count += 1;
            }
        }

        // Delete from database
        index.delete_by_ids(ids)
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        Ok(deleted_count)
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
            if thumbnailer.generate(Path::new(&photo.path), &thumb_spec).is_ok() {
                regenerated += 1;
            }
        }

        Ok(regenerated)
    }

    async fn get_albums(&self) -> PlatformResult<Vec<PhotoAlbum>> {
        // Desktop doesn't have native photo albums
        Err(PlatformError::NotSupported(
            "Photo albums are not available on desktop".to_string()
        ))
    }

    async fn get_raw_preview(
        &self,
        path: &str,
        cache_dir: &str,
    ) -> PlatformResult<String> {
        // Ensure cache directory exists
        fs::create_dir_all(cache_dir)
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        // Generate cache filename from path hash
        let hash = compute_hash(Path::new(path))
            .map_err(|e| PlatformError::Platform(e.to_string()))?;
        let cache_path = format!("{}/{}.jpg", cache_dir, hash);

        // Return cached version if exists
        if Path::new(&cache_path).exists() {
            return Ok(cache_path);
        }

        // Extract preview and save to cache
        let preview_data = extract_raw_preview(Path::new(path))
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        fs::write(&cache_path, preview_data)
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        Ok(cache_path)
    }
}
