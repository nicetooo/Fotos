use std::path::PathBuf;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug)]
pub enum CoreError {
    FileNotFound(PathBuf),
    InvalidFormat(String),
    InternalError(String),
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::FileNotFound(path) => write!(f, "File not found: {:?}", path),
            CoreError::InvalidFormat(msg) => write!(f, "Invalid format: {}", msg),
            CoreError::InternalError(msg) => write!(f, "Internal error: {}", msg),
        }
    }
}

impl std::error::Error for CoreError {}

/// Returns the library version
pub fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// A placeholder for photo scanning logic
pub fn scan_path(path: PathBuf) -> CoreResult<Vec<PathBuf>> {
    if !path.exists() {
        return Err(CoreError::FileNotFound(path));
    }
    
    // Placeholder implementation
    Ok(vec![])
}
