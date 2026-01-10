use image_hasher::{HasherConfig, HashAlg};
use std::path::Path;
use std::io::{BufReader, Read, Seek};
use crate::error::CoreError;

/// Compute perceptual hash of an image.
/// Optimized to use EXIF embedded thumbnail when available (much faster for camera photos).
pub fn compute_hash(path: &Path) -> Result<String, CoreError> {
    let hasher = HasherConfig::new()
        .hash_alg(HashAlg::Gradient)
        .hash_size(8, 8)
        .to_hasher();

    // Try to use embedded thumbnail first (fast path, required for RAW files)
    if let Ok(thumb_data) = try_extract_thumbnail_data(path) {
        if let Ok(img) = image::load_from_memory(&thumb_data) {
            let hash = hasher.hash_image(&img);
            return Ok(hash.to_base64());
        }
    }

    // Check if this is a RAW file - if so, we can't decode it without embedded thumbnail
    if is_raw_file(path) {
        // For RAW files without embedded thumbnail, use a file-based hash
        return compute_file_hash(path);
    }

    // Fallback to full image decode (slow path) - only for standard formats
    let img = image::open(path).map_err(|_| CoreError::ImageDecode)?;
    let hash = hasher.hash_image(&img);
    Ok(hash.to_base64())
}

/// Check if file is a RAW format based on extension
fn is_raw_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some("cr2" | "cr3" | "nef" | "nrw" | "arw" | "srf" | "sr2" |
             "dng" | "raf" | "orf" | "rw2" | "pef" | "raw")
    )
}

/// Compute a simple file-based hash for files that can't be decoded
fn compute_file_hash(path: &Path) -> Result<String, CoreError> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let metadata = std::fs::metadata(path).map_err(|_| CoreError::Io("metadata failed".into()))?;
    let file_size = metadata.len();

    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    file_size.hash(&mut hasher);

    Ok(format!("file:{:016x}", hasher.finish()))
}

/// Detects if file is JPEG or TIFF-based (RAW) by checking magic bytes.
fn is_tiff_based(path: &Path) -> bool {
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

/// Extract embedded JPEG thumbnail data from EXIF.
/// Handles both JPEG and TIFF-based (RAW) files.
fn try_extract_thumbnail_data(path: &Path) -> Result<Vec<u8>, CoreError> {
    let file = std::fs::File::open(path).map_err(|_| CoreError::Io("open failed".into()))?;
    let mut reader = BufReader::new(file);

    let exif_reader = exif::Reader::new();
    let exif = exif_reader.read_from_container(&mut reader)
        .map_err(|_| CoreError::Io("no exif".into()))?;

    let thumbnail_field = exif.get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)
        .ok_or_else(|| CoreError::Io("no thumb offset".into()))?;
    let length_field = exif.get_field(exif::Tag::JPEGInterchangeFormatLength, exif::In::THUMBNAIL)
        .ok_or_else(|| CoreError::Io("no thumb length".into()))?;

    let tiff_offset = thumbnail_field.value.get_uint(0)
        .ok_or_else(|| CoreError::Io("invalid offset".into()))? as usize;
    let length = length_field.value.get_uint(0)
        .ok_or_else(|| CoreError::Io("invalid length".into()))? as usize;

    // Calculate absolute offset based on file type
    let absolute_offset = if is_tiff_based(path) {
        // TIFF-based (RAW): offset is relative to file start
        tiff_offset as u64
    } else {
        // JPEG: offset is relative to TIFF header in APP1 segment
        find_jpeg_tiff_header_offset(path)? + tiff_offset as u64
    };

    // Read thumbnail data
    let file = std::fs::File::open(path).map_err(|_| CoreError::Io("open failed".into()))?;
    let mut reader = BufReader::new(file);

    reader.seek(std::io::SeekFrom::Start(absolute_offset))
        .map_err(|_| CoreError::Io("seek failed".into()))?;

    let mut thumb_data = vec![0u8; length];
    reader.read_exact(&mut thumb_data)
        .map_err(|_| CoreError::Io("read thumb failed".into()))?;

    Ok(thumb_data)
}

/// Find the TIFF header position in a JPEG file by scanning for EXIF APP1 segment.
fn find_jpeg_tiff_header_offset(path: &Path) -> Result<u64, CoreError> {
    let file = std::fs::File::open(path).map_err(|_| CoreError::Io("open failed".into()))?;
    let mut reader = BufReader::new(file);

    let mut buf = [0u8; 2];
    reader.read_exact(&mut buf).map_err(|_| CoreError::Io("read failed".into()))?;
    if buf != [0xFF, 0xD8] {
        return Err(CoreError::Io("not jpeg".into()));
    }

    // Find APP1 EXIF segment
    loop {
        reader.read_exact(&mut buf).map_err(|_| CoreError::Io("read failed".into()))?;
        if buf[0] != 0xFF {
            return Err(CoreError::Io("invalid marker".into()));
        }
        if buf[1] == 0xE1 {
            let mut len_buf = [0u8; 2];
            reader.read_exact(&mut len_buf).map_err(|_| CoreError::Io("read failed".into()))?;
            let mut exif_id = [0u8; 6];
            reader.read_exact(&mut exif_id).map_err(|_| CoreError::Io("read failed".into()))?;
            if &exif_id == b"Exif\0\0" {
                return reader.stream_position()
                    .map_err(|_| CoreError::Io("position failed".into()));
            }
        } else if buf[1] == 0xD9 || buf[1] == 0xDA {
            break;
        } else if buf[1] >= 0xE0 && buf[1] <= 0xEF || buf[1] == 0xFE {
            let mut len_buf = [0u8; 2];
            reader.read_exact(&mut len_buf).map_err(|_| CoreError::Io("read failed".into()))?;
            let seg_len = u16::from_be_bytes(len_buf) as i64 - 2;
            reader.seek(std::io::SeekFrom::Current(seg_len))
                .map_err(|_| CoreError::Io("seek failed".into()))?;
        }
    }

    Err(CoreError::Io("no exif app1".into()))
}
