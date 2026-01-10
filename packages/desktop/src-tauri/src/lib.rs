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
    println!("[list_photos] checking thumbnails for {} photos, thumb_dir={}", photos.len(), thumb_dir);
    for photo in &mut photos {
        let source_path = std::path::Path::new(&photo.path);
        // Use get_cached_path which uses the same thumbnail_key logic as generation
        match thumbnailer.get_cached_path(source_path, &spec) {
            Ok(Some(path)) => {
                println!("[list_photos] thumbnail FOUND: {:?} -> {:?}", source_path, path);
                photo.thumb_path = Some(path.to_string_lossy().to_string());
            }
            Ok(None) => {
                println!("[list_photos] thumbnail NOT FOUND for: {:?}", source_path);
                photo.thumb_path = None;
            }
            Err(e) => {
                println!("[list_photos] thumbnail lookup ERROR for {:?}: {:?}", source_path, e);
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
    println!("Starting import from: {}", root_path);
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

    println!("Registering config and running pipeline...");
    
    let root_path_buf = std::path::Path::new(&root_path);
    let photos = fotos_core::scan_photos(root_path_buf).map_err(|e| e.to_string())?;
    let total = photos.len();
    println!("Found {} photos", total);

    let mut result = ImportResult::default();
    for (i, path) in photos.into_iter().enumerate() {
        let path_str = path.to_string_lossy().to_string();
        
        // Use a block to ensure we can handle errors per-file
        let file_result = (|| -> Result<(), String> {
            println!("[import] processing: {:?}", path);
            let total_start = std::time::Instant::now();

            let t0 = std::time::Instant::now();
            let metadata = fotos_core::read_metadata(&path).map_err(|e| {
                println!("[import] metadata error: {}", e);
                e.to_string()
            })?;
            println!("[timing] metadata: {:?}", t0.elapsed());

            let t1 = std::time::Instant::now();
            let hash = fotos_core::compute_hash(&path).map_err(|e| {
                println!("[import] hash error: {}", e);
                e.to_string()
            })?;
            println!("[timing] hash: {:?}", t1.elapsed());

            let t2 = std::time::Instant::now();
            let thumb_result = fotos_core::generate_thumbnail(&path, &config);
            println!("[timing] thumbnail: {:?}", t2.elapsed());
            match &thumb_result {
                Ok(thumb_path) => println!("[import] thumbnail generated: {:?}", thumb_path),
                Err(e) => println!("[import] thumbnail error: {}", e),
            }
            thumb_result.map_err(|e| e.to_string())?;

            let t3 = std::time::Instant::now();
            index.insert(path_str.clone(), hash, metadata).map_err(|e| e.to_string())?;
            println!("[timing] db insert: {:?}", t3.elapsed());

            println!("[import] SUCCESS: {:?} (total: {:?})", path, total_start.elapsed());
            Ok(())
        })();

        match file_result {
            Ok(_) => result.success += 1,
            Err(e) => {
                println!("Error processing {:?}: {}", path, e);
                result.failure += 1;
            }
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
    
    println!("Pipeline finished: {:?}", result);
    Ok(result)
}

#[tauri::command]
async fn clear_thumbnail_cache(thumb_dir: String) -> Result<(), String> {
    println!("Clearing thumbnail cache at: {}", thumb_dir);
    if std::path::Path::new(&thumb_dir).exists() {
        std::fs::remove_dir_all(&thumb_dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn regenerate_thumbnails(window: tauri::Window, db_path: String, thumb_dir: String) -> Result<(), String> {
    println!("Regenerating thumbnails...");
    
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
            Err(e) => {
                println!("Error generating thumbnail for {:?}: {}", path, e);
                failure += 1;
            }
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
            read_file_bytes
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
