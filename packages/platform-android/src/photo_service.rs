use async_trait::async_trait;
use fotos_shared::{
    PhotoService, ImportOptions, PhotoAlbum, PhotoSource,
    PlatformError, PlatformResult,
};
use fotos_core::{
    PhotoInfo, PhotoIndex, Thumbnailer, ThumbnailSpec,
    extract_raw_preview, compute_hash, read_metadata,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::path::{Path, PathBuf};
use std::fs;

/// Android photo service implementation
///
/// On Android, photos are accessed through MediaStore API:
/// - MediaStore.Images.Media for photos
/// - Scoped storage restrictions (Android 10+)
///
/// The Kotlin layer handles:
/// - Querying MediaStore for photos
/// - Loading images via ContentResolver
/// - Passing data to Rust for processing
pub struct AndroidPhotoService {
    cancel_flag: Arc<AtomicBool>,
}

impl AndroidPhotoService {
    pub fn new() -> Self {
        Self {
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel_flag.clone()
    }

    /// Process a single photo from MediaStore
    /// Called from Kotlin after loading photo data
    pub fn process_photo(
        &self,
        photo_data: &[u8],
        content_uri: &str,
        db_path: &str,
        thumbnail_dir: &str,
    ) -> Result<(), String> {
        let index = PhotoIndex::open(db_path.to_string())
            .map_err(|e| e.to_string())?;

        // Create temp file for processing
        let hash_preview = format!("{:x}", md5_hash(photo_data));
        let temp_path = format!("{}/temp_{}.jpg", thumbnail_dir, hash_preview);
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

        // Store with content:// URI as path
        index.insert(content_uri.to_string(), hash, metadata)
            .map_err(|e| e.to_string())?;

        // Clean up temp file
        let _ = fs::remove_file(&temp_path);

        Ok(())
    }
}

/// Simple hash for temp filename
fn md5_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0;
    for byte in data.iter().take(1024) {
        hash = hash.wrapping_mul(31).wrapping_add(*byte as u64);
    }
    hash
}

impl Default for AndroidPhotoService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PhotoService for AndroidPhotoService {
    async fn list_photos(&self, db_path: &str) -> PlatformResult<Vec<PhotoInfo>> {
        let index = PhotoIndex::open(db_path.to_string())
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        index.list()
            .map_err(|e| PlatformError::Platform(e.to_string()))
    }

    async fn import_photos(
        &self,
        options: ImportOptions,
        _db_path: &str,
        _thumbnail_dir: &str,
    ) -> PlatformResult<u32> {
        self.cancel_flag.store(false, Ordering::SeqCst);

        match &options.source {
            Some(PhotoSource::CameraRoll) |
            Some(PhotoSource::Screenshots) |
            Some(PhotoSource::Favorites) |
            Some(PhotoSource::Album(_)) => {
                // Android photo import uses MediaStore query from Kotlin
                // Then calls process_photo() for each result
                Err(PlatformError::NotSupported(
                    "Android photo import is initiated from native UI".to_string()
                ))
            }
            Some(PhotoSource::Path(_)) => {
                // Direct path access limited by Scoped Storage
                Err(PlatformError::NotSupported(
                    "Path-based import restricted on Android 10+".to_string()
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
                // Don't delete thumbnails for content:// URIs directly
                if !photo.path.starts_with("content://") {
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
        // On Android, deleting MediaStore items requires
        // ContentResolver.delete() with createDeleteRequest() on Android 11+
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
        if Path::new(db_path).exists() {
            fs::remove_file(db_path)
                .map_err(|e| PlatformError::Platform(e.to_string()))?;
        }

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
            // Skip content:// URIs - need to refetch from MediaStore
            if photo.path.starts_with("content://") {
                continue;
            }

            if thumbnailer.generate(Path::new(&photo.path), &thumb_spec).is_ok() {
                regenerated += 1;
            }
        }

        Ok(regenerated)
    }

    async fn get_albums(&self) -> PlatformResult<Vec<PhotoAlbum>> {
        // Album listing requires MediaStore.Images.Media.BUCKET_DISPLAY_NAME
        // queried from Kotlin
        Ok(Vec::new())
    }

    async fn get_raw_preview(
        &self,
        path: &str,
        cache_dir: &str,
    ) -> PlatformResult<String> {
        fs::create_dir_all(cache_dir)
            .map_err(|e| PlatformError::Platform(e.to_string()))?;

        if path.starts_with("content://") {
            return Err(PlatformError::NotSupported(
                "RAW preview for content:// URIs must be fetched via ContentResolver".to_string()
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
