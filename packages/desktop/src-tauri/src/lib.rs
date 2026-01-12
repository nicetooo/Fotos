use fotos_core::{PhotoCoreConfig, PhotoIndex, ImportResult, PhotoInfo};
use std::sync::atomic::{AtomicBool, Ordering};

// Global cancellation flag for import operations
static IMPORT_CANCELLED: AtomicBool = AtomicBool::new(false);

#[tauri::command]
fn cancel_import() {
    IMPORT_CANCELLED.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_core_version() -> String {
    fotos_core::get_version()
}

#[tauri::command]
async fn list_photos(db_path: String, thumb_dir: String) -> Result<Vec<PhotoInfo>, String> {
    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let index = PhotoIndex::open(db_path)
        .map_err(|e| e.to_string())?;
    
    let mut photos = index.list().map_err(|e| e.to_string())?;
    
    // Populate thumb_path and file_size
    let thumbnailer = fotos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
    let spec = fotos_core::ThumbnailSpec { width: 256, height: 256 };
    for photo in &mut photos {
        let source_path = std::path::Path::new(&photo.path);

        // Get thumbnail path
        match thumbnailer.get_cached_path(source_path, &spec) {
            Ok(Some(path)) => {
                photo.thumb_path = Some(path.to_string_lossy().to_string());
            }
            Ok(None) => {
                photo.thumb_path = None;
            }
            Err(_) => {
                photo.thumb_path = None;
            }
        }

        // Get file size
        if let Ok(metadata) = std::fs::metadata(source_path) {
            photo.file_size = metadata.len();
        }
    }

    Ok(photos)
}

#[tauri::command]
async fn import_photos(
    window: tauri::Window,
    root_path: String,
    db_path: String,
    thumb_dir: String,
) -> Result<ImportResult, String> {
    // Reset cancellation flag at start
    IMPORT_CANCELLED.store(false, Ordering::SeqCst);

    // Ensure parent directories exist
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;

    let index = PhotoIndex::open(db_path)
        .map_err(|e| e.to_string())?;

    let config = PhotoCoreConfig {
        thumbnail_dir: thumb_dir,
        thumbnail_size: 256,
    };

    let root_path_buf = std::path::Path::new(&root_path);

    // Support both single file and directory import
    let photos = if root_path_buf.is_file() {
        vec![root_path_buf.to_path_buf()]
    } else {
        fotos_core::scan_photos(root_path_buf).map_err(|e| e.to_string())?
    };
    let total = photos.len();

    let mut result = ImportResult::default();
    let mut skipped = 0usize;
    for (i, path) in photos.into_iter().enumerate() {
        // Check for cancellation
        if IMPORT_CANCELLED.load(Ordering::SeqCst) {
            println!("[Import] CANCELLED at {}/{}", i + 1, total);
            use tauri::Emitter;
            let _ = window.emit("import-cancelled", serde_json::json!({
                "current": i,
                "total": total,
                "success": result.success,
                "failure": result.failure
            }));
            break;
        }

        let path_str = path.to_string_lossy().to_string();

        // Skip if already imported (fast path - avoid expensive metadata/hash/thumbnail work)
        if let Ok(Some(_)) = index.get_by_path(path_str.clone()) {
            skipped += 1;
            // Emit progress but mark as skipped
            use tauri::Emitter;
            let _ = window.emit("import-progress", serde_json::json!({
                "current": i + 1,
                "total": total,
                "success": result.success,
                "failure": result.failure,
                "skipped": skipped,
                "last_path": path_str
            }));
            continue;
        }

        // Use a block to ensure we can handle errors per-file
        let file_result = (|| -> Result<(), String> {
            let metadata = fotos_core::read_metadata(&path).map_err(|e| e.to_string())?;
            let hash = fotos_core::compute_hash(&path).map_err(|e| e.to_string())?;
            // Thumbnail generation may fail if no EXIF thumbnail - that's OK, frontend uses original
            let _ = fotos_core::generate_thumbnail(&path, &config);
            index.insert(path_str.clone(), hash.clone(), metadata).map_err(|e| e.to_string())?;
            Ok(())
        })();

        match file_result {
            Ok(_) => {
                println!("[Import] SUCCESS: {}", path_str);
                result.success += 1;
            },
            Err(e) => {
                println!("[Import] FAILED: {} - {}", path_str, e);
                result.failure += 1;
            },
        }

        // Emit progress every photo
        use tauri::Emitter;
        let _ = window.emit("import-progress", serde_json::json!({
            "current": i + 1,
            "total": total,
            "success": result.success,
            "failure": result.failure,
            "last_path": path_str
        }));
    }

    Ok(result)
}

/// Delete result struct
#[derive(serde::Serialize, Default)]
struct DeleteResult {
    deleted_count: usize,
    deleted_paths: Vec<String>,
    errors: Vec<String>,
}

/// Delete photos from app only (DB + thumbnails), keep original files
#[tauri::command]
async fn delete_photos_from_app(
    ids: Vec<i64>,
    db_path: String,
    thumb_dir: String,
) -> Result<DeleteResult, String> {
    let index = PhotoIndex::open(db_path).map_err(|e| e.to_string())?;

    let thumbnailer = fotos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
    let spec = fotos_core::ThumbnailSpec { width: 256, height: 256 };

    let mut result = DeleteResult::default();

    // Delete each photo from DB and remove its thumbnail
    let deleted_photos = index.delete_by_ids(ids).map_err(|e| e.to_string())?;

    for photo in deleted_photos {
        result.deleted_paths.push(photo.path.clone());
        result.deleted_count += 1;

        // Try to delete thumbnail
        let source_path = std::path::Path::new(&photo.path);
        if let Ok(Some(thumb_path)) = thumbnailer.get_cached_path(source_path, &spec) {
            if let Err(e) = std::fs::remove_file(&thumb_path) {
                result.errors.push(format!("Failed to delete thumbnail {}: {}", thumb_path.display(), e));
            }
        }
    }

    Ok(result)
}

/// Delete photos completely (DB + thumbnails + original files)
#[tauri::command]
async fn delete_photos_completely(
    ids: Vec<i64>,
    db_path: String,
    thumb_dir: String,
) -> Result<DeleteResult, String> {
    let index = PhotoIndex::open(db_path).map_err(|e| e.to_string())?;

    let thumbnailer = fotos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
    let spec = fotos_core::ThumbnailSpec { width: 256, height: 256 };

    let mut result = DeleteResult::default();

    // Delete each photo from DB and remove its thumbnail + original file
    let deleted_photos = index.delete_by_ids(ids).map_err(|e| e.to_string())?;

    for photo in deleted_photos {
        result.deleted_paths.push(photo.path.clone());
        result.deleted_count += 1;

        // Try to delete thumbnail
        let source_path = std::path::Path::new(&photo.path);
        if let Ok(Some(thumb_path)) = thumbnailer.get_cached_path(source_path, &spec) {
            if let Err(e) = std::fs::remove_file(&thumb_path) {
                result.errors.push(format!("Failed to delete thumbnail {}: {}", thumb_path.display(), e));
            }
        }

        // Delete original file
        if let Err(e) = std::fs::remove_file(&photo.path) {
            result.errors.push(format!("Failed to delete original {}: {}", photo.path, e));
        }
    }

    Ok(result)
}

#[tauri::command]
async fn clear_app_data(thumb_dir: String, db_path: String) -> Result<(), String> {
    // Clear thumbnails
    if std::path::Path::new(&thumb_dir).exists() {
        std::fs::remove_dir_all(&thumb_dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;

    // Clear database
    if std::path::Path::new(&db_path).exists() {
        std::fs::remove_file(&db_path).map_err(|e| e.to_string())?;
    }

    // Note: map_tiles directory is preserved
    Ok(())
}

#[tauri::command]
async fn regenerate_thumbnails(window: tauri::Window, db_path: String, thumb_dir: String) -> Result<(), String> {
    
    // Ensure parent directories exist
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
         std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;

    let index = PhotoIndex::open(db_path.clone())
        .map_err(|e| e.to_string())?;
    
    let photos = index.list().map_err(|e| e.to_string())?;
    let total = photos.len();
    
    let config = PhotoCoreConfig {
        thumbnail_dir: thumb_dir,
        thumbnail_size: 256,
    };

    let mut success = 0;
    let mut failure = 0;

    for (i, photo) in photos.iter().enumerate() {
        let path = std::path::PathBuf::from(&photo.path);
        
        let file_result = fotos_core::generate_thumbnail(&path, &config);

        match file_result {
            Ok(_) => success += 1,
            Err(_) => failure += 1,
        }
        
        // Emit progress
        use tauri::Emitter;
        let _ = window.emit("import-progress", serde_json::json!({
            "current": i + 1,
            "total": total,
            "success": success,
            "failure": failure,
            "last_path": photo.path
        }));
    }
    
    Ok(())
}

#[tauri::command]
async fn read_file_bytes(path: String) -> Result<Vec<u8>, String> {
    std::fs::read(&path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_raw_preview(path: String, cache_dir: String) -> Result<String, String> {
    let source_path = std::path::Path::new(&path);

    // Create a unique cache filename based on the source path
    let file_name = source_path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("preview");
    let hash = fotos_core::compute_hash(source_path).map_err(|e| e.to_string())?;
    let preview_path = std::path::PathBuf::from(&cache_dir)
        .join("raw_previews")
        .join(format!("{}_{}.jpg", file_name, &hash[..16]));

    // Return cached preview if it exists
    if preview_path.exists() {
        return Ok(preview_path.to_string_lossy().to_string());
    }

    // Extract and cache the preview
    let preview_bytes = fotos_core::extract_raw_preview(source_path).map_err(|e| e.to_string())?;

    // Ensure directory exists
    if let Some(parent) = preview_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // Write to cache
    std::fs::write(&preview_path, &preview_bytes).map_err(|e| e.to_string())?;

    Ok(preview_path.to_string_lossy().to_string())
}

/// Request photo library access on iOS
/// This is a stub that needs native Swift implementation
#[tauri::command]
async fn request_photo_library_access(
    _db_path: String,
    _thumb_dir: String,
) -> Result<u32, String> {
    #[cfg(target_os = "ios")]
    {
        // TODO: Implement native iOS photo picker via Swift
        // 1. Request PHPhotoLibrary authorization
        // 2. Present PHPickerViewController
        // 3. Process selected photos through fotos-core
        Err("Photo library access requires native iOS implementation. Use Xcode to add Swift code for PHPickerViewController.".to_string())
    }
    #[cfg(not(target_os = "ios"))]
    {
        Err("Photo library access is only available on iOS. Use folder/file import on desktop.".to_string())
    }
}

#[tauri::command]
async fn get_cached_tile(cache_dir: String, z: u32, x: u32, y: u32) -> Result<Option<String>, String> {
    let tile_path = std::path::PathBuf::from(&cache_dir)
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{}.png", y));

    if tile_path.exists() {
        Ok(Some(tile_path.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

#[tauri::command]
async fn download_tile(cache_dir: String, z: u32, x: u32, y: u32, url: String) -> Result<String, String> {
    let tile_path = std::path::PathBuf::from(&cache_dir)
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{}.png", y));

    // Check if already cached
    if tile_path.exists() {
        return Ok(tile_path.to_string_lossy().to_string());
    }

    // Create directory structure
    if let Some(parent) = tile_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // Download tile
    let response = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;

    // Save to cache
    std::fs::write(&tile_path, &bytes).map_err(|e| e.to_string())?;

    Ok(tile_path.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            get_core_version,
            import_photos,
            cancel_import,
            list_photos,
            clear_app_data,
            regenerate_thumbnails,
            read_file_bytes,
            get_raw_preview,
            get_cached_tile,
            download_tile,
            delete_photos_from_app,
            delete_photos_completely,
            request_photo_library_access
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
