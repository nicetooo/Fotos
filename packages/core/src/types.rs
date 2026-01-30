use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Record)]
pub struct PhotoId {
    pub id: i64,
}

/// Media type classification for photos, videos, and Live Photos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Photo,
    Video,
    LivePhoto,
}

impl Default for MediaType {
    fn default() -> Self {
        MediaType::Photo
    }
}

impl MediaType {
    /// Convert to database string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            MediaType::Photo => "photo",
            MediaType::Video => "video",
            MediaType::LivePhoto => "live_photo",
        }
    }

    /// Parse from database string representation.
    pub fn from_str(s: &str) -> Self {
        match s {
            "video" => MediaType::Video,
            "live_photo" => MediaType::LivePhoto,
            _ => MediaType::Photo,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
pub struct ImportResult {
    pub success: u32,
    pub failure: u32,
    pub duplicates: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
pub struct PhotoMetadata {
    pub make: Option<String>,
    pub model: Option<String>,
    pub date_taken: Option<String>,
    pub width: u32,
    pub height: u32,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub iso: Option<u32>,
    pub f_number: Option<f32>,
    pub exposure_time: Option<String>,
    pub orientation: u32,
    // --- Video-specific fields ---
    /// Video duration in milliseconds (None for photos).
    pub duration_ms: Option<u64>,
    /// Video codec (e.g. "h264", "hevc"). None for photos.
    pub video_codec: Option<String>,
    /// Audio codec (e.g. "aac", "opus"). None for photos.
    pub audio_codec: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct PhotoInfo {
    pub id: PhotoId,
    pub path: String, // String is more portable for FFI
    pub hash: String,
    pub metadata: PhotoMetadata,
    pub thumb_path: Option<String>,
    pub file_size: u64,
    /// Media type: Photo, Video, or LivePhoto.
    pub media_type: MediaType,
    /// For LivePhoto: path to the paired video file. None for other types.
    pub video_path: Option<String>,
}

/// Lightweight photo data for map markers and timeline.
/// Contains only the fields needed for initial UI rendering.
/// Zero file I/O to construct — all data comes from the database.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct PhotoPreview {
    pub id: i64,
    pub path: String,
    pub thumb_path: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub date_taken: Option<String>,
    pub file_size: u64,
    /// Media type as string: "photo", "video", or "live_photo".
    pub media_type: String,
    /// Video duration in milliseconds (None for photos). Used for UI badges.
    pub duration_ms: Option<u64>,
}
