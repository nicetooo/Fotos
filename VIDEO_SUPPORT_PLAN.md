# 视频支持实施规划

> 状态：**规划完成，待实施**
> 最后更新：2026-01-30

---

## 决策汇总

| 决策项 | 选择 |
|--------|------|
| 目标平台 | **全平台**（Desktop + iOS + Android） |
| 处理方案 | **混合**：Core 纯 Rust 解析 MP4/MOV 元数据 + 各平台原生 API 提取缩略图帧 |
| 格式范围 | **最大范围**：`mp4, mov, m4v, avi, mkv, webm, flv, wmv, 3gp, ts` |
| 播放功能 | **系统播放器打开**（不内嵌播放器）+ Live Photo 预览播放 |
| 数据模型 | **保持 `Photo*` 命名** + 新增 `media_type` 枚举字段 |
| Live Photo | **配对关联** HEIC/JPEG + MOV，标识展示，预览时播放视频部分 |

---

## 架构概览

```
                    ┌──────────────────────────────────┐
                    │           Core (纯 Rust)          │
                    │  ┌─────────┐  ┌───────────────┐  │
                    │  │ scan.rs │  │ video.rs (新)  │  │
                    │  │ +视频扩展│  │ mp4 crate 解析 │  │
                    │  └─────────┘  └───────────────┘  │
                    │  ┌─────────┐  ┌───────────────┐  │
                    │  │ hash.rs │  │ live_photo.rs  │  │
                    │  │ +视频哈希│  │ (新) 配对检测  │  │
                    │  └─────────┘  └───────────────┘  │
                    │  ┌──────────────────────────────┐│
                    │  │ photo_index.rs               ││
                    │  │ +media_type/duration/video_path││
                    │  └──────────────────────────────┘│
                    └──────────────┬───────────────────┘
                                   │
              ┌────────────────────┼────────────────────┐
              │                    │                     │
    ┌─────────▼──────┐  ┌─────────▼──────┐  ┌──────────▼─────┐
    │ Desktop (Tauri) │  │  iOS (Swift)   │  │Android (Kotlin)│
    │ FFmpeg CLI 缩略图│  │ AVFoundation   │  │MediaMetadata-  │
    │ open/xdg-open   │  │ AVAssetImage-  │  │Retriever       │
    │ ffprobe 元数据   │  │ Generator      │  │Intent.ACTION_  │
    └─────────┬──────┘  └─────────┬──────┘  │VIEW            │
              │                    │         └──────────┬─────┘
              └────────────────────┼────────────────────┘
                                   │
                    ┌──────────────▼───────────────────┐
                    │        Frontend (Svelte 5)        │
                    │  ┌─────────┐  ┌───────────────┐  │
                    │  │Grid:    │  │Preview:        │  │
                    │  │时长badge │  │视频→系统播放器  │  │
                    │  │LIVE标识  │  │Live→悬停/长按播放│  │
                    │  └─────────┘  └───────────────┘  │
                    └──────────────────────────────────┘
```

---

## Phase 1：Core 数据基础层

> **目标**：让系统"认识"视频和 Live Photo，能存储、能查询
>
> **预估**：3-4 天

### 1.1 新增类型定义

**文件**：`packages/core/src/types.rs`

在 `PhotoId` 之后（第 7 行后）新增枚举：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum MediaType {
    Photo,
    Video,
    LivePhoto,
}

impl Default for MediaType {
    fn default() -> Self { MediaType::Photo }
}
```

修改 `PhotoMetadata`（第 15-28 行），在 `orientation` 字段后新增：

```rust
// --- 视频专属字段 ---
pub duration_ms: Option<u64>,      // 视频时长（毫秒）
pub video_codec: Option<String>,   // 视频编码（如 "h264", "hevc"）
pub audio_codec: Option<String>,   // 音频编码（如 "aac", "opus"）
```

修改 `PhotoInfo`（第 30-38 行），在 `file_size` 字段后新增：

```rust
pub media_type: MediaType,
pub video_path: Option<String>,    // Live Photo 对应的 MOV 路径
```

修改 `PhotoPreview`（第 43-52 行），在 `file_size` 字段后新增：

```rust
pub media_type: String,            // "photo" / "video" / "live_photo"（轻量序列化）
pub duration_ms: Option<u64>,      // 视频时长，UI 展示用
```

修改 `lib.rs`（第 12 行），导出列表新增 `MediaType`。

### 1.2 视频格式检测

**文件**：`packages/core/src/fs/scan.rs`

将现有 `is_supported_image()` 从私有改为 `pub`（其他模块需要区分图片/视频）。

在 `is_supported_image()` 之后（第 51 行后）新增两个函数：

```rust
/// 检查是否为支持的视频文件
pub fn is_supported_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some(
            "mp4" | "mov" | "m4v" | "avi" | "mkv" | "webm" |
            "flv" | "wmv" | "3gp" | "ts"
        )
    )
}

/// 检查是否为支持的媒体文件（图片或视频）
pub fn is_supported_media(path: &Path) -> bool {
    is_supported_image(path) || is_supported_video(path)
}
```

修改 `scan_photos()`（第 23 行）：

```diff
- if path.is_file() && is_supported_image(path) {
+ if path.is_file() && is_supported_media(path) {
```

更新函数文档注释，说明现在扫描所有媒体文件（图片 + 视频）。函数名保持 `scan_photos` 不变，避免下游全部改名。

**文件**：`packages/core/src/image/parse.rs`

在 `is_raw_file()` 之后（第 43 行后）新增：

```rust
/// 检查是否为视频文件
pub fn is_video_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some(
            "mp4" | "mov" | "m4v" | "avi" | "mkv" | "webm" |
            "flv" | "wmv" | "3gp" | "ts"
        )
    )
}
```

新增测试：

```rust
#[test]
fn test_scan_includes_video_files() {
    // 在 temp 目录创建 .mp4 和 .mov 文件，验证 scan_photos 返回它们
}

#[test]
fn test_is_supported_video_extensions() {
    assert!(is_supported_video(Path::new("video.mp4")));
    assert!(is_supported_video(Path::new("video.MOV")));
    assert!(is_supported_video(Path::new("video.mkv")));
    assert!(!is_supported_video(Path::new("photo.jpg")));
    assert!(!is_supported_video(Path::new("photo.cr2")));
}
```

### 1.3 视频元数据解析

**新增依赖**（`packages/core/Cargo.toml`）：

```toml
mp4 = "0.14"  # 纯 Rust MP4/MOV 容器解析，零系统依赖
```

> `mp4` crate 是纯 Rust 实现，支持解析 MP4/MOV/M4V 容器中的 `moov` box，
> 提取 duration、track 信息、creation_time 等。符合 Core 层零平台依赖约束。

**新增文件**：`packages/core/src/metadata/video.rs`

```rust
//! 视频元数据解析模块
//!
//! 使用纯 Rust mp4 crate 解析 MP4/MOV/M4V 容器格式。
//! 对于非 MP4 格式（AVI, MKV, WebM, FLV 等），返回默认 PhotoMetadata，
//! 由平台层（FFmpeg/原生 API）补充完整信息。

