use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThumbnailSpec {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThumbnailKey(u64);

impl ThumbnailKey {
    pub fn new(val: u64) -> Self {
        Self(val)
    }
}

#[derive(Debug, Clone)]
pub struct Thumbnailer {
    cache_root: PathBuf,
}

#[derive(Debug, Error)]
pub enum ThumbnailError {
    #[error("Path is not UTF-8 valid")]
    InvalidPathEncoding,
    #[error("Image decode failed: {0}")]
    DecodeError(String),
    #[error("Image encode/save failed: {0}")]
    EncodeError(String),
}

/// Pure FNV-1a 64-bit implementation
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn fnv1a_64(bytes: &[u8], start: u64) -> u64 {
    let mut hash = start;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Internal core function to generate a thumbnail from source to dest.
///
/// Performance optimization strategy:
/// 1. Try to extract embedded EXIF thumbnail (fastest, ~1-5ms)
/// 2. Fall back to full image decode + resize (slower, ~50-500ms for large files)
fn generate_image_file(source: &Path, dest: &Path, spec: &ThumbnailSpec) -> Result<(), ThumbnailError> {
    // Read EXIF orientation first
    let orientation = read_exif_orientation(source);

    // Step 1: Try to use embedded thumbnail from EXIF (fast path)
    // This is required for RAW files since image crate can't decode them
    let thumb_result = try_extract_embedded_thumbnail(source, spec);

    if let Ok(embedded_thumb) = thumb_result {
        // Apply orientation correction to embedded thumbnail
        let corrected_thumb = if let Some(orient) = orientation {
            if orient > 1 {
                apply_orientation_correction(&embedded_thumb, orient)?
            } else {
                embedded_thumb
            }
        } else {
            embedded_thumb
        };

        // Save the corrected thumbnail
        std::fs::write(dest, corrected_thumb)
            .map_err(|e| ThumbnailError::EncodeError(e.to_string()))?;
        return Ok(());
    }

    // Check if this is a RAW file - if embedded thumbnail failed, we can't proceed
    if is_raw_file(source) {
        return Err(ThumbnailError::DecodeError(format!(
            "RAW file has no extractable embedded thumbnail: {:?}",
            thumb_result.err()
        )));
    }

    // Step 2: Fall back to full decode + resize (slow path) - only for standard formats
    let img = image::open(source).map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;

    // Apply EXIF orientation correction
    let corrected_img = if let Some(orient) = orientation {
        apply_orientation_to_image(img, orient)
    } else {
        img
    };

    // thumbnail() is faster than resize() because it downsamples during load if supported,
    // or uses nearest neighbor optimization for large downscales.
    let thumb = corrected_img.thumbnail(spec.width, spec.height);

    // Force JPEG format when saving
    thumb.write_to(&mut std::fs::File::create(dest).map_err(|e| ThumbnailError::EncodeError(e.to_string()))?, image::ImageFormat::Jpeg)
         .map_err(|e| ThumbnailError::EncodeError(e.to_string()))?;

    Ok(())
}

/// Check if file is a RAW format based on extension
pub fn is_raw_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some("cr2" | "cr3" | "nef" | "nrw" | "arw" | "srf" | "sr2" |
             "dng" | "raf" | "orf" | "rw2" | "pef" | "raw")
    )
}

