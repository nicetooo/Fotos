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

    // Try to use embedded thumbnail first (fast path)
    if let Ok(thumb_data) = try_extract_thumbnail_data(path) {
        if let Ok(img) = image::load_from_memory(&thumb_data) {
            println!("[hash] using embedded thumbnail for hash");
            let hash = hasher.hash_image(&img);
            return Ok(hash.to_base64());
        }
    }

    // Fallback to full image decode (slow path)
    println!("[hash] falling back to full image decode");
    let img = image::open(path).map_err(|_| CoreError::ImageDecode)?;
    let hash = hasher.hash_image(&img);
    Ok(hash.to_base64())
}

/// Extract embedded JPEG thumbnail data from EXIF (same logic as thumbnail.rs)
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

    // Find TIFF header position in JPEG
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
                let tiff_header_pos = reader.stream_position()
                    .map_err(|_| CoreError::Io("position failed".into()))?;
                let absolute_offset = tiff_header_pos + tiff_offset as u64;

                reader.seek(std::io::SeekFrom::Start(absolute_offset))
                    .map_err(|_| CoreError::Io("seek failed".into()))?;

                let mut thumb_data = vec![0u8; length];
                reader.read_exact(&mut thumb_data)
                    .map_err(|_| CoreError::Io("read thumb failed".into()))?;

                return Ok(thumb_data);
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
