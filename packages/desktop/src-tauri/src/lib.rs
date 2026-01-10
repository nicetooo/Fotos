use std::path::PathBuf;
use fotos_core::{PhotoCoreConfig, PhotoIndex, ImportResult};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_core_version() -> String {
    fotos_core::get_version()
}

#[tauri::command]
fn import_photos(
    root_path: String,
    db_path: String,
    thumb_dir: String,
) -> Result<ImportResult, String> {
    let index = PhotoIndex::open(PathBuf::from(db_path).as_path())
        .map_err(|e| e.to_string())?;
    
    let config = PhotoCoreConfig {
        thumbnail_dir: PathBuf::from(thumb_dir),
        thumbnail_size: 256,
    };

    fotos_core::run_import_pipeline(
        PathBuf::from(root_path).as_path(),
        &index,
        &config
    ).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet, 
            get_core_version,
            import_photos
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
