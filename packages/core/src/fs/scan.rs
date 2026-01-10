use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::error::CoreError;

/// Scans the given directory for supported photo files.
/// 
/// ### ⚠️ Performance & Scale Note
/// Current implementation returns a full `Vec<PathBuf>` once processing is complete.
/// For directories containing a very large number of files (e.g., 100k+), this may 
/// consume substantial memory.
/// 
/// **Recommendations for Callers:**
/// - Control the scale of the root directory passed to this function.
/// - Be mindful of the frequency of calls in low-memory environments.
/// - Future versions may provide an iterator-based or paged implementation.
pub fn scan_photos(root: &Path) -> Result<Vec<PathBuf>, CoreError> {
    let mut result = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        let path = entry.path();

        if path.is_file() && is_supported_image(path) {
            // Stability filters
            if let Ok(metadata) = entry.metadata() {
                if metadata.len() > 0 {
                    result.push(path.to_path_buf());
                }
            }
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
        Some("jpg" | "jpeg" | "png" | "webp")
    )
}
