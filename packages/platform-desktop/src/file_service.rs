use async_trait::async_trait;
use fotos_shared::{
    FileService, FilePickerOptions, PickerResult, FileFilter,
    PlatformError, PlatformResult,
};
use std::path::Path;
use tokio::fs;

pub struct DesktopFileService;

impl DesktopFileService {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DesktopFileService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FileService for DesktopFileService {
    async fn pick_files(&self, options: FilePickerOptions) -> PlatformResult<PickerResult> {
        use tauri_plugin_dialog::{DialogExt, FileDialogBuilder};

        // Note: This requires a Tauri app handle which we don't have here
        // The actual implementation will be in the Tauri commands layer
        // This is a placeholder that shows the interface
        Err(PlatformError::NotSupported(
            "File picker must be called through Tauri commands".to_string()
        ))
    }

    async fn read_file(&self, path: &str) -> PlatformResult<Vec<u8>> {
        fs::read(path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PlatformError::FileNotFound(path.to_string())
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

    async fn reveal_in_file_manager(&self, path: &str) -> PlatformResult<()> {
        // Note: This requires Tauri's opener plugin
        // The actual implementation will be in the Tauri commands layer
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .args(["-R", path])
                .spawn()
                .map_err(|e| PlatformError::Platform(e.to_string()))?;
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer")
                .args(["/select,", path])
                .spawn()
                .map_err(|e| PlatformError::Platform(e.to_string()))?;
        }
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("xdg-open")
                .arg(Path::new(path).parent().unwrap_or(Path::new(path)))
                .spawn()
                .map_err(|e| PlatformError::Platform(e.to_string()))?;
        }
        Ok(())
    }

    async fn open_file(&self, path: &str) -> PlatformResult<()> {
        opener::open(path).map_err(|e| PlatformError::Platform(e.to_string()))
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
