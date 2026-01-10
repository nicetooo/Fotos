use fotos_core::{PhotoCoreConfig, PhotoIndex, ImportResult, PhotoInfo};

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
    
    // Populate thumb_path using the same key logic as thumbnail generation
    let thumbnailer = fotos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
    let spec = fotos_core::ThumbnailSpec { width: 256, height: 256 };
    for photo in &mut photos {
        let source_path = std::path::Path::new(&photo.path);
        // Use get_cached_path which uses the same thumbnail_key logic as generation
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
    let photos = fotos_core::scan_photos(root_path_buf).map_err(|e| e.to_string())?;
    let total = photos.len();

    let mut result = ImportResult::default();
    for (i, path) in photos.into_iter().enumerate() {
        let path_str = path.to_string_lossy().to_string();
        
        // Use a block to ensure we can handle errors per-file
        let file_result = (|| -> Result<(), String> {
            let metadata = fotos_core::read_metadata(&path).map_err(|e| e.to_string())?;
            let hash = fotos_core::compute_hash(&path).map_err(|e| e.to_string())?;
            fotos_core::generate_thumbnail(&path, &config).map_err(|e| e.to_string())?;
            index.insert(path_str.clone(), hash, metadata).map_err(|e| e.to_string())?;
            Ok(())
        })();

        match file_result {
            Ok(_) => result.success += 1,
            Err(_) => result.failure += 1,
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

#[tauri::command]
async fn clear_thumbnail_cache(thumb_dir: String) -> Result<(), String> {
    if std::path::Path::new(&thumb_dir).exists() {
        std::fs::remove_dir_all(&thumb_dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;
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
        .invoke_handler(tauri::generate_handler![
            greet,
            get_core_version,
            import_photos,
            list_photos,
            clear_thumbnail_cache,
            regenerate_thumbnails,
            read_file_bytes,
            get_cached_tile,
            download_tile
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
