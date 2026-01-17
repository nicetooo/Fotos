use async_trait::async_trait;
use footos_shared::{
    FileService, FilePickerOptions, PickerResult,
    PlatformError, PlatformResult,
};
use std::path::Path;
use tokio::fs;

/// iOS file service implementation
///
/// Note: iOS uses sandboxed file access. Files outside the app sandbox
/// require special handling through iOS APIs (PHPickerViewController,
/// UIDocumentPickerViewController, etc.) which must be called from Swift.
pub struct IosFileService;

impl IosFileService {
    pub fn new() -> Self {
        Self
    }
}

impl Default for IosFileService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FileService for IosFileService {
    async fn pick_files(&self, _options: FilePickerOptions) -> PlatformResult<PickerResult> {
        // iOS file picking requires native Swift/ObjC APIs
        // This will be called from Swift via FFI
        Err(PlatformError::NotSupported(
            "File picker must be called through native iOS APIs".to_string()
        ))
    }

    async fn read_file(&self, path: &str) -> PlatformResult<Vec<u8>> {
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
        // Ensure parent directory exists
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
        // iOS doesn't have a traditional file manager accessible to apps
        Err(PlatformError::NotSupported(
            "Reveal in file manager is not supported on iOS".to_string()
        ))
    }

    async fn open_file(&self, _path: &str) -> PlatformResult<()> {
        // Opening files requires UIDocumentInteractionController from Swift
        Err(PlatformError::NotSupported(
            "Open file must be called through native iOS APIs".to_string()
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
