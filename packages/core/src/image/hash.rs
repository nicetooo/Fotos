use image_hasher::{HasherConfig, HashAlg};
use std::path::Path;
use crate::error::CoreError;

pub fn compute_hash(path: &Path) -> Result<String, CoreError> {
    let img = image::open(path).map_err(|_| CoreError::ImageDecode)?;
    
    let hasher = HasherConfig::new()
        .hash_alg(HashAlg::Gradient)
        .hash_size(8, 8)
        .to_hasher();
        
    let hash = hasher.hash_image(&img);
    Ok(hash.to_base64())
}
