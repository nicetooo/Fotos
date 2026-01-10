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
pub use image::{generate_thumbnail, compute_hash};
pub use index::PhotoIndex;
pub use metadata::{read_metadata, read_date_taken};

/// Runs the complete import pipeline for a directory.
/// 1. Scans for supported photos.
/// 2. Extracts metadata (including dimensions).
/// 3. Computes perceptual hash.
/// 4. Generates a thumbnail on disk.
/// 5. Inserts the photo into the index.
/// Returns success and failure counts.
pub fn run_import_pipeline(
    root: &std::path::Path,
    index: &PhotoIndex,
    config: &PhotoCoreConfig,
) -> Result<ImportResult, CoreError> {
    let photos = scan_photos(root)?;
    let mut result = ImportResult::default();

    for path in photos {
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

        if generate_thumbnail(&path, config).is_err() {
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
        match index.insert(path_str, &hash, &metadata) {
            Ok(_) => result.success += 1,
            Err(_) => {
                result.failure += 1;
                continue;
            }
        }
    }

    Ok(result)
}