use crate::types::PhotoMetadata;
use crate::error::CoreError;
use std::path::Path;

/// 从视频文件读取元数据。
///
/// 支持 MP4/MOV/M4V 容器（纯 Rust 解析）。
/// 非 MP4 格式返回带 duration_ms=None 的默认 PhotoMetadata，
/// 调用者应通过平台 API 补充。
pub fn read_video_metadata(path: &Path) -> Result<PhotoMetadata, CoreError> {
    let mut metadata = PhotoMetadata::default();

    let f = std::fs::File::open(path)?;
    let size = f.metadata()?.len();
    let reader = std::io::BufReader::new(f);

    let mp4 = match mp4::Mp4Reader::read_header(reader, size) {
        Ok(mp4) => mp4,
        Err(_) => return Ok(metadata), // 非 MP4 格式，降级返回空 metadata
    };

    // Duration
    metadata.duration_ms = Some(mp4.duration().as_millis() as u64);

    // 遍历 tracks 提取视频和音频信息
    for track in mp4.tracks().values() {
        match track.track_type() {
            Ok(mp4::TrackType::Video) => {
                if metadata.width == 0 {
                    metadata.width = track.width() as u32;
                    metadata.height = track.height() as u32;
                    metadata.video_codec = track.media_type()
                        .ok()
                        .map(|mt| format!("{:?}", mt));
                }
            }
            Ok(mp4::TrackType::Audio) => {
                if metadata.audio_codec.is_none() {
                    metadata.audio_codec = track.media_type()
                        .ok()
                        .map(|mt| format!("{:?}", mt));
                }
            }
            _ => {}
        }
    }

    // Creation time → date_taken
    // MP4 creation_time 是自 1904-01-01T00:00:00Z 的秒数
    let creation_time = mp4.moov.mvhd.creation_time;
    if creation_time > 0 {
        metadata.date_taken = mp4_creation_time_to_exif_string(creation_time);
    }

    // GPS：Apple MOV 在 udta box 的 ©xyz 或 com.apple.quicktime.location.ISO6709 中
    // mp4 crate 不直接支持自定义 box 解析，GPS 提取留作后续迭代
    // 平台层可通过 ffprobe 或 AVAsset 补充 GPS

    Ok(metadata)
}

/// 将 MP4 creation_time（自 1904-01-01 的秒数）转为 EXIF 格式字符串。
/// 返回格式：`"2024-01-15 10:30:00"`
fn mp4_creation_time_to_exif_string(creation_time: u64) -> Option<String> {
    // MP4 epoch: 1904-01-01 00:00:00 UTC
    // Unix epoch: 1970-01-01 00:00:00 UTC
    // Difference: 2082844800 seconds
    const MP4_UNIX_EPOCH_DIFF: u64 = 2_082_844_800;

    if creation_time <= MP4_UNIX_EPOCH_DIFF {
        return None; // 无效时间
    }

    let unix_timestamp = creation_time - MP4_UNIX_EPOCH_DIFF;
    // 简易时间格式化（不引入 chrono 依赖）
    // 使用 UNIX_EPOCH + Duration 计算
    let secs = unix_timestamp;
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // 从天数计算年月日（简化版，不处理闰秒）
    let (year, month, day) = days_to_ymd(days);

    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds
    ))
}

