use crate::error::CoreError;
use std::path::Path;

pub struct ImageDimensions {
    pub width: u32,
    pub height: u32,
}

pub fn get_dimensions(path: &Path) -> Result<ImageDimensions, CoreError> {
    let img = image::open(path).map_err(|_| CoreError::ImageDecode)?;
    let (width, height) = image::GenericImageView::dimensions(&img);
    Ok(ImageDimensions { width, height })
}
