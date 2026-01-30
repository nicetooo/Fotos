//! Shared image format detection and JPEG/TIFF parsing utilities.
//!
//! Extracted from hash.rs and thumbnail.rs to eliminate code duplication.
//! These functions are used by both the hashing and thumbnail pipelines.

use crate::error::CoreError;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

/// Check if file is HEIC/HEIF format based on extension.
pub(crate) fn is_heic_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some("heic" | "heif")
    )
}

/// Check if file is a RAW format based on extension.
pub fn is_raw_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some(
            "cr2"
                | "cr3"
                | "nef"
                | "nrw"
                | "arw"
                | "srf"
                | "sr2"
                | "dng"
                | "raf"
                | "orf"
                | "rw2"
                | "pef"
                | "raw"
        )
    )
}

/// Detects if file is TIFF-based (RAW) by checking magic bytes.
/// Returns true for TIFF-based files (II or MM header), false otherwise.
pub(crate) fn is_tiff_based(path: &Path) -> bool {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut magic = [0u8; 2];
    if file.read_exact(&mut magic).is_err() {
        return false;
    }

    // TIFF: starts with II (little-endian) or MM (big-endian)
    magic == [0x49, 0x49] || magic == [0x4D, 0x4D]
}

/// Find the TIFF header position in a JPEG file by scanning for EXIF APP1 segment.
///
/// Returns the byte offset of the TIFF header within the JPEG file.
/// This is needed to compute absolute offsets for embedded thumbnails in JPEG files,
/// where EXIF offsets are relative to the TIFF header, not the file start.
pub(crate) fn find_jpeg_tiff_header_offset(path: &Path) -> Result<u64, CoreError> {
    let file = std::fs::File::open(path).map_err(|_| CoreError::Io("open failed".into()))?;
    let mut reader = BufReader::new(file);

    let mut buf = [0u8; 2];
    reader
        .read_exact(&mut buf)
        .map_err(|_| CoreError::Io("read failed".into()))?;
    if buf != [0xFF, 0xD8] {
        return Err(CoreError::Io("not jpeg".into()));
    }

    // Find APP1 EXIF segment
    loop {
        reader
            .read_exact(&mut buf)
            .map_err(|_| CoreError::Io("read failed".into()))?;
        if buf[0] != 0xFF {
            return Err(CoreError::Io("invalid marker".into()));
        }
        if buf[1] == 0xE1 {
            // APP1 - potential EXIF segment
            let mut len_buf = [0u8; 2];
            reader
                .read_exact(&mut len_buf)
                .map_err(|_| CoreError::Io("read failed".into()))?;
            let seg_len = u16::from_be_bytes(len_buf) as i64;
            let mut exif_id = [0u8; 6];
            reader
                .read_exact(&mut exif_id)
                .map_err(|_| CoreError::Io("read failed".into()))?;
            if &exif_id == b"Exif\0\0" {
                return reader
                    .stream_position()
                    .map_err(|_| CoreError::Io("position failed".into()));
            }
            // Not EXIF APP1, skip rest of segment
            if seg_len > 8 {
                reader
                    .seek(std::io::SeekFrom::Current(seg_len - 8))
                    .map_err(|_| CoreError::Io("seek failed".into()))?;
            }
        } else if buf[1] == 0xD9 || buf[1] == 0xDA {
            // EOI or SOS - no more segments to check
            break;
        } else if (buf[1] >= 0xD0 && buf[1] <= 0xD7) || buf[1] == 0x01 {
            // RST markers (0xD0-0xD7) and TEM (0x01) have no length
            continue;
        } else if buf[1] == 0x00 {
            // Stuffed byte (0xFF 0x00), skip
            continue;
        } else {
            // All other markers have length field - skip them
            let mut len_buf = [0u8; 2];
            reader
                .read_exact(&mut len_buf)
                .map_err(|_| CoreError::Io("read failed".into()))?;
            let seg_len = u16::from_be_bytes(len_buf) as i64 - 2;
            if seg_len > 0 {
                reader
                    .seek(std::io::SeekFrom::Current(seg_len))
                    .map_err(|_| CoreError::Io("seek failed".into()))?;
            }
        }
    }

    Err(CoreError::Io("no exif app1".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn test_is_heic_file_extensions() {
        assert!(is_heic_file(Path::new("photo.heic")));
        assert!(is_heic_file(Path::new("photo.HEIC")));
        assert!(is_heic_file(Path::new("photo.heif")));
        assert!(is_heic_file(Path::new("photo.HEIF")));
        assert!(!is_heic_file(Path::new("photo.jpg")));
        assert!(!is_heic_file(Path::new("photo.png")));
        assert!(!is_heic_file(Path::new("photo.cr2")));
        assert!(!is_heic_file(Path::new("no_extension")));
    }

    #[test]
    fn test_is_raw_file_extensions() {
        assert!(is_raw_file(Path::new("photo.cr2")));
        assert!(is_raw_file(Path::new("photo.CR2")));
        assert!(is_raw_file(Path::new("photo.nef")));
        assert!(is_raw_file(Path::new("photo.NEF")));
        assert!(is_raw_file(Path::new("photo.arw")));
        assert!(is_raw_file(Path::new("photo.dng")));
        assert!(is_raw_file(Path::new("photo.raf")));
        assert!(is_raw_file(Path::new("photo.orf")));
        assert!(is_raw_file(Path::new("photo.rw2")));
        assert!(is_raw_file(Path::new("photo.pef")));
        assert!(is_raw_file(Path::new("photo.raw")));
        assert!(is_raw_file(Path::new("photo.cr3")));
        assert!(is_raw_file(Path::new("photo.nrw")));
        assert!(is_raw_file(Path::new("photo.srf")));
        assert!(is_raw_file(Path::new("photo.sr2")));
        assert!(!is_raw_file(Path::new("photo.jpg")));
        assert!(!is_raw_file(Path::new("photo.png")));
        assert!(!is_raw_file(Path::new("photo.heic")));
    }

    #[test]
    fn test_is_tiff_based_valid() {
        let temp_dir = std::env::temp_dir().join("footos_parse_tiff_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // Little-endian TIFF (II)
        let le_path = temp_dir.join("le.tiff");
        let mut f = fs::File::create(&le_path).unwrap();
        f.write_all(&[0x49, 0x49, 0x2A, 0x00]).unwrap();
        assert!(is_tiff_based(&le_path));

        // Big-endian TIFF (MM)
        let be_path = temp_dir.join("be.tiff");
        let mut f = fs::File::create(&be_path).unwrap();
        f.write_all(&[0x4D, 0x4D, 0x00, 0x2A]).unwrap();
        assert!(is_tiff_based(&be_path));

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_is_tiff_based_jpeg() {
        let temp_dir = std::env::temp_dir().join("footos_parse_jpeg_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // JPEG header
        let jpeg_path = temp_dir.join("test.jpg");
        let mut f = fs::File::create(&jpeg_path).unwrap();
        f.write_all(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        assert!(!is_tiff_based(&jpeg_path));

        // Non-existent file
        assert!(!is_tiff_based(Path::new("/nonexistent/file.tiff")));

        // Empty file
        let empty_path = temp_dir.join("empty");
        fs::File::create(&empty_path).unwrap();
        assert!(!is_tiff_based(&empty_path));

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_find_jpeg_tiff_header_offset_valid_jpeg() {
        let temp_dir = std::env::temp_dir().join("footos_parse_offset_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // Construct a minimal JPEG with APP1 EXIF segment
        let jpeg_path = temp_dir.join("exif.jpg");
        let mut f = fs::File::create(&jpeg_path).unwrap();

        // SOI
        f.write_all(&[0xFF, 0xD8]).unwrap();
        // APP1 marker
        f.write_all(&[0xFF, 0xE1]).unwrap();
        // Segment length (2 bytes): 6 (Exif\0\0) + 2 (length itself) + 8 (TIFF header) = 16
        f.write_all(&[0x00, 0x10]).unwrap();
        // Exif ID
        f.write_all(b"Exif\0\0").unwrap();
        // TIFF header (II, magic 42, offset 8)
        f.write_all(&[0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00])
            .unwrap();

        let offset = find_jpeg_tiff_header_offset(&jpeg_path).unwrap();
        // Offset should be right after "Exif\0\0": SOI(2) + APP1 marker(2) + length(2) + ExifID(6) = 12
        assert_eq!(offset, 12);

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_find_jpeg_tiff_header_offset_no_exif() {
        let temp_dir = std::env::temp_dir().join("footos_parse_noexif_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // JPEG with only SOI + SOS (no EXIF)
        let jpeg_path = temp_dir.join("noexif.jpg");
        let mut f = fs::File::create(&jpeg_path).unwrap();
        f.write_all(&[0xFF, 0xD8, 0xFF, 0xDA]).unwrap();
        // SOS needs a length field
        f.write_all(&[0x00, 0x02]).unwrap();

        let result = find_jpeg_tiff_header_offset(&jpeg_path);
        assert!(result.is_err());

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_find_jpeg_tiff_header_offset_not_jpeg() {
        let temp_dir = std::env::temp_dir().join("footos_parse_notjpeg_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // Not a JPEG (PNG header)
        let png_path = temp_dir.join("test.png");
        let mut f = fs::File::create(&png_path).unwrap();
        f.write_all(&[0x89, 0x50, 0x4E, 0x47]).unwrap();

        let result = find_jpeg_tiff_header_offset(&png_path);
        assert!(result.is_err());

        fs::remove_dir_all(&temp_dir).unwrap();
    }
}