/// 将自 1970-01-01 的天数转为 (year, month, day)
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // 简化算法，足够精确处理 1970-2100 年范围
    let mut y = 1970;
    let mut remaining = days;

    loop {
        let days_in_year = if is_leap_year(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let months_days: [u64; 12] = if is_leap_year(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0;
    for md in &months_days {
        if remaining < *md {
            break;
        }
        remaining -= *md;
        m += 1;
    }

    (y, m + 1, remaining + 1)
}

fn is_leap_year(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mp4_creation_time_conversion() {
        // 2024-01-15 10:30:00 UTC
        // Unix: 1705311000
        // MP4: 1705311000 + 2082844800 = 3788155800
        let result = mp4_creation_time_to_exif_string(3_788_155_800);
        assert_eq!(result, Some("2024-01-15 10:30:00".to_string()));
    }

    #[test]
    fn test_invalid_creation_time() {
        assert_eq!(mp4_creation_time_to_exif_string(0), None);
        assert_eq!(mp4_creation_time_to_exif_string(100), None);
    }

    #[test]
    fn test_read_video_metadata_non_mp4_graceful() {
        // 非 MP4 文件应返回默认 metadata 而非 Err
        let temp_dir = std::env::temp_dir().join("footos_video_meta_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let fake_avi = temp_dir.join("test.avi");
        std::fs::write(&fake_avi, b"RIFF....AVI ").unwrap();

        let result = read_video_metadata(&fake_avi);
        assert!(result.is_ok());
        let meta = result.unwrap();
        assert_eq!(meta.duration_ms, None); // 无法解析
        assert_eq!(meta.width, 0);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }
}
```

**修改**：`packages/core/src/metadata/mod.rs`

```diff
  pub mod exif;
+ pub mod video;

  pub use exif::{read_metadata, read_date_taken};
+ pub use video::read_video_metadata;
```

**修改**：`packages/core/src/lib.rs`（第 17 行区域）

```diff
  pub use metadata::{read_date_taken, read_metadata};
+ pub use metadata::read_video_metadata;
```

### 1.4 视频哈希策略

**文件**：`packages/core/src/image/hash.rs`

在 `compute_file_hash()` 之后（第 47 行后）新增：

```rust
/// 计算视频文件的特征哈希。
///
/// 不做感知哈希（视频文件太大，解码太慢），采用文件首尾 64KB 内容 + 文件大小哈希。
/// 足以区分绝大多数视频文件，速度 <1ms。
///
/// 返回格式：`"video:xxxxxxxxxxxxxxxx"`
pub fn compute_video_hash(path: &Path) -> Result<String, CoreError> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::io::{Read, Seek, SeekFrom};

    let file = std::fs::File::open(path)
        .map_err(|_| CoreError::Io("open failed".into()))?;
    let file_size = file.metadata()
        .map_err(|_| CoreError::Io("metadata failed".into()))?.len();

    let mut hasher = DefaultHasher::new();
    file_size.hash(&mut hasher);

    let mut reader = std::io::BufReader::new(file);
    let chunk_size = 64 * 1024usize; // 64KB

    // 读取首部 64KB
    let read_size = chunk_size.min(file_size as usize);
    let mut head_buf = vec![0u8; read_size];
    reader.read_exact(&mut head_buf)
        .map_err(|_| CoreError::Io("read head failed".into()))?;
    head_buf.hash(&mut hasher);

    // 读取尾部 64KB（如果文件足够大，首尾不重叠）
    if file_size > (chunk_size as u64) * 2 {
        reader.seek(SeekFrom::End(-(chunk_size as i64)))
            .map_err(|_| CoreError::Io("seek tail failed".into()))?;
        let mut tail_buf = vec![0u8; chunk_size];
        reader.read_exact(&mut tail_buf)
            .map_err(|_| CoreError::Io("read tail failed".into()))?;
        tail_buf.hash(&mut hasher);
    }

    Ok(format!("video:{:016x}", hasher.finish()))
}
```

**修改** `packages/core/src/lib.rs` 导出：

```diff
  pub use image::{compute_hash, extract_raw_preview, ThumbnailError, ThumbnailSpec, Thumbnailer};
+ pub use image::compute_video_hash;
```

**修改** `packages/core/src/image/mod.rs`（如果存在 mod.rs，确保导出 `compute_video_hash`）。

### 1.5 数据库 Schema 迁移

**文件**：`packages/core/src/index/photo_index.rs`

在现有 migration 之后（第 45-50 行区域，`ALTER TABLE photos ADD COLUMN file_size` 之后）新增：

```rust
// Schema migration: video and media type support
let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN media_type TEXT DEFAULT 'photo';");
let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN duration_ms INTEGER;");
let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN video_codec TEXT;");
let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN audio_codec TEXT;");
let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN video_path TEXT;");
let _ = conn.execute_batch(
    "CREATE INDEX IF NOT EXISTS idx_photos_media_type ON photos (media_type);"
);
```

> `ALTER TABLE ADD COLUMN` 在列已存在时会报错，被 `let _` 静默忽略（现有模式）。
> 新列全有 DEFAULT 值，已有数据自动填充 `media_type = 'photo'`，其余为 NULL。

**修改 `insert()` 方法签名**（第 69 行）：

```rust
pub fn insert(
    &self,
    path: String,
    hash: String,
    metadata: PhotoMetadata,
    media_type: &str,              // 新增："photo" / "video" / "live_photo"
    video_path: Option<String>,    // 新增：Live Photo 的 MOV 路径
) -> Result<PhotoId, CoreError>
```

修改 INSERT 语句：

```sql
INSERT INTO photos (
    path, hash, make, model, date_taken, width, height,
    lat, lon, iso, f_number, exposure_time, orientation,
    media_type, duration_ms, video_codec, audio_codec, video_path
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
        ?14, ?15, ?16, ?17, ?18)
```

参数列表新增：

```rust
params![
    path, hash,
    metadata.make, metadata.model, metadata.date_taken,
    metadata.width, metadata.height,
    metadata.lat, metadata.lon,
    metadata.iso, metadata.f_number, metadata.exposure_time,
    metadata.orientation,
    media_type,                    // 新增
    metadata.duration_ms,          // 新增
    metadata.video_codec,          // 新增
    metadata.audio_codec,          // 新增
    video_path,                    // 新增
],
```

**修改 `list()` 方法**（第 391-455 行）：

SELECT 新增列：

```sql
SELECT
    id, path, hash, make, model, date_taken, width, height,
    lat, lon, iso, f_number, exposure_time, orientation,
    thumb_path, file_size,
    media_type, duration_ms, video_codec, audio_codec, video_path
FROM photos
```

映射新增字段（在 `file_size` 读取之后）：

```rust
media_type: {
    let mt: String = row.get::<_, Option<String>>(16)?
        .unwrap_or_else(|| "photo".to_string());
    match mt.as_str() {
        "video" => MediaType::Video,
        "live_photo" => MediaType::LivePhoto,
        _ => MediaType::Photo,
    }
},
video_path: row.get(17)?,
```

> 注意：`duration_ms`, `video_codec`, `audio_codec` 已在 `PhotoMetadata` 中，
> 在 metadata 构建时读取：

```rust
metadata: PhotoMetadata {
    // ... 现有字段 ...
    duration_ms: row.get(17)?,
    video_codec: row.get(18)?,
    audio_codec: row.get(19)?,
},
// media_type 和 video_path 在 metadata 之外
media_type: /* 见上 */ row 20,
video_path: row.get(21)?,
```

> ⚠️ 列索引需要重新对齐：thumb_path=14, file_size=15, media_type=16,
> duration_ms=17, video_codec=18, audio_codec=19, video_path=20

**修改 `list_previews()` 方法**（第 346-380 行）：

```sql
SELECT id, path, thumb_path, lat, lon, date_taken, file_size,
       media_type, duration_ms
FROM photos
```

映射新增：

```rust
media_type: row.get::<_, Option<String>>(7)?.unwrap_or_else(|| "photo".to_string()),
duration_ms: row.get(8)?,
```

**修改 `get_by_path()` 和 `get_by_id()`**：同步新增字段读取（SELECT 和映射）。

**新增方法 `update_media_info()`**：

```rust
/// 更新媒体类型和视频路径（用于 Live Photo 配对后更新）。
pub fn update_media_info(
    &self,
    path: String,
    media_type: &str,
    video_path: Option<String>,
) -> Result<(), CoreError> {
    let conn = self.conn.lock()
        .map_err(|e| CoreError::Database(e.to_string()))?;
    conn.execute(
        "UPDATE photos SET media_type = ?1, video_path = ?2 WHERE path = ?3",
        params![media_type, video_path, path],
    )?;
    Ok(())
}
```

**更新所有调用 `insert()` 的位置**：

由于 `insert()` 新增了两个参数，所有调用者需要更新：

- `packages/core/src/lib.rs` → `run_import_pipeline()` 中的 `index.insert(...)` 调用
- `packages/desktop/src-tauri/src/lib.rs` → `import_photos` Phase 3 的 `index.insert(...)` 调用
- `packages/desktop/src-tauri/src/lib.rs` → `process_ios_photo` 的 `index.insert(...)` 调用
- `packages/platform-ios/src/photo_service.rs` → `index.insert(...)` 调用
- `packages/platform-android/src/photo_service.rs` → `index.insert(...)` 调用
- 所有测试中的 `index.insert(...)` 调用

现有调用统一追加默认参数：`"photo", None`

**更新 `setup_test_index()`**（第 462-490 行）：建表语句新增列：

```sql
CREATE TABLE photos (
    -- ... 现有列 ...
    media_type TEXT DEFAULT 'photo',
    duration_ms INTEGER,
    video_codec TEXT,
    audio_codec TEXT,
    video_path TEXT
);
```

### 1.6 Live Photo 配对检测

**新增文件**：`packages/core/src/fs/live_photo.rs`

```rust
//! Live Photo 配对检测
//!
//! iPhone Live Photo 由一张静态图片（HEIC/JPEG）+ 一段短视频（MOV/MP4）组成，
//! 文件名相同（如 IMG_1234.HEIC + IMG_1234.MOV）。
//!
//! 本模块在文件扫描后检测配对关系，将配对的视频标记为 Live Photo 的一部分，
//! 避免视频单独入库。

use std::collections::HashMap;
use std::path::PathBuf;

/// Live Photo 配对结果
#[derive(Debug, Clone)]
pub struct LivePhotoPair {
    /// 静态图片路径（HEIC/JPEG）
    pub image_path: PathBuf,
    /// 配对视频路径（MOV/MP4）
    pub video_path: PathBuf,
}

/// 从文件列表中检测 Live Photo 配对。
///
/// 配对规则：
/// 1. 同目录、同 basename（如 IMG_1234.HEIC + IMG_1234.MOV）
/// 2. 一个是图片文件，一个是视频文件
/// 3. 视频文件 < 30MB（排除碰巧同名的长视频）
///
/// 返回所有检测到的配对。配对中的视频文件不应单独作为 Video 入库。
pub fn detect_live_photo_pairs(files: &[PathBuf]) -> Vec<LivePhotoPair> {
    // 按 (目录, basename) 分组
    let mut groups: HashMap<(String, String), (Vec<PathBuf>, Vec<PathBuf>)> = HashMap::new();

    for path in files {
        let dir = path.parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let basename = path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();

        if basename.is_empty() {
            continue;
        }

        let key = (dir, basename);
        let entry = groups.entry(key).or_insert_with(|| (vec![], vec![]));

        if super::scan::is_supported_image(path) {
            entry.0.push(path.clone());
        } else if super::scan::is_supported_video(path) {
            entry.1.push(path.clone());
        }
    }

    let mut pairs = Vec::new();
    for (_, (images, videos)) in &groups {
        // 严格 1:1 配对
        if images.len() == 1 && videos.len() == 1 {
            // 检查视频文件大小，排除大视频文件
            if let Ok(meta) = std::fs::metadata(&videos[0]) {
                if meta.len() < 30 * 1024 * 1024 { // < 30MB
                    pairs.push(LivePhotoPair {
                        image_path: images[0].clone(),
                        video_path: videos[0].clone(),
                    });
                }
            }
        }
    }

    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_detect_live_photo_pairs_basic() {
        let temp = std::env::temp_dir().join("footos_livephoto_test");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        // Live Photo 配对
        let img = temp.join("IMG_1234.heic");
        let mov = temp.join("IMG_1234.mov");
        fs::write(&img, b"fake heic").unwrap();
        fs::write(&mov, b"fake mov data").unwrap();

        // 普通照片（无配对）
        let solo = temp.join("DSC_5678.jpg");
        fs::write(&solo, b"fake jpg").unwrap();

        // 普通视频（无配对）
        let vid = temp.join("VID_9999.mp4");
        fs::write(&vid, b"fake mp4").unwrap();

        let files = vec![
            img.clone(), mov.clone(),
            solo.clone(), vid.clone(),
        ];

        let pairs = detect_live_photo_pairs(&files);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].image_path, img);
        assert_eq!(pairs[0].video_path, mov);

        fs::remove_dir_all(&temp).unwrap();
    }

    #[test]
    fn test_large_video_not_paired() {
        let temp = std::env::temp_dir().join("footos_livephoto_large");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        let img = temp.join("IMG_0001.heic");
        let mov = temp.join("IMG_0001.mov");
        fs::write(&img, b"fake heic").unwrap();
        // 创建 > 30MB 的"视频"（实际只写一小段，metadata 大小检查会通过）
        // 在真实测试中需要创建大文件或 mock metadata
        fs::write(&mov, vec![0u8; 100]).unwrap(); // 小文件，会被配对

        let pairs = detect_live_photo_pairs(&[img, mov]);
        assert_eq!(pairs.len(), 1); // 小文件被配对

        fs::remove_dir_all(&temp).unwrap();
    }
}
```

**修改**：`packages/core/src/fs/mod.rs`

```diff
  pub mod scan;
