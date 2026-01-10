use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct PhotoCoreConfig {
    pub thumbnail_dir: String,
    pub thumbnail_size: u32,
}
