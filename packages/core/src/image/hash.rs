use crate::error::CoreError;
use crate::image::parse::{find_jpeg_tiff_header_offset, is_heic_file, is_tiff_based};
use image_hasher::{HashAlg, HasherConfig};
use std::io::{BufReader, Cursor, Read, Seek};
use std::path::Path;

/// Compute perceptual hash of an image.
/// Optimized to use EXIF embedded thumbnail when available (much faster for camera photos).
pub fn compute_hash(path: &Path) -> Result<String, CoreError> {
    // For HEIC files, use file-based hash directly since image crate can't decode HEIC
    if is_heic_file(path) {
        return compute_file_hash(path);
    }

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

    // For files without EXIF thumbnail (RAW or regular images),
    // use file-based hash to avoid slow full image decode.
    // File hash is sufficient for deduplication (same file = same hash).
    compute_file_hash(path)
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

/// Identify a file by its content, for telling apart "the same file again"
/// from "another photo".
///
/// FNV-1a (64 bit) over the file size, the first 64 KB and the last 64 KB.
/// Copies of one file agree; different photos do not, since headers, embedded
/// thumbnails and the compressed data at the end all differ. Reads at most
/// 128 KB, so it stays fast on large RAW files and slow disks. The algorithm
/// is fixed here (not `DefaultHasher`) because the value is stored.
///
/// Returns `"c:xxxxxxxxxxxxxxxx"`.
pub fn compute_content_hash(path: &Path) -> Result<String, CoreError> {
    use std::io::SeekFrom;

    const CHUNK: usize = 64 * 1024;
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut file = std::fs::File::open(path).map_err(|_| CoreError::Io("open failed".into()))?;
    let size = file
        .metadata()
        .map_err(|_| CoreError::Io("metadata failed".into()))?
        .len();

    let mut hash = FNV_OFFSET;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    };
    feed(&size.to_le_bytes());

    let mut head = vec![0u8; CHUNK.min(size as usize)];
    file.read_exact(&mut head)
        .map_err(|_| CoreError::Io("read head failed".into()))?;
    feed(&head);

    if size > (CHUNK as u64) * 2 {
        file.seek(SeekFrom::End(-(CHUNK as i64)))
            .map_err(|_| CoreError::Io("seek tail failed".into()))?;
        let mut tail = vec![0u8; CHUNK];
        file.read_exact(&mut tail)
            .map_err(|_| CoreError::Io("read tail failed".into()))?;
        feed(&tail);
    }

    Ok(format!("c:{:016x}", hash))
}

/// Compute a content-based hash for video files.
///
/// Does not perform perceptual hashing (video files are too large, decoding too slow).
/// Instead, hashes file size + first 64KB + last 64KB of content.
/// Sufficient to distinguish the vast majority of video files, runs in <1ms.
///
/// Returns format: `"video:xxxxxxxxxxxxxxxx"`
pub fn compute_video_hash(path: &Path) -> Result<String, CoreError> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::io::{Read, Seek, SeekFrom};

    let file = std::fs::File::open(path).map_err(|_| CoreError::Io("open failed".into()))?;
    let file_size = file
        .metadata()
        .map_err(|_| CoreError::Io("metadata failed".into()))?
        .len();

    let mut hasher = DefaultHasher::new();
    file_size.hash(&mut hasher);

    let mut reader = std::io::BufReader::new(file);
    let chunk_size = 64 * 1024usize; // 64KB

    // Read head 64KB (or less for small files)
    let read_size = chunk_size.min(file_size as usize);
    let mut head_buf = vec![0u8; read_size];
    reader
        .read_exact(&mut head_buf)
        .map_err(|_| CoreError::Io("read head failed".into()))?;
    head_buf.hash(&mut hasher);

    // Read tail 64KB (only if file is large enough that head and tail don't overlap)
    if file_size > (chunk_size as u64) * 2 {
        reader
            .seek(SeekFrom::End(-(chunk_size as i64)))
            .map_err(|_| CoreError::Io("seek tail failed".into()))?;
        let mut tail_buf = vec![0u8; chunk_size];
        reader
            .read_exact(&mut tail_buf)
            .map_err(|_| CoreError::Io("read tail failed".into()))?;
        tail_buf.hash(&mut hasher);
    }

    Ok(format!("video:{:016x}", hasher.finish()))
}