+ pub mod live_photo;

  pub use scan::scan_photos;
+ pub use live_photo::{detect_live_photo_pairs, LivePhotoPair};
```

**修改**：`packages/core/src/lib.rs` 导出新增：

```diff
  pub use fs::scan_photos;
+ pub use fs::{detect_live_photo_pairs, LivePhotoPair};
+ pub use fs::scan::{is_supported_image, is_supported_video};
```

---

## Phase 2：平台视频处理层

> **目标**：各平台能提取视频缩略图帧、打开系统播放器
>
> **预估**：3-4 天

### 2.1 Desktop — FFmpeg CLI 调用

**文件**：`packages/desktop/src-tauri/src/lib.rs`

新增 3 个 Tauri 命令：

#### `check_ffmpeg_available`

```rust
/// 检查系统是否安装了 FFmpeg
#[tauri::command]
fn check_ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
```

#### `extract_video_thumbnail`

```rust
/// 使用 FFmpeg 提取视频第 1 秒的帧作为缩略图。
/// 返回缩略图缓存路径。
/// 如果 FFmpeg 不可用，返回 Err（前端降级显示通用视频图标）。
#[tauri::command]
async fn extract_video_thumbnail(
    state: tauri::State<'_, AppState>,
    video_path: String,
) -> Result<String, String> {
    let thumb_dir = state.get_thumb_dir()?;
    let source = std::path::Path::new(&video_path);

    // 复用 Core 的缩略图缓存路径逻辑
    let spec = footos_core::ThumbnailSpec::default();
    let key = footos_core::image::thumbnail::thumbnail_key(source, &spec)
        .map_err(|e| e.to_string())?;
    let dest = footos_core::image::thumbnail::cache_path(
        std::path::Path::new(&thumb_dir), &key
    );

    // 已缓存则直接返回
    if dest.exists() {
        return Ok(dest.to_string_lossy().to_string());
    }

    // 确保父目录存在
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // 调用 ffmpeg 提取第 1 秒的帧，缩放到缩略图尺寸
    let dest_str = dest.to_str().ok_or("Invalid thumbnail path")?;
    let scale_filter = format!(
        "scale={}:{}:force_original_aspect_ratio=decrease",
        spec.width, spec.height
    );

    let output = std::process::Command::new("ffmpeg")
        .args([
            "-ss", "1",           // seek 到第 1 秒
            "-i", &video_path,
            "-vframes", "1",      // 只取 1 帧
            "-vf", &scale_filter, // 缩放
            "-q:v", "5",          // JPEG 质量
            "-y",                 // 覆盖已有文件
            dest_str,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("FFmpeg execution failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // 清理可能产生的空文件
        let _ = std::fs::remove_file(&dest);
        return Err(format!("FFmpeg failed: {}", stderr));
    }

    // 验证输出文件存在且非空
    if !dest.exists() || std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0) == 0 {
        let _ = std::fs::remove_file(&dest);
        return Err("FFmpeg produced empty output".to_string());
    }

    Ok(dest.to_string_lossy().to_string())
}
```

#### `open_in_system_player`

```rust
/// 用系统默认应用打开文件（视频用系统播放器）
#[tauri::command]
fn open_in_system_player(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open: {}", e))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path])
            .spawn()
            .map_err(|e| format!("Failed to open: {}", e))?;
    }
    #[cfg(target_os = "ios")]
    {
        // iOS: 由前端 Swift bridge 处理
        return Err("Use native iOS API to open videos".to_string());
    }
    #[cfg(target_os = "android")]
    {
        // Android: 由前端 Kotlin bridge 处理
        return Err("Use native Android API to open videos".to_string());
    }
    Ok(())
}
```

#### `read_video_metadata_ffprobe`（非 MP4 格式兜底）

```rust
/// 使用 ffprobe 读取视频元数据。
/// 用于 Core 层 mp4 crate 无法解析的格式（AVI, MKV, WebM, FLV 等）。
#[tauri::command]
async fn read_video_metadata_ffprobe(path: String) -> Result<serde_json::Value, String> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v", "quiet",
            "-print_format", "json",
            "-show_format",
            "-show_streams",
            &path,
        ])
        .output()
        .map_err(|e| format!("ffprobe execution failed: {}", e))?;

    if !output.status.success() {
        return Err("ffprobe failed".to_string());
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Parse ffprobe output failed: {}", e))
}
```

**注册新命令**（`invoke_handler` 数组，约第 962 行）：

```diff
  invoke_handler(tauri::generate_handler![
      greet,
      get_core_version,
      init_app,
      import_photos,
      cancel_import,
      list_photos,
      list_photos_preview,
      clear_app_data,
      regenerate_thumbnails,
      read_file_bytes,
      get_raw_preview,
      get_cached_tile,
      download_tile,
      delete_photos_from_app,
      delete_photos_completely,
      request_photo_library_access,
      process_ios_photo,
-     import_all_ios_photos
+     import_all_ios_photos,
+     extract_video_thumbnail,
+     open_in_system_player,
+     check_ffmpeg_available,
+     read_video_metadata_ffprobe,
  ])
