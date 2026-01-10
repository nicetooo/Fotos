use std::path::{Path, PathBuf};

use crate::{config::PhotoCoreConfig, error::CoreError};

pub fn generate_thumbnail(
    input: &Path,
    config: &PhotoCoreConfig,
) -> Result<PathBuf, CoreError> {
    // 1. Generate a deterministic filename
    let filename = get_thumbnail_filename(input)?;
    let output_path = config.thumbnail_dir.join(filename);

    // 2. Performance: Basic disk-level cache check
    if output_path.exists() {
        return Ok(output_path);
    }

    // 3. Ensure the thumbnail directory exists
    if !config.thumbnail_dir.exists() {
        std::fs::create_dir_all(&config.thumbnail_dir)?;
    }

    // 4. Decode and generate
    // 'thumbnail' method is efficient as it doesn't decode the whole image for large files
    let img = image::open(input).map_err(|_| CoreError::ImageDecode)?;
    let thumb = img.thumbnail(
        config.thumbnail_size,
        config.thumbnail_size,
    );

    // 5. Save to disk
    thumb
        .save(&output_path)
        .map_err(|_| CoreError::ImageDecode)?;

    Ok(output_path)
}

/// Generates a stable, deterministic filename for a given path using BLAKE3.
/// Does not depend on time, random numbers, or Rust version hash stability.
fn get_thumbnail_filename(path: &Path) -> Result<String, CoreError> {
    let path_str = path.to_str().ok_or_else(|| {
        CoreError::InvalidInput(format!("Path contains invalid UTF-8: {:?}", path))
    })?;
    let hash = blake3::hash(path_str.as_bytes());
    Ok(format!("{}.jpg", hash.to_hex()))
}