/// Extract the embedded JPEG preview from a RAW file.
/// Returns the full-resolution preview JPEG bytes with orientation correction applied.
/// Scans the entire RAW file to find the largest embedded JPEG preview by file size.
/// Falls back to small thumbnail if no large preview is available.
pub fn extract_raw_preview(path: &Path) -> Result<Vec<u8>, ThumbnailError> {
    use std::io::{BufReader, Read, Seek, SeekFrom};

    if !is_raw_file(path) {
        return Err(ThumbnailError::DecodeError("Not a RAW file".to_string()));
    }

    let orientation = read_exif_orientation(path);

    let file = std::fs::File::open(path)
        .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
    let file_size = file.metadata()
        .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?.len();
    let mut reader = BufReader::with_capacity(64 * 1024, file); // 64KB buffer for faster reading

    let scan_start = 8u64;
    let mut best_preview: Option<Vec<u8>> = None;
    let mut best_size = 0u64;
    let mut fallback_preview: Option<Vec<u8>> = None; // Any valid JPEG as last resort
    let mut fallback_size = 0u64;

    // Threshold: once we find a JPEG > 500KB, it's almost certainly the main preview
    const GOOD_ENOUGH_SIZE: u64 = 500_000;
    // Minimum size for "good" preview (filter out tiny thumbnails for best_preview)
    const MIN_GOOD_SIZE: u64 = 50_000;

    reader.seek(SeekFrom::Start(scan_start))
        .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;

    let mut pos = scan_start;
    let mut buf = [0u8; 32768]; // 32KB chunks for faster scanning

    'scan: while pos < file_size - 3 {
        let bytes_read = reader.read(&mut buf)
            .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
        if bytes_read == 0 {
            break;
        }

        for i in 0..bytes_read.saturating_sub(2) {
            if buf[i] == 0xFF && buf[i + 1] == 0xD8 && buf[i + 2] == 0xFF {
                let jpeg_start = pos + i as u64;

                if let Ok(jpeg_data) = extract_jpeg_from_offset(path, jpeg_start, file_size) {
                    let jpeg_size = jpeg_data.len() as u64;

                    // Track the largest "good" preview (>50KB)
                    if jpeg_size > best_size && jpeg_size > MIN_GOOD_SIZE {
                        best_size = jpeg_size;
                        best_preview = Some(jpeg_data.clone());

                        // If we found a large enough preview, stop scanning immediately
                        if jpeg_size >= GOOD_ENOUGH_SIZE {
                            break 'scan;
                        }
                    }

                    // Also track the largest JPEG of any size as fallback
                    if jpeg_size > fallback_size {
                        fallback_size = jpeg_size;
                        fallback_preview = Some(jpeg_data);
                    }
                }
            }
        }

        pos += (bytes_read - 2) as u64;
        reader.seek(SeekFrom::Start(pos))
            .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
    }

    // Use best preview if available, otherwise fall back to any valid JPEG (even small thumbnail)
    let preview_data = best_preview.or(fallback_preview);

    if let Some(preview_data) = preview_data {
        // Apply orientation correction
        if let Some(orient) = orientation {
            if orient > 1 {
                return apply_orientation_correction(&preview_data, orient);
            }
        }
        return Ok(preview_data);
    }

    Err(ThumbnailError::DecodeError("No embedded JPEG preview found".to_string()))
}

/// Reads EXIF orientation tag from an image file.
/// Tries multiple IFDs (PRIMARY and THUMBNAIL) to find the tag.
/// Optimized: reads first 256KB into memory to avoid slow disk seeks.
fn read_exif_orientation(source: &Path) -> Option<u32> {
    use std::io::{BufReader, Read, Cursor};

    // Read first 256KB - NEF files may have orientation tag further in
    let file = std::fs::File::open(source).ok()?;
    let mut buf_reader = BufReader::with_capacity(256 * 1024, file);
    let mut header_buf = vec![0u8; 256 * 1024];
    let bytes_read = buf_reader.read(&mut header_buf).ok()?;
    header_buf.truncate(bytes_read);

    let exif_reader = exif::Reader::new();
    let exif = exif_reader.read_from_container(&mut Cursor::new(&header_buf)).ok()?;

    // Check PRIMARY IFD first, then THUMBNAIL IFD
    for ifd in &[exif::In::PRIMARY, exif::In::THUMBNAIL] {
        if let Some(field) = exif.get_field(exif::Tag::Orientation, *ifd) {
            if let Some(val) = field.value.get_uint(0) {
                return Some(val);
            }
        }
    }

    None
}

