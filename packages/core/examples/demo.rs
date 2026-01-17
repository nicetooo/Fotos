use footos_core::{run_import_pipeline, PhotoCoreConfig, PhotoIndex};
use std::fs;
use std::path::PathBuf;

fn main() {
    let temp_dir = std::env::temp_dir().join("footos_demo");
    let src_dir = temp_dir.join("photos");
    let thumb_dir = temp_dir.join("thumbnails");
    let db_path = temp_dir.join("footos.db");

    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).unwrap();
    }
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&thumb_dir).unwrap();

    println!("--- 足迹相册 Core Demo ---");
    println!("Temp Dir: {:?}", temp_dir);

    // Create a mock image file
    let img_path = src_dir.join("test.jpg");
    fs::write(&img_path, b"this is not a real image but the pipeline will try to process it").unwrap();
    println!("Created mock file: {:?}", img_path);

    let index = PhotoIndex::open(db_path.to_string_lossy().to_string()).expect("Failed to open index");
    let config = PhotoCoreConfig {
        thumbnail_dir: thumb_dir.to_string_lossy().to_string(),
        thumbnail_size: 256,
    };

    println!("Starting import pipeline...");
    let result = run_import_pipeline(src_dir.to_string_lossy().to_string(), index.clone(), config).expect("Pipeline failed");

    println!("Import Results:");
    println!("  Success: {}", result.success);
    println!("  Failure: {} (expected, as the source is not a valid image)", result.failure);

    println!("Database entries:");
    let photos = index.list().expect("Failed to list photos");
    for p in photos {
        println!("  - {:?}", p.path);
    }
    
    println!("Demo finished.");
}
