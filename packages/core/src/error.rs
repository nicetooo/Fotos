use thiserror::Error;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("IO error: {0}")]
    Io(String),
    #[error("Image decode error")]
    ImageDecode,
    #[error("Database error: {0}")]
    Database(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
}

impl From<std::io::Error> for CoreError {
    fn from(err: std::io::Error) -> Self {
        CoreError::Io(err.to_string())
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(err: rusqlite::Error) -> Self {
        CoreError::Database(err.to_string())
    }
}
