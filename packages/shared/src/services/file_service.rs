use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::error::PlatformResult;

/// File filter for picker dialogs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

/// Result of a file/folder picker operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PickerResult {
    /// Single file selected
    File(String),
    /// Multiple files selected
    Files(Vec<String>),
    /// Folder selected
    Folder(String),
    /// User cancelled
    Cancelled,
}

/// Options for file picker
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilePickerOptions {
    pub title: Option<String>,
    pub filters: Vec<FileFilter>,
    pub multiple: bool,
    pub directory: bool,
    pub default_path: Option<String>,
}

/// Platform-agnostic file service trait
#[async_trait]
pub trait FileService: Send + Sync {
    /// Open a file/folder picker dialog
    async fn pick_files(&self, options: FilePickerOptions) -> PlatformResult<PickerResult>;

    /// Read file as bytes
    async fn read_file(&self, path: &str) -> PlatformResult<Vec<u8>>;

    /// Write bytes to file
    async fn write_file(&self, path: &str, data: &[u8]) -> PlatformResult<()>;

    /// Delete a file
    async fn delete_file(&self, path: &str) -> PlatformResult<()>;

    /// Check if file exists
    async fn file_exists(&self, path: &str) -> PlatformResult<bool>;

    /// Create directory (including parents)
    async fn create_dir(&self, path: &str) -> PlatformResult<()>;

    /// Delete directory recursively
    async fn delete_dir(&self, path: &str) -> PlatformResult<()>;

    /// List files in directory
    async fn list_dir(&self, path: &str) -> PlatformResult<Vec<String>>;

    /// Reveal file in native file manager (desktop only)
    async fn reveal_in_file_manager(&self, path: &str) -> PlatformResult<()>;

    /// Open file with default application
    async fn open_file(&self, path: &str) -> PlatformResult<()>;

    /// Get file size in bytes
    async fn file_size(&self, path: &str) -> PlatformResult<u64>;
}