```

### 2.2 导入管线修改

**文件**：`packages/desktop/src-tauri/src/lib.rs`

#### 修改 `ProcessedPhoto` 结构（第 210-217 行）

```diff
  struct ProcessedPhoto {
      path: std::path::PathBuf,
      path_str: String,
      hash: String,
      metadata: footos_core::PhotoMetadata,
      thumb_path_str: Option<String>,
      file_size: u64,
+     media_type: String,        // "photo" / "video" / "live_photo"
+     video_path: Option<String>, // Live Photo 的视频路径
  }
```

#### 修改 Phase 2 并行处理（第 401-469 行）

在 `par_iter().map()` 闭包内，根据文件类型分支：

```rust
.map(|path| {
    if IMPORT_CANCELLED.load(Ordering::Relaxed) {
        return ProcessResult::Cancelled;
    }

    let path_str = path.to_string_lossy().to_string();
    let is_video = footos_core::is_supported_video(path);

    // === Hash ===
    let hash = if is_video {
        footos_core::compute_video_hash(path)
    } else {
        footos_core::compute_hash(path)
    };
    let hash = match hash {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(path = %path_str, error = %e, "Import: hash failed");
            return ProcessResult::Failed(path_str);
        }
    };

    if IMPORT_CANCELLED.load(Ordering::Relaxed) {
        return ProcessResult::Cancelled;
    }

    // === Metadata ===
    let metadata = if is_video {
        footos_core::read_video_metadata(path).unwrap_or_default()
    } else {
        match footos_core::read_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(path = %path_str, error = %e, "Import: metadata failed");
                return ProcessResult::Failed(path_str);
            }
        }
    };

    // === Thumbnail ===
    let thumb_path_str = if is_video {
        // 视频缩略图：调用 ffmpeg CLI（同步版本）
        extract_video_thumbnail_sync(path, &thumb_dir_clone)
    } else {
        footos_core::generate_thumbnail(path, &config)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    };

    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let media_type = if is_video { "video" } else { "photo" }.to_string();

    // ... 进度通知 ...

    ProcessResult::New(ProcessedPhoto {
        path: path.clone(),
        path_str,
        hash,
        metadata,
        thumb_path_str,
        file_size,
        media_type,
        video_path: None, // Live Photo 在 Phase 3 处理
    })
})
```

新增辅助函数（在 `import_photos` 之外）：

```rust
/// FFmpeg 视频缩略图提取（同步版本，用于 rayon 并行处理）
fn extract_video_thumbnail_sync(
    video_path: &std::path::Path,
    thumb_dir: &str,
) -> Option<String> {
    let spec = footos_core::ThumbnailSpec::default();
    let key = footos_core::image::thumbnail::thumbnail_key(video_path, &spec).ok()?;
    let dest = footos_core::image::thumbnail::cache_path(
        std::path::Path::new(thumb_dir), &key
    );

    if dest.exists() {
        return Some(dest.to_string_lossy().to_string());
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }

    let scale_filter = format!(
        "scale={}:{}:force_original_aspect_ratio=decrease",
        spec.width, spec.height
    );

    let output = std::process::Command::new("ffmpeg")
        .args([
            "-ss", "1",
            "-i", &video_path.to_string_lossy(),
            "-vframes", "1",
            "-vf", &scale_filter,
            "-q:v", "5",
            "-y",
            dest.to_str()?,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if output.status.success() && dest.exists() {
        Some(dest.to_string_lossy().to_string())
    } else {
        let _ = std::fs::remove_file(&dest);
        None
    }
}
```

#### 修改 Phase 1 和 Phase 3 之间：Live Photo 配对

在 Phase 1（filter）之后、Phase 2（parallel）之前：

```rust
// ── Live Photo 配对检测 ──
let live_pairs = footos_core::detect_live_photo_pairs(&new_photos);
let live_video_set: std::collections::HashSet<std::path::PathBuf> = live_pairs.iter()
    .map(|p| p.video_path.clone())
    .collect();
let live_photo_map: std::collections::HashMap<std::path::PathBuf, std::path::PathBuf> =
    live_pairs.iter()
    .map(|p| (p.image_path.clone(), p.video_path.clone()))
    .collect();

tracing::info!(
    pairs = live_pairs.len(),
    "Import: Live Photo pairs detected"
);

// 从待处理列表中移除 Live Photo 的视频（它们不单独入库）
new_photos.retain(|p| !live_video_set.contains(p));
```

#### 修改 Phase 3 DB insert

```rust
ProcessResult::New(photo) => {
    // ... 现有去重逻辑 ...

    // 检查是否为 Live Photo 图片端
    let (final_media_type, final_video_path) = if let Some(vp) = live_photo_map.get(&photo.path) {
        ("live_photo".to_string(), Some(vp.to_string_lossy().to_string()))
    } else {
        (photo.media_type.clone(), photo.video_path.clone())
    };

    match index.insert(
        photo.path_str.clone(),
        photo.hash,
        photo.metadata,
        &final_media_type,       // 新参数
        final_video_path.clone(), // 新参数
    ) {
        Ok(_) => {
            let _ = index.update_thumb_info(
                photo.path_str,
                photo.thumb_path_str,
                photo.file_size,
            );
            result.success += 1;
            batch_writes += 1;
        }
        Err(e) => {
            tracing::error!(error = %e, "Import: DB insert failed");
            result.failure += 1;
        }
    }
}
```

### 2.3 iOS 平台处理

**文件**：`packages/platform-ios/src/photo_service.rs`

修改 `process_photo()` 签名：

```rust
pub fn process_photo(
    &self,
    photo_data: &[u8],
    identifier: &str,
    db_path: &str,
    thumbnail_dir: &str,
    media_type: &str,              // 新增："photo" / "video" / "live_photo"
    video_data: Option<&[u8]>,     // 新增：Live Photo 的 MOV 数据
) -> Result<(), String>
```

处理逻辑增加视频分支：

```rust
// 元数据：视频用 read_video_metadata，图片用 read_metadata
let metadata = if media_type == "video" {
    footos_core::read_video_metadata(path).unwrap_or_default()
} else {
    read_metadata(path).map_err(|e| e.to_string())?
};

// 哈希：视频用 compute_video_hash
let hash = if media_type == "video" {
    footos_core::compute_video_hash(path).map_err(|e| e.to_string())?
} else {
    compute_hash(path).map_err(|e| e.to_string())?
};

// Live Photo 视频部分保存
let video_path = if let Some(vdata) = video_data {
    let video_temp = format!("{}/live_{}.mov", thumbnail_dir, identifier);
    fs::write(&video_temp, vdata).map_err(|e| e.to_string())?;
    Some(format!("photos-video://{}", identifier))
} else {
    None
};

// 入库时传入 media_type 和 video_path
index.insert(ios_path, hash, metadata, media_type, video_path)
    .map_err(|e| e.to_string())?;
```

**Swift 端需新增**（不在 Rust scope 但需要配合）：
- `PHAsset.mediaType == .video` → 用 `AVAssetImageGenerator.copyCGImage()` 提取缩略图帧 → 传入 Rust
- `PHAsset.mediaSubtypes.contains(.photoLive)` → 用 `PHAssetResource` 获取 paired video → 传入 Rust

### 2.4 Android 平台处理

**文件**：`packages/platform-android/src/photo_service.rs`

同 iOS 的修改模式：`process_photo()` 新增 `media_type` 和 `video_data` 参数。

**Kotlin 端需新增**（不在 Rust scope 但需要配合）：
- `MediaMetadataRetriever.getFrameAtTime(1_000_000)` → Bitmap → JPEG bytes 作为缩略图
- `MediaMetadataRetriever.extractMetadata(METADATA_KEY_DURATION)` 等提取元数据
- `Intent(ACTION_VIEW).setDataAndType(uri, "video/*")` 打开系统播放器

---

## Phase 3：前端 UI 改造

> **目标**：视频和 Live Photo 在各处正确展示
>
> **预估**：3-4 天

### 3.1 类型定义

**文件**：`packages/desktop/src/shared/types.ts`

修改 `PhotoMetadata`（第 10-21 行），新增：

```typescript
export interface PhotoMetadata {
    // ... 现有字段 ...
    duration_ms?: number;
    video_codec?: string;
    audio_codec?: string;
}
```

修改 `PhotoInfo`（第 23-34 行），新增：

```typescript
export interface PhotoInfo {
    // ... 现有字段 ...
    media_type?: 'photo' | 'video' | 'live_photo';
    videoPath?: string;
}
```

修改 `PhotoPreview`（第 41-49 行），新增：

```typescript
export interface PhotoPreview {
    // ... 现有字段 ...
    media_type?: string;
    duration_ms?: number;
}
```

### 3.2 文件扩展名

**文件**：`packages/desktop/src/lib/platform/desktop.ts`

```diff
- const IMAGE_EXTENSIONS = [
-     "jpg", "jpeg", "png", "heic", "heif", "webp",
-     "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2",
-     "dng", "raf", "orf", "rw2", "pef", "raw"
- ];
+ const IMAGE_EXTENSIONS = [
+     "jpg", "jpeg", "png", "heic", "heif", "webp",
+     "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2",
+     "dng", "raf", "orf", "rw2", "pef", "raw"
+ ];
+
+ const VIDEO_EXTENSIONS = [
+     "mp4", "mov", "m4v", "avi", "mkv", "webm",
+     "flv", "wmv", "3gp", "ts"
+ ];
+
+ const MEDIA_EXTENSIONS = [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS];
```

修改 `requestImport()` 和 `requestScan()` 中的 filters：

```diff
  filters: [{
-     name: "Images",
-     extensions: IMAGE_EXTENSIONS
+     name: "Media Files",
+     extensions: MEDIA_EXTENSIONS
  }]
```

### 3.3 分组逻辑

**文件**：`packages/desktop/src/shared/utils/photo-grouping.ts`

新增：

```typescript
/** 视频文件扩展名 */
const VIDEO_EXTENSIONS = new Set([
    "mp4", "mov", "m4v", "avi", "mkv", "webm",
    "flv", "wmv", "3gp", "ts"
]);

/** 检查是否是视频文件 */
export function isVideoFile(path: string): boolean {
    const ext = path.split(".").pop()?.toLowerCase() || "";
    return VIDEO_EXTENSIONS.has(ext);
}
```

修改 `groupPhotos()`：视频和 Live Photo 不参与 RAW+JPEG 配对逻辑：

```diff
  export function groupPhotos(photos: PhotoInfo[]): PhotoInfo[] {
      const byBaseName = new Map<string, { jpegs: PhotoInfo[]; raws: PhotoInfo[] }>();
+     const result: PhotoInfo[] = [];

      for (const photo of photos) {
+         // 视频和 Live Photo 不参与 RAW+JPEG 分组，直接放入结果
+         if (photo.media_type === 'video' || photo.media_type === 'live_photo') {
+             result.push(photo);
+             continue;
+         }
+
          const baseName = getBaseName(photo.path);
          // ... 现有分组逻辑 ...
      }

-     const result: PhotoInfo[] = [];
      // ... 现有构建结果逻辑 ...
```

### 3.4 Desktop 网格视图

**文件**：`packages/desktop/src/desktop/components/VirtualPhotoGrid.svelte`

在 `<script>` 中新增辅助函数：

```typescript
/** 格式化视频时长（毫秒 → "m:ss"） */
function formatDuration(ms?: number): string {
    if (!ms) return "0:00";
    const totalSec = Math.floor(ms / 1000);
    const min = Math.floor(totalSec / 60);
    const sec = totalSec % 60;
    if (min >= 60) {
        const hr = Math.floor(min / 60);
        const remainMin = min % 60;
        return `${hr}:${remainMin.toString().padStart(2, '0')}:${sec.toString().padStart(2, '0')}`;
    }
    return `${min}:${sec.toString().padStart(2, '0')}`;
}
```

在模板中，每张照片的缩略图上新增视频/Live Photo 标识（与现有 RAW badge 并列）：

```svelte
<!-- 视频时长 badge -->
{#if photo.media_type === 'video'}
    <div class="absolute bottom-1 left-1 px-1.5 py-0.5 rounded text-[10px]
                font-semibold bg-black/70 text-white flex items-center gap-1">
        <i class="fa-solid fa-play text-[8px]"></i>
        {formatDuration(photo.metadata?.duration_ms || photo.duration_ms)}
    </div>
{/if}

<!-- Live Photo 标识 -->
{#if photo.media_type === 'live_photo'}
    <div class="absolute bottom-1 left-1 px-1.5 py-0.5 rounded text-[10px]
                font-semibold bg-black/70 text-white">
        LIVE
    </div>
{/if}
```

### 3.5 Desktop 预览组件

**文件**：`packages/desktop/src/desktop/components/ImagePreview.svelte`

修改 props：

```diff
- let { src, alt = "Preview", thumbPath = "", t } = $props<{
+ let { src, alt = "Preview", thumbPath = "", t, mediaType = "photo", videoPath = "" } = $props<{
      src: string;
      alt?: string;
      thumbPath?: string;
      t: Translations;
+     mediaType?: string;
+     videoPath?: string;
  }>();
```

新增状态：

```typescript
let liveVideoPlaying = $state(false);
```

新增函数：

```typescript
async function openInSystemPlayer() {
    try {
        await invoke("open_in_system_player", { path: src });
    } catch (e) {
        console.error("Failed to open in system player:", e);
    }
}
```

修改模板，根据 `mediaType` 分支渲染：

```svelte
{#if mediaType === 'video'}
    <!-- 视频预览：缩略图 + 居中播放按钮 -->
    <div class="relative w-full h-full flex items-center justify-center bg-black">
        {#if imageSrc}
            <img
                src={imageSrc}
                {alt}
                class="max-w-full max-h-full object-contain opacity-70"
                onload={handleLoad}
                onerror={handleError}
            />
        {/if}
        <button
            class="absolute inset-0 flex items-center justify-center cursor-pointer"
            onclick={openInSystemPlayer}
        >
            <div class="w-20 h-20 rounded-full bg-white/20 backdrop-blur-sm
                        flex items-center justify-center
                        hover:bg-white/30 transition-colors">
                <i class="fa-solid fa-play text-white text-3xl ml-1"></i>
            </div>
        </button>
    </div>

{:else if mediaType === 'live_photo'}
    <!-- Live Photo：默认显示图片，鼠标悬停时播放 MOV -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
        bind:this={container}
        class="w-full h-full overflow-auto bg-transparent relative"
        onwheel={handleWheel}
        ondblclick={handleDoubleClick}
        onmouseenter={() => { if (videoPath) liveVideoPlaying = true; }}
        onmouseleave={() => liveVideoPlaying = false}
    >
        {#if liveVideoPlaying && videoPath}
            <!-- 播放 Live Photo 视频部分 -->
            <div class="absolute inset-0 flex items-center justify-center">
                <video
                    src={convertFileSrc(videoPath)}
                    autoplay
                    loop
                    muted
                    playsinline
                    class="max-w-full max-h-full object-contain"
                />
            </div>
        {:else}
            <!-- 显示静态图片（现有逻辑） -->
            <div class="flex items-center justify-center" style="...">
                {#if imageSrc}
                    <img src={imageSrc} ... />
                {/if}
            </div>
        {/if}

        <!-- Live Photo 标识 -->
        <div class="absolute top-3 left-3 px-2 py-1 rounded-full
                    bg-white/20 backdrop-blur-sm text-xs text-white
                    flex items-center gap-1 pointer-events-none">
            <i class="fa-solid fa-circle-dot"></i>
            LIVE
        </div>
    </div>

{:else}
    <!-- 现有图片预览逻辑（完全不变） -->
    ...
{/if}
```

### 3.6 Mobile 预览组件

**文件**：`packages/desktop/src/mobile/components/ImagePreview.svelte`

类似 Desktop 修改，但 Live Photo 交互使用长按（触摸事件）：

```svelte
{:else if mediaType === 'live_photo'}
    <!-- Live Photo 移动端：长按播放视频部分 -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
        ontouchstart={() => { if (videoPath) liveVideoPlaying = true; }}
        ontouchend={() => liveVideoPlaying = false}
        ontouchcancel={() => liveVideoPlaying = false}
        class="relative w-full h-full"
    >
        {#if liveVideoPlaying && videoPath}
            <video
                src={convertFileSrc(videoPath)}
                autoplay loop muted playsinline
                class="w-full h-full object-contain"
            />
        {:else}
            <img src={imageSrc} ... />
        {/if}
    </div>
{/if}
```

### 3.7 平台服务接口扩展

**文件**：`packages/desktop/src/lib/platform/types.ts`

`PlatformService` 接口新增：

```typescript
/** 用系统播放器打开文件 */
openInSystemPlayer?(path: string): Promise<void>;
```

**文件**：`packages/desktop/src/lib/platform/desktop.ts`

实现：

```typescript
async openInSystemPlayer(path: string): Promise<void> {
    await invoke("open_in_system_player", { path });
}
```

**文件**：`packages/desktop/src/lib/platform/ios.ts`

实现（通过 Swift bridge）：

```typescript
async openInSystemPlayer(path: string): Promise<void> {
    // 通过 WebKit message handler 调用 Swift
    if ((window as any).webkit?.messageHandlers?.openVideo) {
        (window as any).webkit.messageHandlers.openVideo.postMessage({ path });
    }
}
```

**文件**：`packages/desktop/src/lib/platform/android.ts`

实现（通过 Kotlin bridge）：

```typescript
async openInSystemPlayer(path: string): Promise<void> {
    this.postMessage({ command: "openVideo", path });
}
```

### 3.8 地图组件

**文件**：`packages/desktop/src/desktop/components/Map.svelte` 和 `src/mobile/components/Map.svelte`

在 `buildGeoJSON()` 函数中，GeoJSON feature 的 properties 新增：

```typescript
properties: {
    // ... 现有字段 ...
    mediaType: photo.media_type || 'photo',
    durationMs: photo.duration_ms,
}
```

视频和 Live Photo 在地图上的缩略图显示方式与照片相同（圆形缩略图 + GeoJSON 图层），
无需特殊处理。后续可选：在缩略图上叠加小图标区分类型。

### 3.9 Desktop/Mobile App.svelte 传参

**文件**：`packages/desktop/src/desktop/App.svelte`

在调用 `<ImagePreview>` 的地方新增属性传递：

```svelte
<ImagePreview
    src={previewPhoto.path}
    thumbPath={previewPhoto.thumb_path}
    t={t}
    mediaType={previewPhoto.media_type || 'photo'}
    videoPath={previewPhoto.videoPath || previewPhoto.video_path || ''}
/>
```

**文件**：`packages/desktop/src/mobile/App.svelte`

同上。

---

## Phase 4：Live Photo 专项

> **目标**：完整的 Live Photo 体验
>
> **预估**：2-3 天

### 4.1 iOS 原生识别（Swift）

在 PHPickerViewController delegate 中：

```swift
// 判断 Live Photo
if let asset = result.itemProvider.registeredTypeIdentifiers
    .contains("com.apple.live-photo-bundle") {
    // 获取 image + paired video
    let resources = PHAssetResource.assetResources(for: asset)
    let imageResource = resources.first(where: { $0.type == .photo })
    let videoResource = resources.first(where: { $0.type == .pairedVideo })

    // 导出 image data
    exportResource(imageResource) { imageData in
        // 导出 video data
        exportResource(videoResource) { videoData in
            // 传入 Rust
            ios_process_photo(
                imageData, identifier, dbPath, thumbDir,
                "live_photo", videoData
            )
        }
    }
}
```

### 4.2 Desktop 文件系统配对

由 Phase 1.6 的 `detect_live_photo_pairs()` 和 Phase 2.2 的导入管线修改已覆盖。
Desktop 端的 Live Photo 配对是自动的：扫描目录时检测同名 HEIC+MOV。

### 4.3 Android 处理

Android 没有原生 Live Photo 概念。策略：
- 从 iOS 导入的 Live Photo（HEIC+MOV 文件对）：同 Desktop 文件系统配对
- Samsung Motion Photo：后续迭代（格式为单个 JPEG/HEIC 文件内嵌视频轨道）
- Google Motion Photo：后续迭代（类似 Samsung）

---

## 修改文件完整清单

### Rust Core（`packages/core/`）

| 文件 | 改动 | 说明 |
|------|------|------|
| `Cargo.toml` | **修改** | 新增 `mp4 = "0.14"` |
| `src/types.rs` | **修改** | 新增 `MediaType` 枚举；`PhotoMetadata` +3 字段；`PhotoInfo` +2 字段；`PhotoPreview` +2 字段 |
| `src/lib.rs` | **修改** | 新增导出 |
| `src/error.rs` | 不变 | — |
| `src/config.rs` | 不变 | — |
| `src/fs/mod.rs` | **修改** | 新增 `pub mod live_photo` |
| `src/fs/scan.rs` | **修改** | `is_supported_image` 改 pub；新增 `is_supported_video`, `is_supported_media`；`scan_photos` 使用 `is_supported_media` |
| `src/fs/live_photo.rs` | **新增** | Live Photo 配对检测 |
| `src/metadata/mod.rs` | **修改** | 新增 `pub mod video` |
| `src/metadata/exif.rs` | 不变 | — |
| `src/metadata/video.rs` | **新增** | MP4/MOV 视频元数据解析 |
| `src/image/mod.rs` | **修改** | 导出 `compute_video_hash` |
| `src/image/hash.rs` | **修改** | 新增 `compute_video_hash()` |
| `src/image/parse.rs` | **修改** | 新增 `is_video_file()` |
| `src/image/thumbnail.rs` | 不变 | — |
| `src/index/photo_index.rs` | **修改** | Schema migration +5 列；`insert()` +2 参数；`list()`/`list_previews()` 读新字段；新增 `update_media_info()` |

### Tauri Desktop（`packages/desktop/src-tauri/`）

| 文件 | 说明 |
|------|------|
| `src/lib.rs` | 新增 4 命令；修改 `ProcessedPhoto`；修改 `import_photos` Phase 1/2/3；新增 `extract_video_thumbnail_sync` |

### 平台层

| 文件 | 说明 |
|------|------|
| `packages/platform-ios/src/photo_service.rs` | `process_photo()` +2 参数 |
| `packages/platform-android/src/photo_service.rs` | `process_photo()` +2 参数 |

### 前端 Svelte/TypeScript

| 文件 | 说明 |
|------|------|
| `src/shared/types.ts` | `PhotoMetadata` +3 字段；`PhotoInfo` +2 字段；`PhotoPreview` +2 字段 |
| `src/shared/utils/photo-grouping.ts` | 新增 `isVideoFile()`；`groupPhotos()` 跳过视频 |
| `src/lib/platform/types.ts` | `PlatformService` 新增 `openInSystemPlayer?()` |
| `src/lib/platform/desktop.ts` | `MEDIA_EXTENSIONS`；实现 `openInSystemPlayer()` |
| `src/lib/platform/ios.ts` | 实现 `openInSystemPlayer()` |
| `src/lib/platform/android.ts` | 实现 `openInSystemPlayer()` |
| `src/desktop/components/VirtualPhotoGrid.svelte` | 视频时长 badge + Live Photo 标识 |
| `src/desktop/components/ImagePreview.svelte` | 新增 props；视频/Live Photo 分支渲染 |
| `src/mobile/components/ImagePreview.svelte` | 同 Desktop，Live Photo 用长按 |
| `src/desktop/components/Map.svelte` | GeoJSON properties 新增 `mediaType` |
| `src/mobile/components/Map.svelte` | 同上 |
| `src/desktop/App.svelte` | 传递 `mediaType`/`videoPath` 给 Preview |
| `src/mobile/App.svelte` | 同上 |

---

## 不变量约束（实施红线）

1. **Core 层零平台依赖** — 只用 `mp4` 纯 Rust crate，禁止 FFmpeg/系统库/`#[cfg(target_os)]` 业务分支
2. **Core 层 API 限制** — 输入只允许 `PathBuf`/`String`/`Vec<u8>`/纯数据结构，输出只允许 `Result<T, CoreError>`
3. **缩略图规格不变** — 视频缩略图复用 `ThumbnailSpec::default()` (256px) 和 FNV-1a 缓存 key
4. **两阶段加载不变** — Phase 1 返回 `PhotoPreview`（含 `media_type` + `duration_ms`）即可渲染地图
5. **地图渲染方式不变** — GeoJSON + GPU 图层，禁止 DOM Marker
6. **数据库向后兼容** — 新列全有 DEFAULT 值，`ALTER TABLE ADD COLUMN` 升级，已有数据自动填充 `media_type = 'photo'`
7. **Live Photo 的 MOV 不单独入库** — 只作为 `video_path` 关联到图片记录
8. **FFmpeg 降级处理** — Desktop 端 FFmpeg 不可用时，视频不生成缩略图（显示通用视频图标），不阻塞导入

---

## 实施顺序

```
Phase 1  ────────────────  Core 数据基础       ✅ DONE
  1.1 types.rs 新增枚举和字段               ✅
  1.2 scan.rs 视频格式检测                  ✅
  1.3 metadata/video.rs 视频元数据解析       ✅
  1.4 hash.rs 视频哈希                      ✅
  1.5 photo_index.rs Schema + CRUD          ✅
  1.6 fs/live_photo.rs 配对检测             ✅
  └─ cargo test 全部通过                    ✅ (40/40, 2 pre-existing thumbnail failures unrelated)

Phase 2  ────────────────  平台视频处理         ~3-4 天
  2.1 Tauri 新增命令（ffmpeg、系统播放器）
  2.2 import_photos 管线修改
  2.3 iOS 平台修改
  2.4 Android 平台修改

Phase 3  ────────────────  前端 UI              ~3-4 天
  3.1 shared/types.ts
  3.2 platform/desktop.ts
  3.3 photo-grouping.ts
  3.4 VirtualPhotoGrid badge
  3.5 Desktop ImagePreview
  3.6 Mobile ImagePreview
  3.7 平台服务接口
  3.8 Map 组件
  3.9 App.svelte 传参

Phase 4  ────────────────  Live Photo 专项      ~2-3 天
  4.1 iOS Swift 原生识别
  4.2 Desktop 配对验证
  4.3 端到端测试

总估时：~11-15 天（单人全职）
```
