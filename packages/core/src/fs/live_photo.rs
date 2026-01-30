//! Live Photo pairing detection
//!
//! iPhone Live Photo consists of a still image (HEIC/JPEG) + a short video (MOV/MP4),
//! with the same filename (e.g. IMG_1234.HEIC + IMG_1234.MOV).
//!
//! This module detects pairing relationships after file scanning, marking paired videos
//! as part of a Live Photo to prevent them from being indexed as standalone videos.

use std::collections::HashMap;
use std::path::PathBuf;

/// Live Photo pairing result
#[derive(Debug, Clone)]
pub struct LivePhotoPair {
    /// Still image path (HEIC/JPEG)
    pub image_path: PathBuf,
    /// Paired video path (MOV/MP4)
    pub video_path: PathBuf,
}

/// Maximum video file size for Live Photo pairing (30MB).
/// Real Live Photos are typically 2-5 seconds, well under this limit.
/// Larger files are likely coincidentally same-named long videos.
const LIVE_PHOTO_MAX_VIDEO_SIZE: u64 = 30 * 1024 * 1024;

/// Detect Live Photo pairs from a file list.
///
/// Pairing rules:
/// 1. Same directory, same basename (e.g. IMG_1234.HEIC + IMG_1234.MOV)
/// 2. One is an image file, the other is a video file
/// 3. Video file < 30MB (to exclude coincidentally same-named long videos)
///
/// Returns all detected pairs. Paired video files should not be indexed as standalone Videos.
pub fn detect_live_photo_pairs(files: &[PathBuf]) -> Vec<LivePhotoPair> {
    // Group by (directory, basename)
    let mut groups: HashMap<(String, String), (Vec<PathBuf>, Vec<PathBuf>)> = HashMap::new();

    for path in files {
        let dir = path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let basename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();

        if basename.is_empty() {
            continue;
        }

        let key = (dir, basename);
        let entry = groups.entry(key).or_insert_with(|| (vec![], vec![]));

        if super::scan::is_supported_image(path) {
            entry.0.push(path.clone());
        } else if super::scan::is_supported_video(path) {
            entry.1.push(path.clone());
        }
    }

    let mut pairs = Vec::new();
    for (_, (images, videos)) in &groups {
        // Strict 1:1 pairing only
        if images.len() == 1 && videos.len() == 1 {
            // Check video file size to exclude large video files
            if let Ok(meta) = std::fs::metadata(&videos[0]) {
                if meta.len() < LIVE_PHOTO_MAX_VIDEO_SIZE {
                    pairs.push(LivePhotoPair {
                        image_path: images[0].clone(),
                        video_path: videos[0].clone(),
                    });
                }
            }
        }
    }

    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_detect_live_photo_pairs_basic() {
        let temp = std::env::temp_dir().join("footos_livephoto_test");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        // Live Photo pair
        let img = temp.join("IMG_1234.heic");
        let mov = temp.join("IMG_1234.mov");
        fs::write(&img, b"fake heic").unwrap();
        fs::write(&mov, b"fake mov data").unwrap();

        // Standalone photo (no pair)
        let solo = temp.join("DSC_5678.jpg");
        fs::write(&solo, b"fake jpg").unwrap();

        // Standalone video (no pair)
        let vid = temp.join("VID_9999.mp4");
        fs::write(&vid, b"fake mp4").unwrap();

        let files = vec![img.clone(), mov.clone(), solo.clone(), vid.clone()];

        let pairs = detect_live_photo_pairs(&files);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].image_path, img);
        assert_eq!(pairs[0].video_path, mov);

        fs::remove_dir_all(&temp).unwrap();
    }

    #[test]
    fn test_small_video_is_paired() {
        let temp = std::env::temp_dir().join("footos_livephoto_small");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        let img = temp.join("IMG_0001.heic");
        let mov = temp.join("IMG_0001.mov");
        fs::write(&img, b"fake heic").unwrap();
        fs::write(&mov, vec![0u8; 100]).unwrap(); // small file, should be paired

        let pairs = detect_live_photo_pairs(&[img, mov]);
        assert_eq!(pairs.len(), 1);

        fs::remove_dir_all(&temp).unwrap();
    }

    #[test]
    fn test_different_directories_not_paired() {
        let temp = std::env::temp_dir().join("footos_livephoto_dirs");
        let _ = fs::remove_dir_all(&temp);
        let dir_a = temp.join("a");
        let dir_b = temp.join("b");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        // Same basename, different directories — should NOT pair
        let img = dir_a.join("IMG_0001.heic");
        let mov = dir_b.join("IMG_0001.mov");
        fs::write(&img, b"fake heic").unwrap();
        fs::write(&mov, b"fake mov").unwrap();

        let pairs = detect_live_photo_pairs(&[img, mov]);
        assert_eq!(
            pairs.len(),
            0,
            "Files in different directories should not pair"
        );

        fs::remove_dir_all(&temp).unwrap();
    }

    #[test]
    fn test_case_insensitive_basename() {
        let temp = std::env::temp_dir().join("footos_livephoto_case");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        // Different case basenames — should still pair (lowercased comparison)
        let img = temp.join("img_1234.HEIC");
        let mov = temp.join("IMG_1234.mov");
        fs::write(&img, b"fake heic").unwrap();
        fs::write(&mov, b"fake mov").unwrap();

        let pairs = detect_live_photo_pairs(&[img, mov]);
        assert_eq!(pairs.len(), 1, "Case-insensitive basename should pair");

        fs::remove_dir_all(&temp).unwrap();
    }

    #[test]
    fn test_jpeg_mov_pair() {
        let temp = std::env::temp_dir().join("footos_livephoto_jpeg");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        // JPEG + MOV pair (some iPhones export Live Photos as JPEG)
        let img = temp.join("IMG_5678.jpg");
        let mov = temp.join("IMG_5678.MOV");
        fs::write(&img, b"fake jpg").unwrap();
        fs::write(&mov, b"fake mov").unwrap();

        let pairs = detect_live_photo_pairs(&[img, mov]);
        assert_eq!(pairs.len(), 1, "JPEG + MOV should pair as Live Photo");

        fs::remove_dir_all(&temp).unwrap();
    }

    #[test]
    fn test_multiple_images_same_basename_no_pair() {
        let temp = std::env::temp_dir().join("footos_livephoto_multi");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        // Two images with same basename — ambiguous, should NOT pair
        let img1 = temp.join("IMG_0001.heic");
        let img2 = temp.join("IMG_0001.jpg");
        let mov = temp.join("IMG_0001.mov");
        fs::write(&img1, b"fake heic").unwrap();
        fs::write(&img2, b"fake jpg").unwrap();
        fs::write(&mov, b"fake mov").unwrap();

        let pairs = detect_live_photo_pairs(&[img1, img2, mov]);
        assert_eq!(
            pairs.len(),
            0,
            "Multiple images with same basename should not pair"
        );

        fs::remove_dir_all(&temp).unwrap();
    }

    #[test]
    fn test_empty_file_list() {
        let pairs = detect_live_photo_pairs(&[]);
        assert_eq!(pairs.len(), 0);
    }

    #[test]
    fn test_no_pairs_in_photos_only() {
        let temp = std::env::temp_dir().join("footos_livephoto_nopair");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        let img1 = temp.join("IMG_0001.heic");
        let img2 = temp.join("IMG_0002.jpg");
        fs::write(&img1, b"fake").unwrap();
        fs::write(&img2, b"fake").unwrap();

        let pairs = detect_live_photo_pairs(&[img1, img2]);
        assert_eq!(pairs.len(), 0, "Photos-only list should have no pairs");

        fs::remove_dir_all(&temp).unwrap();
    }
}
