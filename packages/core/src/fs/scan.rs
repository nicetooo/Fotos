use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::error::CoreError;

pub fn scan_photos(root: &Path) -> Result<Vec<PathBuf>, CoreError> {
    let mut result = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        let path = entry.path();

        if path.is_file() && is_supported_image(path) {
            result.push(path.to_path_buf());
        }
    }

    Ok(result)
}

fn is_supported_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "png")
    )
}
