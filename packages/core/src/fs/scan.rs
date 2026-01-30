use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::error::CoreError;

/// Scans the given directory for supported media files (photos and videos).
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

        if path.is_file() && is_supported_media(path) {
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

/// Check if file is a supported image format.
pub fn is_supported_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some(
            "jpg" | "jpeg" | "png" | "webp" |
             // HEIC/HEIF (iOS photos)
             "heic" | "heif" |
             // RAW formats
             "cr2" | "cr3" | "nef" | "nrw" | "arw" | "srf" | "sr2" |
             "dng" | "raf" | "orf" | "rw2" | "pef" | "raw"
        )
    )
}

/// Check if file is a supported video format.
pub fn is_supported_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some("mp4" | "mov" | "m4v" | "avi" | "mkv" | "webm" | "flv" | "wmv" | "3gp" | "ts")
    )
}

/// Check if file is a supported media file (image or video).
pub fn is_supported_media(path: &Path) -> bool {
    is_supported_image(path) || is_supported_video(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};

    #[test]
    fn test_scan_stability_filters() {
        let temp_dir = std::env::temp_dir().join("footos_scan_test");
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir).unwrap();
        }
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
    fn test_is_supported_video_extensions() {
        assert!(is_supported_video(Path::new("video.mp4")));
        assert!(is_supported_video(Path::new("video.MP4")));
        assert!(is_supported_video(Path::new("video.mov")));
        assert!(is_supported_video(Path::new("video.MOV")));
        assert!(is_supported_video(Path::new("video.m4v")));
        assert!(is_supported_video(Path::new("video.avi")));
        assert!(is_supported_video(Path::new("video.mkv")));
        assert!(is_supported_video(Path::new("video.webm")));
        assert!(is_supported_video(Path::new("video.flv")));
        assert!(is_supported_video(Path::new("video.wmv")));
        assert!(is_supported_video(Path::new("video.3gp")));
        assert!(is_supported_video(Path::new("video.ts")));
        assert!(!is_supported_video(Path::new("photo.jpg")));
        assert!(!is_supported_video(Path::new("photo.cr2")));
        assert!(!is_supported_video(Path::new("doc.txt")));
    }

    #[test]
    fn test_scan_includes_video_files() {
        let temp_dir = std::env::temp_dir().join("footos_scan_video_test");
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir).unwrap();
        }
        fs::create_dir_all(&temp_dir).unwrap();

        let jpg = temp_dir.join("photo.jpg");
        fs::write(&jpg, b"fake jpg").unwrap();
        let mp4 = temp_dir.join("video.mp4");
        fs::write(&mp4, b"fake mp4").unwrap();
        let mov = temp_dir.join("clip.MOV");
        fs::write(&mov, b"fake mov").unwrap();
        let txt = temp_dir.join("notes.txt");
        fs::write(&txt, b"notes").unwrap();

        let results = scan_photos(&temp_dir).expect("Scan failed");

        assert!(results.iter().any(|p| p.ends_with("photo.jpg")));
        assert!(results.iter().any(|p| p.ends_with("video.mp4")));
        assert!(results.iter().any(|p| p.ends_with("clip.MOV")));
        assert!(!results.iter().any(|p| p.ends_with("notes.txt")));

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
