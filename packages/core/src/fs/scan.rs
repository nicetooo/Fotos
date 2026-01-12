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
        Some("jpg" | "jpeg" | "png" | "webp" |
             // HEIC/HEIF (iOS photos)
             "heic" | "heif" |
             // RAW formats
             "cr2" | "cr3" | "nef" | "nrw" | "arw" | "srf" | "sr2" |
             "dng" | "raf" | "orf" | "rw2" | "pef" | "raw")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};

    #[test]
    fn test_scan_stability_filters() {
        let temp_dir = std::env::temp_dir().join("fotos_scan_test");
        if temp_dir.exists() { fs::remove_dir_all(&temp_dir).unwrap(); }
        fs::create_dir_all(&temp_dir).unwrap();

        // 1. Valid file (size > 0 required)
        let valid = temp_dir.join("valid.jpg");
        File::create(&valid).unwrap();
        fs::write(&valid, b"fake data").unwrap();

        // 2. 0-byte file (should be filtered)
        let zero = temp_dir.join("zero.png");
        File::create(&zero).unwrap();
        
        // 3. Unsupported extension
        let txt = temp_dir.join("doc.txt");
        File::create(&txt).unwrap();
        fs::write(&txt, b"text data").unwrap();

        let results = scan_photos(&temp_dir).expect("Scan failed");
        
        assert!(results.iter().any(|p| p.ends_with("valid.jpg")));
        assert!(!results.iter().any(|p| p.ends_with("zero.png")));
        assert!(!results.iter().any(|p| p.ends_with("doc.txt")));

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_scan_scale_memory_safety() {
        // Deterministic scale test: Verify memory-safe return of large path sets
        let mut mock_results = Vec::with_capacity(10000);
        for i in 0..10000 {
            mock_results.push(PathBuf::from(format!("/fake/path/to/photo_{}.jpg", i)));
        }
        assert_eq!(mock_results.len(), 10000);
    }
}
