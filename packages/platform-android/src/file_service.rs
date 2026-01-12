use async_trait::async_trait;
use fotos_shared::{
    FileService, FilePickerOptions, PickerResult,
    PlatformError, PlatformResult,
};
use std::path::Path;
use tokio::fs;

/// Android file service implementation
///
/// Note: Android uses Scoped Storage (Android 10+) which restricts
/// direct filesystem access. Media files are accessed through:
/// - MediaStore API for photos/videos
/// - Storage Access Framework (SAF) for documents
/// - App-specific directories (getFilesDir(), getCacheDir())
pub struct AndroidFileService;

impl AndroidFileService {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AndroidFileService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FileService for AndroidFileService {
    async fn pick_files(&self, _options: FilePickerOptions) -> PlatformResult<PickerResult> {
        // Android file picking requires Intent-based APIs
        // ACTION_OPEN_DOCUMENT, ACTION_GET_CONTENT, etc.
        // Must be called from Kotlin/Java layer
        Err(PlatformError::NotSupported(
            "File picker must be called through Android Intent APIs".to_string()
        ))
    }

    async fn read_file(&self, path: &str) -> PlatformResult<Vec<u8>> {
        // For content:// URIs, we need to use ContentResolver
        if path.starts_with("content://") {
            return Err(PlatformError::NotSupported(
                "Content URIs must be read through Android ContentResolver".to_string()
            ));
        }

        fs::read(path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PlatformError::FileNotFound(path.to_string())
            } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                PlatformError::PermissionDenied(path.to_string())
            } else {
                PlatformError::Io(e)
            }
        })
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> PlatformResult<()> {
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(path, data).await.map_err(PlatformError::Io)
    }

    async fn delete_file(&self, path: &str) -> PlatformResult<()> {
        fs::remove_file(path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PlatformError::FileNotFound(path.to_string())
            } else {
                PlatformError::Io(e)
            }
        })
    }

    async fn file_exists(&self, path: &str) -> PlatformResult<bool> {
        Ok(Path::new(path).exists())
    }

    async fn create_dir(&self, path: &str) -> PlatformResult<()> {
        fs::create_dir_all(path).await.map_err(PlatformError::Io)
    }

    async fn delete_dir(&self, path: &str) -> PlatformResult<()> {
        fs::remove_dir_all(path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PlatformError::FileNotFound(path.to_string())
            } else {
                PlatformError::Io(e)
            }
        })
    }

    async fn list_dir(&self, path: &str) -> PlatformResult<Vec<String>> {
        let mut entries = Vec::new();
        let mut dir = fs::read_dir(path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PlatformError::FileNotFound(path.to_string())
            } else {
                PlatformError::Io(e)
            }
        })?;

        while let Some(entry) = dir.next_entry().await? {
            if let Some(name) = entry.file_name().to_str() {
                entries.push(name.to_string());
            }
        }
        Ok(entries)
    }

    async fn reveal_in_file_manager(&self, _path: &str) -> PlatformResult<()> {
        // Android doesn't have a standard "reveal in file manager" action
        // Could open file manager app with Intent, but behavior varies by device
        Err(PlatformError::NotSupported(
            "Reveal in file manager is not standardized on Android".to_string()
        ))
    }

    async fn open_file(&self, _path: &str) -> PlatformResult<()> {
        // Opening files requires Intent.ACTION_VIEW from Kotlin/Java
        Err(PlatformError::NotSupported(
            "Open file must be called through Android Intent APIs".to_string()
        ))
    }

    async fn file_size(&self, path: &str) -> PlatformResult<u64> {
        let metadata = fs::metadata(path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PlatformError::FileNotFound(path.to_string())
            } else {
                PlatformError::Io(e)
            }
        })?;
        Ok(metadata.len())
    }
}
