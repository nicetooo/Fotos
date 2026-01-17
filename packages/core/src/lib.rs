pub mod config;
pub mod error;
pub mod types;

pub mod fs;
pub mod image;
pub mod metadata;
pub mod index;

pub use config::PhotoCoreConfig;
pub use error::CoreError;
pub use types::{PhotoId, PhotoInfo, PhotoMetadata, ImportResult};

pub use fs::scan_photos;
pub use image::{Thumbnailer, ThumbnailSpec, ThumbnailError, compute_hash, extract_raw_preview};
pub use index::PhotoIndex;
pub use metadata::{read_metadata, read_date_taken};

uniffi::setup_scaffolding!();

#[uniffi::export]
pub fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Runs the complete import pipeline for a directory.
#[uniffi::export]
pub fn run_import_pipeline(
    root: String,
    index: std::sync::Arc<PhotoIndex>,
    config: PhotoCoreConfig,
) -> Result<ImportResult, CoreError> {
    let root_path = std::path::Path::new(&root);
    let photos = scan_photos(root_path)?;
    println!("Found {} photos to process", photos.len());
    let mut result = ImportResult::default();

    for (i, path) in photos.iter().enumerate() {
        if i % 10 == 0 {
            println!("Processing [{}/{}] ...", i, photos.len());
        }
        // Individual file processing failures increment failure count but don't stop the pipeline
        
        let metadata = match read_metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                result.failure += 1;
                continue;
            }
        };

        let hash = match compute_hash(&path) {
            Ok(h) => h,
            Err(_) => {
                result.failure += 1;
                continue;
            }
        };

        if generate_thumbnail(&path, &config).is_err() {
            result.failure += 1;
            continue;
        }

        let path_str = match path.to_str() {
            Some(s) => s,
            None => {
                result.failure += 1;
                continue;
            }
        };
        match index.insert(path_str.to_string(), hash.clone(), metadata.clone()) {
            Ok(_) => result.success += 1,
            Err(_) => {
                result.failure += 1;
                continue;
            }
        }
    }

    Ok(result)
}

/// Convenience function to generate a thumbnail using the core config
pub fn generate_thumbnail(path: &std::path::Path, config: &PhotoCoreConfig) -> Result<std::path::PathBuf, CoreError> {
    let thumbnailer = Thumbnailer::new(std::path::PathBuf::from(&config.thumbnail_dir));
    let spec = ThumbnailSpec { width: config.thumbnail_size, height: config.thumbnail_size };
    thumbnailer.generate(path, &spec).map_err(|e| CoreError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};

    #[test]
    fn test_pipeline_error_tolerance_and_invariants() {
        let temp_dir = std::env::temp_dir().join("footos_pipeline_test");
        let thumb_dir = temp_dir.join("thumbs");
        let src_dir = temp_dir.join("src");
        
        if temp_dir.exists() { fs::remove_dir_all(&temp_dir).unwrap(); }
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&thumb_dir).unwrap();

        // 1. Valid-ish photo (empty but we mock metadata to skip decode for speed)
        // Actually pipeline calls read_metadata and generate_thumbnail.
        // If they fail, they hit result.failure.
        let v1 = src_dir.join("v1.jpg");
        File::create(&v1).unwrap();
        fs::write(&v1, b"some fake data").unwrap();

        // 2. Another one
        let v2 = src_dir.join("v2.jpg");
        File::create(&v2).unwrap();
        fs::write(&v2, b"more data").unwrap();

        let index = PhotoIndex::open(temp_dir.join("test.db").to_string_lossy().to_string()).unwrap();
        let config = PhotoCoreConfig {
            thumbnail_dir: thumb_dir.to_string_lossy().to_string(),
            thumbnail_size: 256,
        };

        // Note: The real read_metadata might fail because files aren't real images.
        // But the pipeline is error tolerant!
        let result = run_import_pipeline(src_dir.to_string_lossy().to_string(), index, config).unwrap();

        // Since they aren't real images, success will be 0 and failure will be 2.
        // This confirms the pipeline DOES NOT STOP on errors.
        assert_eq!(result.success, 0);
        assert_eq!(result.failure, 2);

        fs::remove_dir_all(&temp_dir).unwrap();
    }
}