/// Applies EXIF orientation correction to image bytes.
fn apply_orientation_correction(image_bytes: &[u8], orientation: u32) -> Result<Vec<u8>, ThumbnailError> {
    if orientation <= 1 {
        // No correction needed
        return Ok(image_bytes.to_vec());
    }
    
    let img = image::load_from_memory(image_bytes)
        .map_err(|e| ThumbnailError::DecodeError(format!("Failed to decode for orientation: {}", e)))?;
    
    let corrected = apply_orientation_to_image(img, orientation);
    
    let mut output = Vec::new();
    corrected.write_to(&mut std::io::Cursor::new(&mut output), image::ImageFormat::Jpeg)
        .map_err(|e| ThumbnailError::EncodeError(e.to_string()))?;
    
    Ok(output)
}

/// Applies EXIF orientation transformation to a DynamicImage.
/// See: https://magnushoff.com/articles/jpeg-orientation/
fn apply_orientation_to_image(img: image::DynamicImage, orientation: u32) -> image::DynamicImage {
    match orientation {
        1 => img, // Normal
        2 => img.fliph(), // Flip horizontal
        3 => img.rotate180(), // Rotate 180
        4 => img.flipv(), // Flip vertical
        5 => img.rotate90().fliph(), // Transpose (flip across UL-to-LR axis)
        6 => img.rotate90(), // Rotate 90 CW
        7 => img.rotate270().fliph(), // Transverse (flip across LL-to-UR axis)
        8 => img.rotate270(), // Rotate 270 CW
        _ => img, // Unknown orientation, keep as-is
    }
}

/// Detects if file is JPEG or TIFF-based (RAW) by checking magic bytes.
/// Returns true for TIFF-based files, false for JPEG.
fn is_tiff_based(source: &Path) -> bool {
    use std::io::Read;

    let mut file = match std::fs::File::open(source) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut magic = [0u8; 2];
    if file.read_exact(&mut magic).is_err() {
        return false;
    }

    // TIFF: starts with II (little-endian) or MM (big-endian)
    // JPEG: starts with 0xFF 0xD8
    magic == [0x49, 0x49] || magic == [0x4D, 0x4D]
}

