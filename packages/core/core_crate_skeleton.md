# core Rust Core Library

> 跨桌面（macOS / Windows / Linux）+ 移动端（iOS / Android）通用的照片管理 **Rust Core 引擎库**。
>
> 本 crate **不包含任何平台、UI、权限、异步或并发逻辑**，仅提供纯引擎能力，适合作为 Tauri / Wails / Mobile FFI 的底层核心。

---

## 一、设计目标

- 平台无关（Platform-agnostic）
- 可被桌面 / 移动端复用
- FFI / IPC 友好
- 易于 AI 受控生成代码（vibe coding + 强边界）

---

## 二、目录结构

```text
core/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── error.rs
    ├── config.rs
    ├── types.rs
    ├── fs/
    │   ├── mod.rs
    │   └── scan.rs
    ├── image/
    │   ├── mod.rs
    │   ├── decode.rs
    │   └── thumbnail.rs
    ├── metadata/
    │   ├── mod.rs
    │   └── exif.rs
    └── index/
        ├── mod.rs
        └── photo_index.rs
```

---

## 三、Cargo.toml

```toml
[package]
name = "photo_core"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["rlib"]

[dependencies]
thiserror = "1.0"
image = { version = "0.24", default-features = false, features = ["jpeg", "png"] }
walkdir = "2.4"
rusqlite = { version = "0.30", features = ["bundled"] }
```

---

## 四、lib.rs

```rust
pub mod config;
pub mod error;
pub mod types;

pub mod fs;
pub mod image;
pub mod metadata;
pub mod index;

pub use config::PhotoCoreConfig;
pub use error::CoreError;
pub use types::{PhotoId, PhotoInfo};
```

---

## 五、error.rs（统一错误模型）

```rust
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
```

---

## 六、config.rs

```rust
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct PhotoCoreConfig {
    pub thumbnail_dir: PathBuf,
    pub thumbnail_size: u32,
}
```

---

## 七、types.rs

```rust
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
```

---

## 八、fs/scan.rs（文件扫描）

```rust
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::error::CoreError;

pub fn scan_photos(root: &Path) -> Result<Vec<PathBuf>, CoreError> {
    let mut result = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        let path = entry.path();

        if path.is_file() && is_supported_image(path) {
            result.push(path.to_path_buf());
        }
    }

    Ok(result)
}

fn is_supported_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "png")
    )
}
```

---

## 九、image/thumbnail.rs（缩略图生成）

```rust
use std::path::{Path, PathBuf};

use crate::{config::PhotoCoreConfig, error::CoreError};

pub fn generate_thumbnail(
    input: &Path,
    output_name: &str,
    config: &PhotoCoreConfig,
) -> Result<PathBuf, CoreError> {
    let img = image::open(input).map_err(|_| CoreError::ImageDecode)?;

    let thumb = img.thumbnail(
        config.thumbnail_size,
        config.thumbnail_size,
    );

    let output_path = config.thumbnail_dir.join(output_name);
    thumb
        .save(&output_path)
        .map_err(|_| CoreError::ImageDecode)?;

    Ok(output_path)
}
```

---

## 十、index/photo_index.rs（SQLite 索引）

```rust
use rusqlite::{Connection, params};
use std::path::Path;

use crate::{error::CoreError, types::{PhotoId, PhotoInfo}};

pub struct PhotoIndex {
    conn: Connection,
}

impl PhotoIndex {
    pub fn open(db_path: &Path) -> Result<Self, CoreError> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS photos (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL,
                width INTEGER,
                height INTEGER
            );",
        )?;
        Ok(Self { conn })
    }

    pub fn insert(&self, path: &str, width: u32, height: u32) -> Result<PhotoId, CoreError> {
        self.conn.execute(
            "INSERT INTO photos (path, width, height) VALUES (?1, ?2, ?3)",
            params![path, width, height],
        )?;

        Ok(PhotoId(self.conn.last_insert_rowid()))
    }

    pub fn list(&self) -> Result<Vec<PhotoInfo>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, width, height FROM photos",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(PhotoInfo {
                id: PhotoId(row.get(0)?),
                path: row.get(1)?,
                width: row.get::<_, i64>(2)? as u32,
                height: row.get::<_, i64>(3)? as u32,
            })
        })?;

        Ok(rows.filter_map(Result::ok).collect())
    }
}
```

---

## 十一、边界声明（用于 AI）

- 不允许平台 API
- 不允许 async / 多线程
- 不允许 UI / 权限 / 回调
- 不允许返回图片内存
- 所有错误必须 `Result`

> **如果某个设计在 iOS 上不成立，它就不属于 photo_core。**

---

## 十二、推荐下一步

- 在此文档基础上生成 `AI 实现 Prompt Checklist`
- 为该 crate 增加 UniFFI / C-ABI 包装层
- 增加 pipeline 示例（scan → thumbnail → index）

