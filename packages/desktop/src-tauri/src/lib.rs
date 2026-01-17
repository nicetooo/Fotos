use footos_core::{PhotoCoreConfig, PhotoIndex, ImportResult, PhotoInfo};
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
    footos_core::get_version()
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
    let thumbnailer = footos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
    let spec = footos_core::ThumbnailSpec { width: 256, height: 256 };
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
    println!("[Import] Starting import for: {}", root_path);
    println!("[Import] DB path: {}", db_path);
    println!("[Import] Thumb dir: {}", thumb_dir);

    // Handle file:// URIs (iOS returns these)
    let root_path = if root_path.starts_with("file://") {
        let decoded = urlencoding::decode(&root_path[7..])
            .map_err(|e| format!("Failed to decode file URI: {}", e))?
            .into_owned();
        println!("[Import] Decoded file URI to: {}", decoded);
        decoded
    } else {
        root_path
    };

    // Reset cancellation flag at start
    IMPORT_CANCELLED.store(false, Ordering::SeqCst);

    // Ensure parent directories exist
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            println!("[Import] Failed to create db parent dir: {}", e);
            e.to_string()
        })?;
    }
    std::fs::create_dir_all(&thumb_dir).map_err(|e| {
        println!("[Import] Failed to create thumb dir: {}", e);
        e.to_string()
    })?;

    let index = PhotoIndex::open(db_path)
        .map_err(|e| {
            println!("[Import] Failed to open DB: {}", e);
            e.to_string()
        })?;

    let config = PhotoCoreConfig {
        thumbnail_dir: thumb_dir,
        thumbnail_size: 256,
    };

    let root_path_buf = std::path::Path::new(&root_path);

    // Check if path exists
    println!("[Import] Checking path exists: {}", root_path_buf.exists());
    println!("[Import] Is file: {}", root_path_buf.is_file());
    println!("[Import] Is dir: {}", root_path_buf.is_dir());

    // Quick check: if file doesn't exist, return early with error
    if !root_path_buf.exists() {
        println!("[Import] File does not exist: {}", root_path);
        return Err(format!("File does not exist: {}", root_path));
    }

    // Create permanent storage directory for imported photos
    let photos_dir = std::path::Path::new(&config.thumbnail_dir).parent()
        .map(|p| p.join("Photos"))
        .unwrap_or_else(|| std::path::PathBuf::from(&config.thumbnail_dir).join("Photos"));
    std::fs::create_dir_all(&photos_dir).map_err(|e| {
        println!("[Import] Failed to create photos dir: {}", e);
        e.to_string()
    })?;
    println!("[Import] Photos storage dir: {:?}", photos_dir);

    // Support both single file and directory import
    let photos = if root_path_buf.is_file() {
        println!("[Import] Processing as single file");
        // Verify file is readable
        if let Err(e) = std::fs::metadata(&root_path_buf) {
            println!("[Import] Cannot read file metadata: {}", e);
            return Err(format!("Cannot read file: {}", e));
        }

        // Copy file to permanent storage (iOS temp files get deleted)
        let filename = root_path_buf.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("photo_{}.jpg", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)));
        let dest_path = photos_dir.join(&filename);

        // Only copy if source and dest are different
        if root_path_buf != dest_path {
            println!("[Import] Copying to permanent storage: {:?}", dest_path);
            std::fs::copy(&root_path_buf, &dest_path).map_err(|e| {
                println!("[Import] Failed to copy file: {}", e);
                e.to_string()
            })?;
            vec![dest_path]
        } else {
            vec![root_path_buf.to_path_buf()]
        }
    } else {
        println!("[Import] Processing as directory");
        footos_core::scan_photos(root_path_buf).map_err(|e| {
            println!("[Import] Scan error: {}", e);
            e.to_string()
        })?
    };
    let total = photos.len();
    println!("[Import] Found {} photos to process", total);

    let mut result = ImportResult::default();

    // If no photos found, return early
    if total == 0 {
        println!("[Import] No photos to import");
        return Ok(result);
    }
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
        // Returns: Ok(true) = new photo, Ok(false) = duplicate, Err = failure
        let file_result = (|| -> Result<bool, String> {
            println!("[Import] Computing hash...");
            let hash = footos_core::compute_hash(&path).map_err(|e| {
                println!("[Import] Hash error: {}", e);
                e.to_string()
            })?;
            println!("[Import] Hash: {}", &hash[..hash.len().min(16)]);

            // Check if this photo already exists (by hash)
            if index.exists_by_hash(&hash).unwrap_or(false) {
                println!("[Import] DUPLICATE (already imported): {}", path_str);
                // Clean up the copied file since we don't need it
                let _ = std::fs::remove_file(&path);
                return Ok(false); // Duplicate
            }

            println!("[Import] Reading metadata for: {}", path_str);
            let metadata = footos_core::read_metadata(&path).map_err(|e| {
                println!("[Import] Metadata error: {}", e);
                e.to_string()
            })?;
            println!("[Import] Metadata: {:?}", metadata);

            // Thumbnail generation may fail if no EXIF thumbnail - that's OK, frontend uses original
            println!("[Import] Generating thumbnail...");
            let thumb_result = footos_core::generate_thumbnail(&path, &config);
            println!("[Import] Thumbnail result: {:?}", thumb_result.is_ok());

            println!("[Import] Inserting into DB...");
            index.insert(path_str.clone(), hash.clone(), metadata).map_err(|e| {
                println!("[Import] DB insert error: {}", e);
                e.to_string()
            })?;
            Ok(true) // New photo
        })();

        match file_result {
            Ok(true) => {
                println!("[Import] SUCCESS: {}", path_str);
                result.success += 1;
            },
            Ok(false) => {
                // Duplicate - already counted above
                result.duplicates += 1;
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
            "duplicates": result.duplicates,
            "last_path": path_str
        }));
    }

    println!("[Import] Complete! Success: {}, Failure: {}", result.success, result.failure);
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

    let thumbnailer = footos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
    let spec = footos_core::ThumbnailSpec { width: 256, height: 256 };

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

    let thumbnailer = footos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
    let spec = footos_core::ThumbnailSpec { width: 256, height: 256 };

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
        
        let file_result = footos_core::generate_thumbnail(&path, &config);

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
    let hash = footos_core::compute_hash(source_path).map_err(|e| e.to_string())?;
    let hash_prefix = &hash[..hash.len().min(16)];
    let preview_path = std::path::PathBuf::from(&cache_dir)
        .join("raw_previews")
        .join(format!("{}_{}.jpg", file_name, hash_prefix));

    // Return cached preview if it exists
    if preview_path.exists() {
        return Ok(preview_path.to_string_lossy().to_string());
    }

    // Extract and cache the preview
    let preview_bytes = footos_core::extract_raw_preview(source_path).map_err(|e| e.to_string())?;

    // Ensure directory exists
    if let Some(parent) = preview_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // Write to cache
    std::fs::write(&preview_path, &preview_bytes).map_err(|e| e.to_string())?;

    Ok(preview_path.to_string_lossy().to_string())
}