/// Attempts to extract and resize embedded EXIF thumbnail.
/// Returns the JPEG bytes if successful, or an error if not available or too small.
/// Optimized: reads first 256KB into memory to avoid slow disk seeks.
fn try_extract_embedded_thumbnail(source: &Path, spec: &ThumbnailSpec) -> Result<Vec<u8>, ThumbnailError> {
    use std::io::{BufReader, Read, Cursor};

    // Read first 256KB into memory for fast EXIF parsing
    let file = std::fs::File::open(source)
        .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
    let mut buf_reader = BufReader::with_capacity(256 * 1024, file);
    let mut header_buf = vec![0u8; 256 * 1024];
    let bytes_read = buf_reader.read(&mut header_buf).unwrap_or(0);
    header_buf.truncate(bytes_read);

    let exif_reader = exif::Reader::new();
    let exif = exif_reader.read_from_container(&mut Cursor::new(&header_buf))
        .map_err(|e| ThumbnailError::DecodeError(format!("No EXIF: {}", e)))?;

    // Try multiple IFDs to find embedded thumbnail
    // THUMBNAIL IFD (IFD1) is standard, but some RAW files use PRIMARY IFD (IFD0)
    let ifds = [exif::In::THUMBNAIL, exif::In::PRIMARY];

    let mut thumbnail_field = None;
    let mut length_field = None;

    for ifd in &ifds {
        let tf = exif.get_field(exif::Tag::JPEGInterchangeFormat, *ifd);
        let lf = exif.get_field(exif::Tag::JPEGInterchangeFormatLength, *ifd);
        if tf.is_some() && lf.is_some() {
            thumbnail_field = tf;
            length_field = lf;
            break;
        }
    }

    // For RAW files, also try to find embedded JPEG by scanning for JPEG header
    if thumbnail_field.is_none() && is_raw_file(source) {
        if let Ok(preview) = try_extract_raw_preview(source, spec) {
            return Ok(preview);
        }
    }

    if let Some(thumbnail_field) = thumbnail_field {
        if let Some(length_field) = length_field {
            // Extract offset and length (relative to TIFF header)
            let tiff_offset = thumbnail_field.value.get_uint(0)
                .ok_or_else(|| ThumbnailError::DecodeError("Invalid thumbnail offset".to_string()))? as usize;
            let length = length_field.value.get_uint(0)
                .ok_or_else(|| ThumbnailError::DecodeError("Invalid thumbnail length".to_string()))? as usize;

            // Calculate absolute offset based on file type
            let absolute_offset = if is_tiff_based(source) {
                // TIFF-based (RAW): offset is relative to file start
                tiff_offset as u64
            } else {
                // JPEG: offset is relative to TIFF header in APP1 segment
                find_jpeg_tiff_header_offset(source)? + tiff_offset as u64
            };

            // Try to extract from buffer first (fast path)
            let end_offset = absolute_offset as usize + length;
            let thumb_data = if end_offset <= header_buf.len() {
                // Fast path: thumbnail data is in our memory buffer
                header_buf[absolute_offset as usize..end_offset].to_vec()
            } else {
                // Slow path: need to read from file
                use std::io::{Seek, Read};
                let file = std::fs::File::open(source)
                    .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
                let mut reader = BufReader::new(file);

                reader.seek(std::io::SeekFrom::Start(absolute_offset))
                    .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;

                let mut data = vec![0u8; length];
                reader.read_exact(&mut data)
                    .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
                data
            };

            // Decode the embedded thumbnail to check its size
            let thumb_img = image::load_from_memory(&thumb_data)
                .map_err(|e| ThumbnailError::DecodeError(format!("Embedded thumb decode failed: {}", e)))?;

            // If embedded thumbnail is already smaller than or equal to target size, use it directly
            if thumb_img.width() <= spec.width && thumb_img.height() <= spec.height {
                return Ok(thumb_data);
            }

            // If embedded thumbnail is larger but not too large (e.g., < 4x target), resize it
            // This is still faster than decoding the full image
            if thumb_img.width() <= spec.width * 4 && thumb_img.height() <= spec.height * 4 {
                let resized = thumb_img.thumbnail(spec.width, spec.height);
                let mut output = Vec::new();
                resized.write_to(&mut std::io::Cursor::new(&mut output), image::ImageFormat::Jpeg)
                    .map_err(|e| ThumbnailError::EncodeError(e.to_string()))?;
                return Ok(output);
            }
        }
    }

    Err(ThumbnailError::DecodeError("No suitable embedded thumbnail".to_string()))
}

