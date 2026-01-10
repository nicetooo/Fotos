use std::path::{Path, PathBuf};

use crate::{config::PhotoCoreConfig, error::CoreError};

pub fn generate_thumbnail(
    input: &Path,
    output_name: &str,
    config: &PhotoCoreConfig,
) -> Result<PathBuf, CoreError> {
    let img = image::open(input).map_err(|_| CoreError::ImageDecode)?;

    let thumb = img.thumbnail(
        config.thumbnail_size,
        config.thumbnail_size,
    );

    let output_path = config.thumbnail_dir.join(output_name);
    thumb
        .save(&output_path)
        .map_err(|_| CoreError::ImageDecode)?;

    Ok(output_path)
}
