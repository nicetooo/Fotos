use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct PhotoCoreConfig {
    pub thumbnail_dir: PathBuf,
    pub thumbnail_size: u32,
}