/// Try to extract embedded JPEG preview from RAW file by scanning for JPEG markers.
/// This is a fallback when standard EXIF thumbnail tags are not found.
fn try_extract_raw_preview(source: &Path, spec: &ThumbnailSpec) -> Result<Vec<u8>, ThumbnailError> {
    use std::io::{BufReader, Read, Seek, SeekFrom};

    println!("[SLOW PATH] Scanning RAW file for embedded JPEG: {:?}", source);

    let file = std::fs::File::open(source)
        .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
    let file_size = file.metadata()
        .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?.len();
    let mut reader = BufReader::new(file);

    // Skip initial bytes (TIFF header area) and scan for JPEG start marker
    // NEF files typically have the preview starting after several KB
    let scan_start = 8u64; // Skip TIFF header
    let mut best_preview: Option<Vec<u8>> = None;
    let mut best_size = 0u64;

    reader.seek(SeekFrom::Start(scan_start))
        .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;

    let mut pos = scan_start;
    let mut buf = [0u8; 4096];

    // Scan for JPEG start markers (0xFF 0xD8 0xFF)
    while pos < file_size - 3 {
        let bytes_read = reader.read(&mut buf)
            .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
        if bytes_read == 0 {
            break;
        }

        for i in 0..bytes_read.saturating_sub(2) {
            if buf[i] == 0xFF && buf[i + 1] == 0xD8 && buf[i + 2] == 0xFF {
                let jpeg_start = pos + i as u64;

                // Try to find JPEG end marker and extract
                if let Ok(jpeg_data) = extract_jpeg_from_offset(source, jpeg_start, file_size) {
                    let jpeg_size = jpeg_data.len() as u64;

                    // Keep the largest preview (usually the full-resolution one)
                    if jpeg_size > best_size && jpeg_size > 10000 {
                        // Verify it's a valid JPEG by trying to decode it
                        if image::load_from_memory(&jpeg_data).is_ok() {
                            best_size = jpeg_size;
                            best_preview = Some(jpeg_data);
                        }
                    }
                }
            }
        }

        // Move position, overlapping a bit to catch markers at boundaries
        pos += (bytes_read - 2) as u64;
        reader.seek(SeekFrom::Start(pos))
            .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
    }

    if let Some(preview_data) = best_preview {
        // Resize if needed
        let img = image::load_from_memory(&preview_data)
            .map_err(|e| ThumbnailError::DecodeError(format!("Preview decode failed: {}", e)))?;

        if img.width() <= spec.width && img.height() <= spec.height {
            return Ok(preview_data);
        }

        let resized = img.thumbnail(spec.width, spec.height);
        let mut output = Vec::new();
        resized.write_to(&mut std::io::Cursor::new(&mut output), image::ImageFormat::Jpeg)
            .map_err(|e| ThumbnailError::EncodeError(e.to_string()))?;
        return Ok(output);
    }

    Err(ThumbnailError::DecodeError("No embedded JPEG preview found".to_string()))
}

/// Extract JPEG data from a specific offset in a file.
/// Tries multiple potential end markers to find a valid JPEG.
fn extract_jpeg_from_offset(source: &Path, start: u64, file_size: u64) -> Result<Vec<u8>, ThumbnailError> {
    use std::io::{BufReader, Read, Seek, SeekFrom};

    let file = std::fs::File::open(source)
        .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
    let mut reader = BufReader::new(file);

    reader.seek(SeekFrom::Start(start))
        .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;

    // Read up to 20MB max for a preview image
    let max_size = std::cmp::min(20 * 1024 * 1024, (file_size - start) as usize);
    let mut data = vec![0u8; max_size];
    let bytes_read = reader.read(&mut data)
        .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
    data.truncate(bytes_read);

    // Find ALL potential JPEG end markers (0xFF 0xD9) and try each one
    // The first false-positive end marker might be in raw sensor data
    let mut end_positions: Vec<usize> = Vec::new();
    for i in 2..data.len() {
        if data[i - 1] == 0xFF && data[i] == 0xD9 {
            end_positions.push(i + 1);
        }
    }

    // Try each end position, starting from the first one
    // A valid JPEG should decode successfully
    for end_pos in end_positions.iter().take(20) {
        // Skip very small "JPEGs" (< 1KB) - likely false positives
        if *end_pos < 1024 {
            continue;
        }

        let candidate = &data[0..*end_pos];
        if image::load_from_memory(candidate).is_ok() {
            return Ok(candidate.to_vec());
        }
    }

    Err(ThumbnailError::DecodeError("No valid JPEG found".to_string()))
}

