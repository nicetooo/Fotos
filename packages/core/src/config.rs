use serde::{Deserialize, Serialize};

/// Single source of truth for thumbnail dimensions across all platforms.
///
/// This value is baked into the cache key (via `ThumbnailSpec`), so changing it
/// invalidates every existing thumbnail on disk.  Bump with care.
pub const DEFAULT_THUMBNAIL_SIZE: u32 = 256;

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct PhotoCoreConfig {
    pub thumbnail_dir: String,
    pub thumbnail_size: u32,
}