/// Request photo library access on iOS
/// This triggers the Swift photo picker via JavaScript bridge
#[tauri::command]
async fn request_photo_library_access(
    window: tauri::Window,
    db_path: String,
    thumb_dir: String,
) -> Result<String, String> {
    #[cfg(target_os = "ios")]
    {
        // On iOS, we trigger the Swift photo picker via JavaScript
        // The Swift code will call back with selected photos
        use tauri::Emitter;
        window.emit("show-ios-photo-picker", serde_json::json!({
            "dbPath": db_path,
            "thumbDir": thumb_dir
        })).map_err(|e| e.to_string())?;
        Ok("Photo picker triggered".to_string())
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (window, db_path, thumb_dir);
        Err("Photo library access is only available on iOS. Use folder/file import on desktop.".to_string())
    }
}

/// Process a photo from iOS photo picker
#[tauri::command]
async fn process_ios_photo(
    photo_json: String,
    db_path: String,
    thumb_dir: String,
) -> Result<bool, String> {
    use serde_json::Value;

    let photo: Value = serde_json::from_str(&photo_json)
        .map_err(|e| format!("Invalid photo JSON: {}", e))?;

    let path = photo["path"].as_str()
        .ok_or("Missing path in photo data")?;

    let identifier = photo["identifier"].as_str()
        .unwrap_or("unknown");

    // Ensure directories exist
    std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let source_path = std::path::Path::new(path);

    // Read metadata
    let metadata = footos_core::read_metadata(source_path)
        .map_err(|e| format!("Failed to read metadata: {}", e))?;

    // Compute hash
    let hash = footos_core::compute_hash(source_path)
        .map_err(|e| format!("Failed to compute hash: {}", e))?;

    // Generate thumbnail
    let config = footos_core::PhotoCoreConfig {
        thumbnail_dir: thumb_dir.clone(),
        thumbnail_size: 256,
    };
    let _ = footos_core::generate_thumbnail(source_path, &config);

    // Open database and insert
    let index = footos_core::PhotoIndex::open(db_path)
        .map_err(|e| format!("Failed to open database: {}", e))?;

    // Use iOS photo identifier as the path prefix for reference
    let stored_path = format!("ios-photo://{}", identifier);

    index.insert(stored_path, hash, metadata)
        .map_err(|e| format!("Failed to insert photo: {}", e))?;

    // Clean up temp file
    let _ = std::fs::remove_file(path);

    Ok(true)
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
            request_photo_library_access,
            process_ios_photo,
            import_all_ios_photos
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Import all authorized photos from iOS photo library
/// This is the iOS-specific implementation that uses PhotoKit directly
#[tauri::command]
async fn import_all_ios_photos(
    window: tauri::Window,
    db_path: String,
    thumb_dir: String,
) -> Result<ImportResult, String> {
    #[cfg(target_os = "ios")]
    {
        use std::process::Command;

        // On iOS, we need to use the native PhotoKit API
        // For now, emit an event to trigger Swift code via JavaScript injection
        use tauri::Emitter;
        window.emit("trigger-ios-import-all", serde_json::json!({
            "dbPath": db_path,
            "thumbDir": thumb_dir
        })).map_err(|e| e.to_string())?;

        // Return a pending result - actual results come via events
        Ok(ImportResult {
            success: 0,
            failure: 0,
            duplicates: 0,
        })
    }

    #[cfg(not(target_os = "ios"))]
    {
        let _ = (window, db_path, thumb_dir);
        Err("This command is only available on iOS".to_string())
    }
}