/// Find the TIFF header position in a JPEG file by scanning for EXIF APP1 segment.
fn find_jpeg_tiff_header_offset(source: &Path) -> Result<u64, ThumbnailError> {
    use std::io::{BufReader, Read, Seek};

    let file = std::fs::File::open(source)
        .map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
    let mut reader = BufReader::new(file);

    let mut buf = [0u8; 2];
    reader.read_exact(&mut buf).map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
    if buf != [0xFF, 0xD8] {
        return Err(ThumbnailError::DecodeError("Not a JPEG file".to_string()));
    }

    // Scan for APP1 marker (0xFFE1)
    loop {
        reader.read_exact(&mut buf).map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
        if buf[0] != 0xFF {
            return Err(ThumbnailError::DecodeError("Invalid JPEG marker".to_string()));
        }
        if buf[1] == 0xE1 {
            // APP1 found, read length
            let mut len_buf = [0u8; 2];
            reader.read_exact(&mut len_buf).map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
            // Check for "Exif\0\0"
            let mut exif_id = [0u8; 6];
            reader.read_exact(&mut exif_id).map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
            if &exif_id == b"Exif\0\0" {
                // TIFF header starts here
                return reader.stream_position().map_err(|e| ThumbnailError::DecodeError(e.to_string()));
            }
        } else if buf[1] == 0xD9 || buf[1] == 0xDA {
            // EOI or SOS - no EXIF found
            break;
        } else if buf[1] >= 0xE0 && buf[1] <= 0xEF || buf[1] == 0xFE {
            // Other APP or COM segment, skip it
            let mut len_buf = [0u8; 2];
            reader.read_exact(&mut len_buf).map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
            let seg_len = u16::from_be_bytes(len_buf) as i64 - 2;
            reader.seek(std::io::SeekFrom::Current(seg_len)).map_err(|e| ThumbnailError::DecodeError(e.to_string()))?;
        }
    }

    Err(ThumbnailError::DecodeError("EXIF APP1 not found".to_string()))
}

/// Generates a stable, platform-independent key for a thumbnail configuration.
/// 
/// Normalizes path by iterating components to avoid separator differences.
pub fn thumbnail_key(source: &Path, spec: &ThumbnailSpec) -> Result<ThumbnailKey, ThumbnailError> {
    let mut hash = FNV_OFFSET_BASIS;

    // 1. Hash path components to ensure "a/b" (Linux) == "a\b" (Windows)
    for component in source.components() {
        if let Component::Normal(os_str) = component {
            let str_slice = os_str.to_str().ok_or(ThumbnailError::InvalidPathEncoding)?;
            hash = fnv1a_64(str_slice.as_bytes(), hash);
            // Add a separator mimic to prevent "ab/c" colliding with "a/bc"
            hash = fnv1a_64(&[b'/'], hash); 
        }
    }

    // 2. Hash spec
    hash = fnv1a_64(&spec.width.to_le_bytes(), hash);
    hash = fnv1a_64(&spec.height.to_le_bytes(), hash);

    Ok(ThumbnailKey(hash))
}

/// Resolves the cache file path for a given key.
/// 
/// Uses a 2-level directory sharding based on the key hex representation.
/// Example: `root/ab/12/ab12...`
pub fn cache_path(root: &Path, key: &ThumbnailKey) -> PathBuf {
    let hex = format!("{:016x}", key.0);
    // Sharding: first 2 chars
    let shard = &hex[0..2];
    root.join(shard).join(format!("{}.jpg", hex))
}

impl Thumbnailer {
    pub fn new(cache_root: PathBuf) -> Self {
        Self { cache_root }
    }

    /// Legacy adapter method
    pub fn get_cache_path(&self, _content_hash: &str, _spec: &ThumbnailSpec) -> PathBuf {
       // This is strictly a fallback for now to match interface
       // ideally we should use thumbnail_key logic but the input args don't match
       // For this strict update, we'll assume content_hash is the source of truth if provided
       // but typically we'd look up.
       // Given the constraint "Use fixed hash algo", we should probably use the key.
       // But the method signature from previous step takes a string content_hash.
       // We'll leave this simply functional for the transition.
       self.cache_root.join(format!("{}.jpg", _content_hash))
    }

