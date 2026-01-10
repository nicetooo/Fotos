use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhotoId(pub i64);

#[derive(Debug, Clone)]
pub struct PhotoInfo {
    pub id: PhotoId,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}