/// Extract embedded JPEG thumbnail data from EXIF.
/// Handles both JPEG and TIFF-based (RAW) files.
/// Optimized: reads first 256KB into memory to avoid slow disk seeks on external drives.
fn try_extract_thumbnail_data(path: &Path) -> Result<Vec<u8>, CoreError> {
    // Read first 256KB into memory for fast EXIF parsing
    let file = std::fs::File::open(path).map_err(|_| CoreError::Io("open failed".into()))?;
    let mut buf_reader = BufReader::with_capacity(256 * 1024, file);
    let mut header_buf = vec![0u8; 256 * 1024];
    let bytes_read = buf_reader.read(&mut header_buf).unwrap_or(0);
    header_buf.truncate(bytes_read);
    let exif = crate::metadata::exif::read_container(&mut Cursor::new(&header_buf))
        .map_err(|_| CoreError::Io("no exif".into()))?;

    let thumbnail_field = exif
        .get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)
        .ok_or_else(|| CoreError::Io("no thumb offset".into()))?;
    let length_field = exif
        .get_field(exif::Tag::JPEGInterchangeFormatLength, exif::In::THUMBNAIL)
        .ok_or_else(|| CoreError::Io("no thumb length".into()))?;

    let tiff_offset = thumbnail_field
        .value
        .get_uint(0)
        .ok_or_else(|| CoreError::Io("invalid offset".into()))? as usize;
    let length = length_field
        .value
        .get_uint(0)
        .ok_or_else(|| CoreError::Io("invalid length".into()))? as usize;

    // Calculate absolute offset based on file type
    let absolute_offset = if is_tiff_based(path) {
        // TIFF-based (RAW): offset is relative to file start
        tiff_offset as u64
    } else {
        // JPEG: offset is relative to TIFF header in APP1 segment
        find_jpeg_tiff_header_offset(path)? + tiff_offset as u64
    };

    // Check if thumbnail is within our buffer
    let end_offset = absolute_offset as usize + length;
    if end_offset <= header_buf.len() {
        // Fast path: thumbnail is in buffer
        return Ok(header_buf[absolute_offset as usize..end_offset].to_vec());
    }

    // Slow path: need to read from file
    let file = std::fs::File::open(path).map_err(|_| CoreError::Io("open failed".into()))?;
    let mut reader = BufReader::new(file);

    reader
        .seek(std::io::SeekFrom::Start(absolute_offset))
        .map_err(|_| CoreError::Io("seek failed".into()))?;

    let mut thumb_data = vec![0u8; length];
    reader
        .read_exact(&mut thumb_data)
        .map_err(|_| CoreError::Io("read thumb failed".into()))?;

    Ok(thumb_data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_compute_video_hash_returns_video_prefix() {
        let dir = std::env::temp_dir().join("footos_video_hash_test");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("test.mp4");

        // Create a file with some content
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(&vec![0xAA; 1024]).unwrap();

        let hash = compute_video_hash(&file_path).unwrap();
        assert!(
            hash.starts_with("video:"),
            "Hash should start with 'video:' prefix, got: {}",
            hash
        );
        assert_eq!(
            hash.len(),
            "video:".len() + 16,
            "Hash hex part should be 16 chars"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compute_video_hash_deterministic() {
        let dir = std::env::temp_dir().join("footos_video_hash_det_test");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("det.mp4");

        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(&vec![0xBB; 2048]).unwrap();

        let hash1 = compute_video_hash(&file_path).unwrap();
        let hash2 = compute_video_hash(&file_path).unwrap();
        assert_eq!(hash1, hash2, "Same file should produce same hash");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compute_video_hash_different_content() {
        let dir = std::env::temp_dir().join("footos_video_hash_diff_test");
        let _ = std::fs::create_dir_all(&dir);

        let file_a = dir.join("a.mp4");
        let file_b = dir.join("b.mp4");

        let mut fa = std::fs::File::create(&file_a).unwrap();
        fa.write_all(&vec![0xCC; 1024]).unwrap();

        let mut fb = std::fs::File::create(&file_b).unwrap();
        fb.write_all(&vec![0xDD; 1024]).unwrap();

        let hash_a = compute_video_hash(&file_a).unwrap();
        let hash_b = compute_video_hash(&file_b).unwrap();
        assert_ne!(
            hash_a, hash_b,
            "Different content should produce different hashes"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compute_video_hash_large_file_reads_tail() {
        let dir = std::env::temp_dir().join("footos_video_hash_large_test");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("large.mp4");

        // Create a file larger than 128KB so tail chunk is read
        let mut f = std::fs::File::create(&file_path).unwrap();
        let size = 200 * 1024; // 200KB
        let mut data = vec![0x00; size];
        // Put distinct data at the tail
        for i in (size - 64 * 1024)..size {
            data[i] = 0xFF;
        }
        f.write_all(&data).unwrap();

        let hash = compute_video_hash(&file_path).unwrap();
        assert!(hash.starts_with("video:"));

        // Modify only the tail — hash should change
        data[size - 1] = 0x01;
        std::fs::write(&file_path, &data).unwrap();
        let hash2 = compute_video_hash(&file_path).unwrap();
        assert_ne!(hash, hash2, "Changing tail should change hash");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compute_video_hash_nonexistent_file() {
        let result = compute_video_hash(Path::new("/nonexistent/video.mp4"));
        assert!(result.is_err());
    }
}
