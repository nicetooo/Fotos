use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use crate::error::CoreError;
use crate::types::PhotoMetadata;
use exif::{In, Tag, Reader, Value};

/// Reads comprehensive EXIF metadata from a photo.
pub fn read_metadata(path: &Path) -> Result<PhotoMetadata, CoreError> {
    let file = File::open(path)?;
    let mut buf_reader = BufReader::new(file);

    let mut metadata = PhotoMetadata::default();

    // EXIF Parsing (get dimensions from EXIF instead of decoding image header)
    let exif_reader = Reader::new();
    let exif = match exif_reader.read_from_container(&mut buf_reader) {
        Ok(exif) => exif,
        Err(_) => return Ok(metadata), // Return partial metadata if EXIF fails
    };

    // Get dimensions from EXIF (much faster than image::ImageReader)
    if let Some(field) = exif.get_field(Tag::PixelXDimension, In::PRIMARY) {
        if let Some(w) = field.value.get_uint(0) {
            metadata.width = w;
        }
    }
    if let Some(field) = exif.get_field(Tag::PixelYDimension, In::PRIMARY) {
        if let Some(h) = field.value.get_uint(0) {
            metadata.height = h;
        }
    }
    // Fallback to ImageWidth/ImageLength if PixelXDimension not available
    if metadata.width == 0 {
        if let Some(field) = exif.get_field(Tag::ImageWidth, In::PRIMARY) {
            if let Some(w) = field.value.get_uint(0) {
                metadata.width = w;
            }
        }
    }
    if metadata.height == 0 {
        if let Some(field) = exif.get_field(Tag::ImageLength, In::PRIMARY) {
            if let Some(h) = field.value.get_uint(0) {
                metadata.height = h;
            }
        }
    }

    // Device & Time
    if let Some(field) = exif.get_field(Tag::Make, In::PRIMARY) {
        metadata.make = Some(field.display_value().with_unit(&exif).to_string());
    }
    if let Some(field) = exif.get_field(Tag::Model, In::PRIMARY) {
        metadata.model = Some(field.display_value().with_unit(&exif).to_string());
    }
    if let Some(field) = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY) {
        metadata.date_taken = Some(field.display_value().with_unit(&exif).to_string());
    }

    // Exposure Parameters
    if let Some(field) = exif.get_field(Tag::PhotographicSensitivity, In::PRIMARY) {
        match &field.value {
            Value::Short(v) => metadata.iso = v.first().map(|&x| x as u32),
            Value::Long(v) => metadata.iso = v.first().map(|&x| x as u32),
            _ => {}
        }
    }
    if let Some(field) = exif.get_field(Tag::FNumber, In::PRIMARY) {
        if let Value::Rational(v) = &field.value {
            metadata.f_number = v.first().map(|r| r.to_f32());
        }
    }
    if let Some(field) = exif.get_field(Tag::ExposureTime, In::PRIMARY) {
        metadata.exposure_time = Some(field.display_value().with_unit(&exif).to_string());
    }

    // Orientation
    if let Some(field) = exif.get_field(Tag::Orientation, In::PRIMARY) {
        if let Value::Short(v) = &field.value {
            metadata.orientation = v.first().copied().unwrap_or(1) as u32;
        }
    }

    // GPS
    metadata.lat = get_gps_coord(&exif, Tag::GPSLatitude, Tag::GPSLatitudeRef);
    metadata.lon = get_gps_coord(&exif, Tag::GPSLongitude, Tag::GPSLongitudeRef);

    Ok(metadata)
}

/// Reads the 'DateTimeOriginal' from EXIF metadata.
/// - Returns Err(CoreError::Io) if the file cannot be opened.
/// - Returns Ok(None) if EXIF is missing or corrupted.
pub fn read_date_taken(path: &Path) -> Result<Option<String>, CoreError> {
    let file = File::open(path)?;
    let mut buf_reader = BufReader::new(file);

    let exif_reader = Reader::new();
    let exif = match exif_reader.read_from_container(&mut buf_reader) {
        Ok(exif) => exif,
        Err(_) => return Ok(None), // EXIF 解析失败，降级返回 Ok(None)
    };

    if let Some(field) = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY) {
        return Ok(Some(field.display_value().with_unit(&exif).to_string()));
    }

    Ok(None)
}

fn get_gps_coord(exif: &exif::Exif, tag: Tag, ref_tag: Tag) -> Option<f64> {
    let value = exif.get_field(tag, In::PRIMARY)?.value.clone();
    let ref_val = exif.get_field(ref_tag, In::PRIMARY)?.value.display_as(ref_tag).to_string();

    if let Value::Rational(v) = value {
        if v.len() >= 3 {
            let deg = v[0].to_f64();
            let min = v[1].to_f64();
            let sec = v[2].to_f64();
            let res = deg + min / 60.0 + sec / 3600.0;
            if ref_val == "S" || ref_val == "W" {
                return Some(-res);
            }
            return Some(res);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_metadata_graceful_failure_invariant() {
        let temp_dir = std::env::temp_dir().join("fotos_metadata_test");
        if temp_dir.exists() { std::fs::remove_dir_all(&temp_dir).unwrap(); }
        std::fs::create_dir_all(&temp_dir).unwrap();

        let corrupt_path = temp_dir.join("corrupt.jpg");
        let mut file = File::create(&corrupt_path).unwrap();
        // Write garbage data that is NOT a valid image
        file.write_all(b"not an image at all").unwrap();

        // Contract: read_date_taken should return Ok(None) for non-image/corrupt files
        let result = read_date_taken(&corrupt_path).expect("Should not fail IO");
        assert_eq!(result, None);

        // Contract: read_metadata should return basic object with 0 dimensions rather than Err
        let meta = read_metadata(&corrupt_path).expect("Should not fail IO");
        assert_eq!(meta.width, 0);
        assert_eq!(meta.height, 0);
        assert_eq!(meta.date_taken, None);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }
}
