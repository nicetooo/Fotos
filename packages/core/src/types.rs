use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhotoId(pub i64);

#[derive(Debug, Clone, Default)]
pub struct ImportResult {
    pub success: u32,
    pub failure: u32,
}

#[derive(Debug, Clone, Default)]
pub struct PhotoMetadata {
    pub make: Option<String>,
    pub model: Option<String>,
    pub date_taken: Option<String>,
    pub width: u32,
    pub height: u32,
    // Advanced Info
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub iso: Option<u32>,
    pub f_number: Option<f32>,
    pub exposure_time: Option<String>,
    pub orientation: u32,
}

#[derive(Debug, Clone)]
pub struct PhotoInfo {
    pub id: PhotoId,
    pub path: PathBuf,
    pub hash: String,
    pub metadata: PhotoMetadata,
}
