use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Record)]
pub struct PhotoId {
    pub id: i64,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct PhotoInfo {
    pub id: PhotoId,
    pub path: String, // String is more portable for FFI
    pub hash: String,
    pub metadata: PhotoMetadata,
    pub thumb_path: Option<String>,
    pub file_size: u64,
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
}
