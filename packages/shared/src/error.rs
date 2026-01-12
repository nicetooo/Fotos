use thiserror::Error;

#[derive(Error, Debug)]
pub enum PlatformError {
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Operation cancelled")]
    Cancelled,

    #[error("Not supported on this platform: {0}")]
    NotSupported(String),

    #[error("Platform error: {0}")]
    Platform(String),
}

pub type PlatformResult<T> = Result<T, PlatformError>;
