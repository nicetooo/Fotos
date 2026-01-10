use serde::{Serialize, Deserialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Record)]
pub struct PhotoId {
    pub id: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
pub struct ImportResult {
    pub success: u32,
    pub failure: u32,
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
    pub created_at: Option<i64>, // Unix timestamp
    pub modified_at: Option<i64>, // Unix timestamp
}
