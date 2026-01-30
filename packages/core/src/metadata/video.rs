//! Video metadata extraction module.
//!
//! Uses the pure Rust `mp4` crate to parse MP4/MOV/M4V container formats.
//! For non-MP4 formats (AVI, MKV, WebM, FLV, etc.), returns a default
//! PhotoMetadata with all video fields set to None — the platform layer
//! (FFmpeg CLI or native APIs) is responsible for filling in those values.

use crate::error::CoreError;
use crate::types::PhotoMetadata;
use std::path::Path;

/// Read video metadata from an MP4/MOV/M4V file.
///
/// For non-MP4 container formats, returns a default PhotoMetadata
/// (all video fields `None`). Callers should fall back to platform-specific
/// tools (e.g., ffprobe) for those formats.
pub fn read_video_metadata(path: &Path) -> Result<PhotoMetadata, CoreError> {
    let mut metadata = PhotoMetadata::default();

    let f = std::fs::File::open(path).map_err(|e| CoreError::Io(e.to_string()))?;
    let size = f
        .metadata()
        .map_err(|e| CoreError::Io(e.to_string()))?
        .len();
    let reader = std::io::BufReader::new(f);

    let mp4 = match mp4::Mp4Reader::read_header(reader, size) {
        Ok(mp4) => mp4,
        Err(_) => {
            // Non-MP4 format: return empty metadata for platform layer to fill
            return Ok(metadata);
        }
    };

    // Duration
    metadata.duration_ms = Some(mp4.duration().as_millis() as u64);

    // Extract video track info (dimensions + codec)
    for track in mp4.tracks().values() {
        if let Ok(mp4::TrackType::Video) = track.track_type() {
            if metadata.width == 0 {
                metadata.width = track.width() as u32;
                metadata.height = track.height() as u32;
                metadata.video_codec = track.media_type().ok().map(|mt| format!("{:?}", mt));
            }
            break;
        }
    }

    // Extract audio track info (codec)
    for track in mp4.tracks().values() {
        if let Ok(mp4::TrackType::Audio) = track.track_type() {
            if metadata.audio_codec.is_none() {
                metadata.audio_codec = track.media_type().ok().map(|mt| format!("{:?}", mt));
            }
            break;
        }
    }

    // Creation time → date_taken
    // MP4 creation_time is seconds since 1904-01-01T00:00:00Z
    let creation_time = mp4.moov.mvhd.creation_time;
    if creation_time > 0 {
        metadata.date_taken = mp4_creation_time_to_exif_string(creation_time);
    }

    Ok(metadata)
}

/// Convert MP4 creation_time (seconds since 1904-01-01) to EXIF format string.
/// Returns format: `"2024-01-15 10:30:00"`
fn mp4_creation_time_to_exif_string(creation_time: u64) -> Option<String> {
    // MP4 epoch: 1904-01-01 00:00:00 UTC
    // Unix epoch: 1970-01-01 00:00:00 UTC
    // Difference: 2082844800 seconds
    const MP4_UNIX_EPOCH_DIFF: u64 = 2_082_844_800;

    if creation_time <= MP4_UNIX_EPOCH_DIFF {
        return None;
    }

    let unix_timestamp = creation_time - MP4_UNIX_EPOCH_DIFF;
    let secs = unix_timestamp;
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = days_to_ymd(days);

    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds
    ))
}

/// Convert days since 1970-01-01 to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let mut y = 1970;
    let mut remaining = days;

    loop {
        let days_in_year = if is_leap_year(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let months_days: [u64; 12] = if is_leap_year(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0;
    for md in &months_days {
        if remaining < *md {
            break;
        }
        remaining -= *md;
        m += 1;
    }

    (y, m + 1, remaining + 1)
}

fn is_leap_year(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mp4_creation_time_conversion() {
        // 2024-01-15 09:30:00 UTC
        // Unix: 1705311000 (19737 days * 86400 + 34200 seconds)
        // MP4: 1705311000 + 2082844800 = 3788155800
        let result = mp4_creation_time_to_exif_string(3_788_155_800);
        assert_eq!(result, Some("2024-01-15 09:30:00".to_string()));
    }

    #[test]
    fn test_invalid_creation_time() {
        assert_eq!(mp4_creation_time_to_exif_string(0), None);
        assert_eq!(mp4_creation_time_to_exif_string(100), None);
    }

    #[test]
    fn test_read_video_metadata_non_mp4_graceful() {
        // Non-MP4 files should return default metadata (not Err)
        let temp_dir = std::env::temp_dir().join("footos_video_meta_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let fake_avi = temp_dir.join("test.avi");
        std::fs::write(&fake_avi, b"RIFF....AVI ").unwrap();

        let result = read_video_metadata(&fake_avi);
        assert!(result.is_ok());
        let meta = result.unwrap();
        assert_eq!(meta.duration_ms, None);
        assert_eq!(meta.width, 0);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_days_to_ymd_basic() {
        // 1970-01-01 = day 0
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
        // 1970-01-31 = day 30
        assert_eq!(days_to_ymd(30), (1970, 1, 31));
        // 1970-02-01 = day 31
        assert_eq!(days_to_ymd(31), (1970, 2, 1));
        // 2000-01-01 (leap year)
        assert_eq!(days_to_ymd(10957), (2000, 1, 1));
    }
}
