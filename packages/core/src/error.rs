use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Image decode error")]
    ImageDecode,

    #[error("Metadata parse error")]
    Metadata,

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Invalid input: {0}")]
    InvalidInput(String),
}