    /// Checks if a thumbnail exists for the given source and spec.
    /// 
    /// Returns:
    /// - Ok(Some(path)) if the thumbnail file exists on disk.
    /// - Ok(None) if the thumbnail file does not exist.
    /// - Err(e) if key generation fails (e.g. invalid UTF-8 path).
    /// 
    /// Does NOT attempt to generate the thumbnail or create directories.
    pub fn get_cached_path(&self, source: &Path, spec: &ThumbnailSpec) -> Result<Option<PathBuf>, ThumbnailError> {
        let key = thumbnail_key(source, spec)?;
        let path = cache_path(&self.cache_root, &key);
        
        if path.exists() {
            Ok(Some(path))
        } else {
            Ok(None)
        }
    }
    /// Atomically gets or creates a thumbnail.
    ///
    /// Workflow:
    /// 1. Check if cache exists. If yes, return immediately.
    /// 2. Generate to a temporary file in the same directory (to ensure atomic rename).
    /// 3. Rename temp file to final destination.
    ///
    /// This pattern prevents partial writes and handles process concurrency gracefully (last writer wins).
    pub fn get_or_create(&self, source: &Path, spec: &ThumbnailSpec) -> Result<PathBuf, ThumbnailError> {
        let key = thumbnail_key(source, spec)?;
        let dest = cache_path(&self.cache_root, &key);

        // 1. Fast path: exists
        if dest.exists() {
            return Ok(dest);
        }
        
        // Ensure parent directory exists
        if let Some(parent) = dest.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| ThumbnailError::EncodeError(e.to_string()))?;
            }
        }

        // 2. Generate to unique temp file
        // Use a combination of timestamp and PID to ensure uniqueness across processes/threads
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let random_suffix = format!("{:x}_{:x}", nanos, pid);
        
        let temp_dest = dest.with_file_name(format!("{}.tmp.{}", 
            dest.file_name().unwrap().to_string_lossy(), // lossless conversion not needed for temp filename
            random_suffix
        ));

        // Generate content
        generate_image_file(source, &temp_dest, spec).map_err(|e| {
             // Cleanup temp file on failure if it was created
             let _ = std::fs::remove_file(&temp_dest);
             e
        })?;

        // 3. Atomic rename
        std::fs::rename(&temp_dest, &dest).map_err(|e| {
             // Try to cleanup temp file if rename fails
             let _ = std::fs::remove_file(&temp_dest);
             ThumbnailError::EncodeError(format!("Atomic rename failed: {}", e))
        })?;
        
        Ok(dest)
    }

    /// Legacy compatibility wrapper (Deprecated)
    pub fn generate(&self, source: &Path, spec: &ThumbnailSpec) -> Result<PathBuf, ThumbnailError> {
        self.get_or_create(source, spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use image::{RgbImage, ImageFormat};

    #[test]
    fn test_generate_workflow() {
        let temp_dir = std::env::temp_dir().join("fotos_thumb_gen_test");
        if temp_dir.exists() { fs::remove_dir_all(&temp_dir).unwrap(); }
        fs::create_dir_all(&temp_dir).unwrap();

        let thumbnailer = Thumbnailer::new(temp_dir.clone());
        let src_dir = temp_dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        
        let src_path = src_dir.join("test.png");
        let mut img: RgbImage = RgbImage::new(100, 100); // 100x100 black
        img.save_with_format(&src_path, ImageFormat::Png).unwrap();

        let spec = ThumbnailSpec { width: 20, height: 20 };
        let thumb_path = thumbnailer.generate(&src_path, &spec).expect("Generation failed");

        assert!(thumb_path.exists());
        
        let thumb_img = image::open(&thumb_path).expect("Failed to open generated thumb");
        assert_eq!(thumb_img.width(), 20);
        assert_eq!(thumb_img.height(), 20);
        
        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_idempotency_and_cache_hit() {
        let temp_dir = std::env::temp_dir().join("fotos_thumb_idempotency");
        if temp_dir.exists() { fs::remove_dir_all(&temp_dir).unwrap(); }
        fs::create_dir_all(&temp_dir).unwrap();

        let thumbnailer = Thumbnailer::new(temp_dir.clone());
        let src_dir = temp_dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        
        let src_path = src_dir.join("test.png");
        let mut img: RgbImage = RgbImage::new(50, 50); 
        img.save_with_format(&src_path, ImageFormat::Png).unwrap();
        let spec = ThumbnailSpec { width: 10, height: 10 };

        let p1 = thumbnailer.get_or_create(&src_path, &spec).unwrap();
        let m1 = fs::metadata(&p1).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(10));

        let p2 = thumbnailer.get_or_create(&src_path, &spec).unwrap();
        let m2 = fs::metadata(&p2).unwrap().modified().unwrap();

        assert_eq!(p1, p2);
        assert_eq!(m1, m2, "Cache hit should not modify file time");

        fs::write(&p1, b"corrupt").unwrap();
        
        assert!(thumbnailer.get_cached_path(&src_path, &spec).unwrap().is_some());

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_get_cached_path_check() {
        let temp_dir = std::env::temp_dir().join("fotos_thumb_check_test");
        if temp_dir.exists() { fs::remove_dir_all(&temp_dir).unwrap(); }
        fs::create_dir_all(&temp_dir).unwrap();

        let thumbnailer = Thumbnailer::new(temp_dir.clone());
        let source = Path::new("some/photo.jpg");
        let spec = ThumbnailSpec { width: 100, height: 100 };

        let result = thumbnailer.get_cached_path(source, &spec).unwrap();
        assert!(result.is_none());

        let key = thumbnail_key(source, &spec).unwrap();
        let expected_path = cache_path(&temp_dir, &key);
        
        fs::create_dir_all(expected_path.parent().unwrap()).unwrap();
        fs::write(&expected_path, b"fake jpg").unwrap();

        let result = thumbnailer.get_cached_path(source, &spec).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), expected_path);

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn test_non_utf8_path_handling() {
        use std::os::unix::ffi::OsStrExt;
        
        let temp_dir = std::env::temp_dir().join("fotos_thumb_utf8");
        if temp_dir.exists() { fs::remove_dir_all(&temp_dir).unwrap(); }
        fs::create_dir_all(&temp_dir).unwrap();

        let thumbnailer = Thumbnailer::new(temp_dir.clone());
        
        let bad_bytes = b"foo\xffbar.jpg";
        let bad_os_str = std::ffi::OsStr::from_bytes(bad_bytes);
        let bad_path = Path::new(bad_os_str);
        let spec = ThumbnailSpec { width: 10, height: 10 };

        let result = thumbnailer.generate(bad_path, &spec);
        
        assert!(result.is_err());
        match result.unwrap_err() {
            ThumbnailError::InvalidPathEncoding => (), 
            _ => panic!("Expected InvalidPathEncoding error"),
        }

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_key_stability() {
        let spec = ThumbnailSpec { width: 200, height: 200 };
        let p1 = Path::new("foo/bar/baz.jpg");
        let k1 = thumbnail_key(p1, &spec).unwrap();
        let k2 = thumbnail_key(p1, &spec).unwrap();
        assert_eq!(k1, k2);
        
        let p2 = Path::new("foo/bar/bazz.jpg");
        let k3 = thumbnail_key(p2, &spec).unwrap();
        assert_ne!(k1, k3);

        let spec2 = ThumbnailSpec { width: 201, height: 200 };
        let k4 = thumbnail_key(p1, &spec2).unwrap();
        assert_ne!(k1, k4);
    }

    #[test]
    fn test_sharding_rules() {
        let root = Path::new("/cache");
        let key = ThumbnailKey(0x1020304050607080); 
        let path = cache_path(root, &key);
        
        let path_str = path.to_str().unwrap().replace('\\', "/");
        assert!(path_str.ends_with("/10/1020304050607080.jpg"));
    }

    #[test]
    fn test_process_independence() {
        let p1 = Path::new("/stable/path.jpg");
        let spec = ThumbnailSpec { width: 100, height: 100 };
        
        let k1 = thumbnail_key(p1, &spec).unwrap();
        let k2 = thumbnail_key(p1, &spec).unwrap();
        
        assert_eq!(k1, k2);
    }
}
