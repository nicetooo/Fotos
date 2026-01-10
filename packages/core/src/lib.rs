pub mod config;
pub mod error;
pub mod types;

pub mod fs;
pub mod image;
pub mod metadata;
pub mod index;

pub use config::PhotoCoreConfig;
pub use error::CoreError;
pub use types::{PhotoId, PhotoInfo};

pub use fs::scan_photos;
pub use image::generate_thumbnail;
pub use index::PhotoIndex;
