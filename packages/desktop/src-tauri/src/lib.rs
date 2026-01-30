use footos_core::{PhotoCoreConfig, PhotoIndex, ImportResult, PhotoInfo, PhotoPreview, DEFAULT_THUMBNAIL_SIZE};
use rayon::prelude::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

// Global cancellation flag for import operations
static IMPORT_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Application state: shared PhotoIndex connection + cached paths.
///
/// Registered as Tauri managed state at app startup.
/// All DB-using commands receive `State<AppState>` instead of opening
/// a new `PhotoIndex` per call — eliminates repeated schema checks,
/// PRAGMA setup, and connection overhead.
struct AppState {
    index: Mutex<Option<Arc<PhotoIndex>>>,
    db_path: Mutex<String>,
    thumb_dir: Mutex<String>,
}

impl AppState {
    fn new() -> Self {
        Self {
            index: Mutex::new(None),
            db_path: Mutex::new(String::new()),
            thumb_dir: Mutex::new(String::new()),
        }
    }

    /// Initialize (or re-initialize) the database connection and store paths.
    /// Called once at app startup from `init_app`, and again after `clear_app_data`.
    fn init(&self, db_path: String, thumb_dir: String) -> Result<(), String> {
        if let Some(parent) = std::path::Path::new(&db_path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;

        let index = PhotoIndex::open(db_path.clone()).map_err(|e| {
            println!("[AppState::init] Failed to open DB: {}", e);
            e.to_string()
        })?;

        *self.db_path.lock().map_err(|e| e.to_string())? = db_path;
        *self.thumb_dir.lock().map_err(|e| e.to_string())? = thumb_dir;
        *self.index.lock().map_err(|e| e.to_string())? = Some(index);

        Ok(())
    }

    /// Get the shared PhotoIndex, or error if not initialized.
    fn get_index(&self) -> Result<Arc<PhotoIndex>, String> {
        self.index
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or_else(|| "Database not initialized. Call init_app first.".to_string())
    }

    fn get_db_path(&self) -> Result<String, String> {
        Ok(self.db_path.lock().map_err(|e| e.to_string())?.clone())
    }

    fn get_thumb_dir(&self) -> Result<String, String> {
        Ok(self.thumb_dir.lock().map_err(|e| e.to_string())?.clone())
    }

    /// Drop the cached connection (before deleting DB files).
    fn reset_index(&self) -> Result<(), String> {
        *self.index.lock().map_err(|e| e.to_string())? = None;
        Ok(())
    }
}

#[tauri::command]
fn cancel_import() {
    IMPORT_CANCELLED.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_core_version() -> String {
    footos_core::get_version()
}

/// Initialize the app state with database and thumbnail paths.
/// Must be called once from the frontend at startup, before any DB commands.
#[tauri::command]
fn init_app(state: tauri::State<'_, AppState>, db_path: String, thumb_dir: String) -> Result<(), String> {
    println!("[init_app] db: {}, thumbs: {}", db_path, thumb_dir);
    state.init(db_path, thumb_dir)
}

/// Lightweight preview list for map markers and timeline.
/// Returns only the fields needed for initial UI rendering.
/// Zero file I/O -- all data comes from the database.
/// Typically completes in <50ms even for 10K+ photos.
#[tauri::command]
async fn list_photos_preview(state: tauri::State<'_, AppState>) -> Result<Vec<PhotoPreview>, String> {
    println!("[list_photos_preview] Called");

    let index = state.get_index()?;

    let previews = index.list_previews().map_err(|e| {
        println!("[list_photos_preview] Error: {}", e);
        e.to_string()
    })?;
    println!("[list_photos_preview] Found {} previews", previews.len());

    Ok(previews)
}

#[tauri::command]
async fn list_photos(state: tauri::State<'_, AppState>) -> Result<Vec<PhotoInfo>, String> {
    println!("[list_photos] Called");

    let index = state.get_index()?;
    let thumb_dir = state.get_thumb_dir()?;

    let mut photos = index.list().map_err(|e| {
        println!("[list_photos] Error listing photos: {}", e);
        e.to_string()
    })?;
    println!("[list_photos] Found {} photos", photos.len());

    // Only resolve thumb_path/file_size for photos that don't have them in the DB.
    // Photos imported after the preview aggregation update will already have these values.
    let thumbnailer = footos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
    let spec = footos_core::ThumbnailSpec::default();
    let mut backfill_count = 0u32;
    let use_txn = index.begin_transaction().is_ok();
    for photo in &mut photos {
        // Skip if already populated from DB
        if photo.thumb_path.is_some() && photo.file_size > 0 {
            continue;
        }

        let source_path = std::path::Path::new(&photo.path);

        // Backfill thumb_path
        if photo.thumb_path.is_none() {
            match thumbnailer.get_cached_path(source_path, &spec) {
                Ok(Some(path)) => {
                    photo.thumb_path = Some(path.to_string_lossy().to_string());
                }
                _ => {}
            }
        }

        // Backfill file_size
        if photo.file_size == 0 {
            if let Ok(metadata) = std::fs::metadata(source_path) {
                photo.file_size = metadata.len();
            }
        }

        // Persist the backfilled data so next call is instant
        if photo.thumb_path.is_some() || photo.file_size > 0 {
            let _ = index.update_thumb_info(
                photo.path.clone(),
                photo.thumb_path.clone(),
                photo.file_size,
            );
            backfill_count += 1;
        }
    }
    if use_txn && backfill_count > 0 {
        let _ = index.commit_transaction();
    }

    if backfill_count > 0 {
        println!("[list_photos] Backfilled thumb info for {} photos", backfill_count);
    }

    Ok(photos)
}

/// Result of parallel Phase 2 processing for a single photo.
enum ProcessResult {
    /// Successfully processed — ready for DB insert.
    New(ProcessedPhoto),
    /// Processing failed (hash, metadata, or thumbnail error).
    Failed(String),
    /// Import was cancelled before this photo was processed.
    Cancelled,
}

/// Extended import result with timing data (Tauri layer only).
/// Core's ImportResult doesn't include timing — that's a platform concern.
#[derive(serde::Serialize, Default)]
struct ImportResultWithTiming {
    success: u32,
    failure: u32,
    duplicates: u32,
    skipped: u32,
    total: u32,
    total_ms: u64,
    phase1_ms: u64,
    phase2_ms: u64,
    phase3_ms: u64,
}

/// All data needed to insert a photo into the DB.
/// Produced in parallel (Phase 2), consumed sequentially (Phase 3).
struct ProcessedPhoto {
    path: std::path::PathBuf,
    path_str: String,
    hash: String,
    metadata: footos_core::PhotoMetadata,
    thumb_path_str: Option<String>,
    file_size: u64,
}

#[tauri::command]
async fn import_photos(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    root_path: String,
) -> Result<ImportResultWithTiming, String> {
    let import_start = std::time::Instant::now();
    println!("[Import] Starting import for: {}", root_path);

    let index = state.get_index()?;
    let thumb_dir = state.get_thumb_dir()?;

    // Handle file:// URIs (iOS returns these)
    // Keep stripping file:// prefix until we get a plain path
    let mut root_path = root_path;
    loop {
        if root_path.starts_with("file://") {
            let decoded = urlencoding::decode(&root_path[7..])
                .map_err(|e| format!("Failed to decode file URI: {}", e))?
                .into_owned();
            println!("[Import] Decoded file URI: {} -> {}", root_path, decoded);
            root_path = decoded;
        } else {
            break;
        }
    }
    println!("[Import] Final path: {}", root_path);

    // Reset cancellation flag at start
    IMPORT_CANCELLED.store(false, Ordering::SeqCst);

    let config = PhotoCoreConfig {
        thumbnail_dir: thumb_dir.clone(),
        thumbnail_size: DEFAULT_THUMBNAIL_SIZE,
    };

    let root_path_buf = std::path::Path::new(&root_path);

    // Check if path exists
    println!("[Import] Checking path exists: {}", root_path_buf.exists());
    println!("[Import] Is file: {}", root_path_buf.is_file());
    println!("[Import] Is dir: {}", root_path_buf.is_dir());

    // Quick check: if file doesn't exist, return early with error
    if !root_path_buf.exists() {
        println!("[Import] File does not exist: {}", root_path);
        return Err(format!("File does not exist: {}", root_path));
    }

    // Create permanent storage directory for imported photos
    let photos_dir = std::path::Path::new(&config.thumbnail_dir).parent()
        .map(|p| p.join("Photos"))
        .unwrap_or_else(|| std::path::PathBuf::from(&config.thumbnail_dir).join("Photos"));
    std::fs::create_dir_all(&photos_dir).map_err(|e| {
        println!("[Import] Failed to create photos dir: {}", e);
        e.to_string()
    })?;
    println!("[Import] Photos storage dir: {:?}", photos_dir);

    // Support both single file and directory import
    let photos = if root_path_buf.is_file() {
        println!("[Import] Processing as single file");
        // Verify file is readable
        if let Err(e) = std::fs::metadata(&root_path_buf) {
            println!("[Import] Cannot read file metadata: {}", e);
            return Err(format!("Cannot read file: {}", e));
        }

        // Copy file to permanent storage (iOS temp files get deleted)
        let filename = root_path_buf.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("photo_{}.jpg", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)));
        let dest_path = photos_dir.join(&filename);

        // Only copy if source and dest are different
        if root_path_buf != dest_path {
            println!("[Import] Copying to permanent storage: {:?}", dest_path);
            std::fs::copy(&root_path_buf, &dest_path).map_err(|e| {
                println!("[Import] Failed to copy file: {}", e);
                e.to_string()
            })?;
            vec![dest_path]
        } else {
            vec![root_path_buf.to_path_buf()]
        }
    } else {
        println!("[Import] Processing as directory");
        footos_core::scan_photos(root_path_buf).map_err(|e| {
            println!("[Import] Scan error: {}", e);
            e.to_string()
        })?
    };
    let total = photos.len();
    println!("[Import] Found {} photos to process", total);

    let mut result = ImportResult::default();

    // If no photos found, return early
    if total == 0 {
        println!("[Import] No photos to import");
        return Ok(ImportResultWithTiming {
            total: total as u32,
            total_ms: import_start.elapsed().as_millis() as u64,
            ..Default::default()
        });
    }

    // ── Phase 1 (Sequential): Filter already-imported photos ──
    // DB lookups are fast (~0.01ms each) and SQLite is single-threaded anyway.
    let phase1_start = std::time::Instant::now();
    let mut new_photos: Vec<std::path::PathBuf> = Vec::with_capacity(total);
    let mut skipped = 0usize;
    for path in &photos {
        if IMPORT_CANCELLED.load(Ordering::SeqCst) {
            break;
        }
        let path_str = path.to_string_lossy().to_string();
        if let Ok(Some(_)) = index.get_by_path(path_str) {
            skipped += 1;
        } else {
            new_photos.push(path.clone());
        }
    }
    let new_count = new_photos.len();
    let phase1_ms = phase1_start.elapsed().as_millis();
    println!(
        "[Import] Phase 1 done in {}ms: {} new, {} already imported",
        phase1_ms, new_count, skipped
    );

    // Emit initial progress (skipped photos accounted for)
    {
        use tauri::Emitter;
        let _ = window.emit(
            "import-progress",
            serde_json::json!({
                "current": skipped,
                "total": total,
                "success": 0,
                "failure": 0,
                "skipped": skipped,
                "last_path": ""
            }),
        );
    }

    if IMPORT_CANCELLED.load(Ordering::SeqCst) {
        use tauri::Emitter;
        let _ = window.emit(
            "import-cancelled",
            serde_json::json!({
                "current": skipped,
                "total": total,
                "success": 0,
                "failure": 0
            }),
        );
        return Ok(ImportResultWithTiming {
            skipped: skipped as u32,
            total: total as u32,
            phase1_ms: phase1_ms as u64,
            total_ms: import_start.elapsed().as_millis() as u64,
            ..Default::default()
        });
    }

    // ── Phase 2 (Parallel): CPU+IO heavy work via rayon ──
    // compute_hash (~50-200ms) + read_metadata (~10-50ms) +
    // generate_thumbnail (~100-500ms) + fs::metadata (<1ms)
    // All pure functions from footos_core — no DB access, no shared mutable state.
    let phase2_start = std::time::Instant::now();
    let processed_counter = AtomicUsize::new(0);
    let window_clone = window.clone();

    let processed: Vec<ProcessResult> = new_photos
        .par_iter()
        .map(|path| {
            // Check cancellation early
            if IMPORT_CANCELLED.load(Ordering::Relaxed) {
                return ProcessResult::Cancelled;
            }

            let path_str = path.to_string_lossy().to_string();

            // Compute perceptual hash
            let hash = match footos_core::compute_hash(path) {
                Ok(h) => h,
                Err(e) => {
                    println!("[Import] Hash error for {}: {}", path_str, e);
                    return ProcessResult::Failed(path_str);
                }
            };

            // Check cancellation between expensive steps
            if IMPORT_CANCELLED.load(Ordering::Relaxed) {
                return ProcessResult::Cancelled;
            }

            // Read EXIF metadata
            let metadata = match footos_core::read_metadata(path) {
                Ok(m) => m,
                Err(e) => {
                    println!("[Import] Metadata error for {}: {}", path_str, e);
                    return ProcessResult::Failed(path_str);
                }
            };

            // Generate thumbnail (may fail for some formats — OK, frontend uses original)
            let thumb_path_str = footos_core::generate_thumbnail(path, &config)
                .ok()
                .map(|p| p.to_string_lossy().to_string());

            // Get file size
            let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

            // Emit progress every ~10 photos to avoid IPC flooding
            let count = processed_counter.fetch_add(1, Ordering::Relaxed) + 1;
            if count % 10 == 0 || count == new_count {
                use tauri::Emitter;
                let _ = window_clone.emit(
                    "import-progress",
                    serde_json::json!({
                        "current": skipped + count,
                        "total": total,
                        "success": 0,
                        "failure": 0,
                        "skipped": skipped,
                        "phase": "processing",
                        "last_path": path_str
                    }),
                );
            }

            ProcessResult::New(ProcessedPhoto {
                path: path.clone(),
                path_str,
                hash,
                metadata,
                thumb_path_str,
                file_size,
            })
        })
        .collect();

    let phase2_ms = phase2_start.elapsed().as_millis();
    println!(
        "[Import] Phase 2 done in {}ms: {} items processed in parallel",
        phase2_ms, processed.len()
    );

    // ── Phase 3 (Sequential): Deduplicate by hash + DB inserts in batched transactions ──
    let phase3_start = std::time::Instant::now();
    const BATCH_SIZE: usize = 100;
    let mut batch_writes = 0usize;
    let use_txn = index.begin_transaction().is_ok();
    let mut cancelled = false;

    for (i, item) in processed.into_iter().enumerate() {
        match item {
            ProcessResult::Cancelled => {
                cancelled = true;
                break;
            }
            ProcessResult::Failed(_path_str) => {
                result.failure += 1;
            }
            ProcessResult::New(photo) => {
                // Deduplicate by hash (another photo with same content may already exist)
                if index.exists_by_hash(&photo.hash).unwrap_or(false) {
                    println!("[Import] DUPLICATE (by hash): {}", photo.path_str);
                    // Clean up copied file for single-file imports (iOS)
                    let _ = std::fs::remove_file(&photo.path);
                    result.duplicates += 1;
                } else {
                    // Insert into DB
                    match index.insert(
                        photo.path_str.clone(),
                        photo.hash,
                        photo.metadata,
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
                            println!("[Import] DB insert error: {}", e);
                            result.failure += 1;
                        }
                    }
                }
            }
        }

        // Intermediate commit every BATCH_SIZE writes for crash safety
        if use_txn && batch_writes >= BATCH_SIZE {
            let _ = index.commit_transaction();
            let _ = index.begin_transaction();
            batch_writes = 0;
        }

        // Emit progress during Phase 3 (every photo, DB inserts are fast)
        use tauri::Emitter;
        let _ = window.emit(
            "import-progress",
            serde_json::json!({
                "current": skipped + i + 1,
                "total": total,
                "success": result.success,
                "failure": result.failure,
                "duplicates": result.duplicates,
                "skipped": skipped,
                "phase": "importing"
            }),
        );
    }

    // Commit any remaining writes
    if use_txn && batch_writes > 0 {
        let _ = index.commit_transaction();
    }

    if cancelled {
        use tauri::Emitter;
        let _ = window.emit(
            "import-cancelled",
            serde_json::json!({
                "current": skipped + result.success as usize + result.failure as usize + result.duplicates as usize,
                "total": total,
                "success": result.success,
                "failure": result.failure
            }),
        );
    }

    let phase3_ms = phase3_start.elapsed().as_millis() as u64;
    let total_ms = import_start.elapsed().as_millis() as u64;
    let phase1_ms = phase1_ms as u64;
    let phase2_ms = phase2_ms as u64;
    println!(
        "[Import] Phase 3 done in {}ms",
        phase3_ms
    );
    println!(
        "[Import] Complete! Total: {}ms (Phase1: {}ms, Phase2: {}ms, Phase3: {}ms)",
        total_ms, phase1_ms, phase2_ms, phase3_ms
    );
    println!(
        "[Import] Results: Success: {}, Failure: {}, Duplicates: {}, Skipped: {}",
        result.success, result.failure, result.duplicates, skipped
    );
    Ok(ImportResultWithTiming {
        success: result.success,
        failure: result.failure,
        duplicates: result.duplicates,
        skipped: skipped as u32,
        total: total as u32,
        total_ms,
        phase1_ms,
        phase2_ms,
        phase3_ms,
    })
}

/// Delete result struct
#[derive(serde::Serialize, Default)]
struct DeleteResult {
    deleted_count: usize,
    deleted_paths: Vec<String>,
    errors: Vec<String>,
}

/// Delete photos from app only (DB + thumbnails), keep original files
#[tauri::command]
async fn delete_photos_from_app(
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<DeleteResult, String> {
    let index = state.get_index()?;
    let thumb_dir = state.get_thumb_dir()?;

    let thumbnailer = footos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
    let spec = footos_core::ThumbnailSpec::default();

    let mut result = DeleteResult::default();

    // Delete each photo from DB and remove its thumbnail
    let deleted_photos = index.delete_by_ids(ids).map_err(|e| e.to_string())?;

    for photo in deleted_photos {
        result.deleted_paths.push(photo.path.clone());
        result.deleted_count += 1;

        // Try to delete thumbnail
        let source_path = std::path::Path::new(&photo.path);
        if let Ok(Some(thumb_path)) = thumbnailer.get_cached_path(source_path, &spec) {
            if let Err(e) = std::fs::remove_file(&thumb_path) {
                result.errors.push(format!("Failed to delete thumbnail {}: {}", thumb_path.display(), e));
            }
        }
    }

    Ok(result)
}

/// Delete photos completely (DB + thumbnails + original files)
#[tauri::command]
async fn delete_photos_completely(
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<DeleteResult, String> {
    let index = state.get_index()?;
    let thumb_dir = state.get_thumb_dir()?;

    let thumbnailer = footos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
    let spec = footos_core::ThumbnailSpec::default();

    let mut result = DeleteResult::default();

    // Delete each photo from DB and remove its thumbnail + original file
    let deleted_photos = index.delete_by_ids(ids).map_err(|e| e.to_string())?;

    for photo in deleted_photos {
        result.deleted_paths.push(photo.path.clone());
        result.deleted_count += 1;

        // Try to delete thumbnail
        let source_path = std::path::Path::new(&photo.path);
        if let Ok(Some(thumb_path)) = thumbnailer.get_cached_path(source_path, &spec) {
            if let Err(e) = std::fs::remove_file(&thumb_path) {
                result.errors.push(format!("Failed to delete thumbnail {}: {}", thumb_path.display(), e));
            }
        }

        // Delete original file
        if let Err(e) = std::fs::remove_file(&photo.path) {
            result.errors.push(format!("Failed to delete original {}: {}", photo.path, e));
        }
    }

    Ok(result)
}

#[tauri::command]
async fn clear_app_data(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let thumb_dir = state.get_thumb_dir()?;
    let db_path = state.get_db_path()?;

    // Drop the cached DB connection before deleting files
    state.reset_index()?;

    // Clear thumbnails
    if std::path::Path::new(&thumb_dir).exists() {
        std::fs::remove_dir_all(&thumb_dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;

    // Clear database (main file + WAL + SHM)
    if std::path::Path::new(&db_path).exists() {
        std::fs::remove_file(&db_path).map_err(|e| e.to_string())?;
    }
    let wal_path = format!("{}-wal", db_path);
    let shm_path = format!("{}-shm", db_path);
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);

    // Re-initialize with a fresh database
    state.init(db_path, thumb_dir)?;

    // Note: map_tiles directory is preserved
    Ok(())
}

#[tauri::command]
async fn regenerate_thumbnails(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let index = state.get_index()?;
    let thumb_dir = state.get_thumb_dir()?;

    std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;

    let photos = index.list().map_err(|e| e.to_string())?;
    let total = photos.len();

    let config = PhotoCoreConfig {
        thumbnail_dir: thumb_dir,
        thumbnail_size: DEFAULT_THUMBNAIL_SIZE,
    };

    let mut success = 0;
    let mut failure = 0;

    // Batch transaction for thumbnail DB updates
    const BATCH_SIZE: usize = 100;
    let mut batch_writes = 0usize;
    let use_txn = index.begin_transaction().is_ok();

    for (i, photo) in photos.iter().enumerate() {
        let path = std::path::PathBuf::from(&photo.path);

        let file_result = footos_core::generate_thumbnail(&path, &config);

        match &file_result {
            Ok(thumb_path) => {
                success += 1;
                batch_writes += 1;
                // Persist the new thumbnail path in DB
                let thumb_str = thumb_path.to_string_lossy().to_string();
                let file_size = std::fs::metadata(&path)
                    .map(|m| m.len())
                    .unwrap_or(photo.file_size);
                let _ = index.update_thumb_info(
                    photo.path.clone(),
                    Some(thumb_str),
                    file_size,
                );
            }
            Err(_) => failure += 1,
        }

        // Intermediate commit every BATCH_SIZE writes
        if use_txn && batch_writes >= BATCH_SIZE {
            let _ = index.commit_transaction();
            let _ = index.begin_transaction();
            batch_writes = 0;
        }

        // Emit progress
        use tauri::Emitter;
        let _ = window.emit("import-progress", serde_json::json!({
            "current": i + 1,
            "total": total,
            "success": success,
            "failure": failure,
            "last_path": photo.path
        }));
    }

    // Commit any remaining writes
    if use_txn && batch_writes > 0 {
        let _ = index.commit_transaction();
    }

    Ok(())
}

#[tauri::command]
async fn read_file_bytes(path: String) -> Result<Vec<u8>, String> {
    std::fs::read(&path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_raw_preview(path: String, cache_dir: String) -> Result<String, String> {
    let source_path = std::path::Path::new(&path);

    // Create a unique cache filename based on the source path
    let file_name = source_path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("preview");
    let hash = footos_core::compute_hash(source_path).map_err(|e| e.to_string())?;
    let hash_prefix = &hash[..hash.len().min(16)];
    let preview_path = std::path::PathBuf::from(&cache_dir)
        .join("raw_previews")
        .join(format!("{}_{}.jpg", file_name, hash_prefix));

    // Return cached preview if it exists
    if preview_path.exists() {
        return Ok(preview_path.to_string_lossy().to_string());
    }

    // Extract and cache the preview
    let preview_bytes = footos_core::extract_raw_preview(source_path).map_err(|e| e.to_string())?;

    // Ensure directory exists
    if let Some(parent) = preview_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // Write to cache
    std::fs::write(&preview_path, &preview_bytes).map_err(|e| e.to_string())?;

    Ok(preview_path.to_string_lossy().to_string())
}

/// Request photo library access on iOS
/// This triggers the Swift photo picker via JavaScript bridge
#[tauri::command]
async fn request_photo_library_access(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    #[cfg(target_os = "ios")]
    {
        let db_path = state.get_db_path()?;
        let thumb_dir = state.get_thumb_dir()?;
        // On iOS, we trigger the Swift photo picker via JavaScript
        // The Swift code will call back with selected photos
        use tauri::Emitter;
        window.emit("show-ios-photo-picker", serde_json::json!({
            "dbPath": db_path,
            "thumbDir": thumb_dir
        })).map_err(|e| e.to_string())?;
        Ok("Photo picker triggered".to_string())
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (window, state);
        Err("Photo library access is only available on iOS. Use folder/file import on desktop.".to_string())
    }
}

/// Process a photo from iOS photo picker
#[tauri::command]
async fn process_ios_photo(
    state: tauri::State<'_, AppState>,
    photo_json: String,
) -> Result<bool, String> {
    use serde_json::Value;

    let index = state.get_index()?;
    let thumb_dir = state.get_thumb_dir()?;

    let photo: Value = serde_json::from_str(&photo_json)
        .map_err(|e| format!("Invalid photo JSON: {}", e))?;

    let path = photo["path"].as_str()
        .ok_or("Missing path in photo data")?;

    let identifier = photo["identifier"].as_str()
        .unwrap_or("unknown");

    let source_path = std::path::Path::new(path);

    // Read metadata
    let metadata = footos_core::read_metadata(source_path)
        .map_err(|e| format!("Failed to read metadata: {}", e))?;

    // Compute hash
    let hash = footos_core::compute_hash(source_path)
        .map_err(|e| format!("Failed to compute hash: {}", e))?;

    // Generate thumbnail
    let config = footos_core::PhotoCoreConfig {
        thumbnail_dir: thumb_dir,
        thumbnail_size: DEFAULT_THUMBNAIL_SIZE,
    };
    let thumb_path_result = footos_core::generate_thumbnail(source_path, &config);
    let thumb_path_str = thumb_path_result.as_ref().ok()
        .map(|p| p.to_string_lossy().to_string());

    // Get file size for preview aggregation
    let file_size = std::fs::metadata(source_path)
        .map(|m| m.len())
        .unwrap_or(0);

    // Use iOS photo identifier as the path prefix for reference
    let stored_path = format!("ios-photo://{}", identifier);

    index.insert(stored_path.clone(), hash, metadata)
        .map_err(|e| format!("Failed to insert photo: {}", e))?;

    // Persist thumb_path and file_size for instant preview loading
    let _ = index.update_thumb_info(stored_path, thumb_path_str, file_size);

    // Clean up temp file
    let _ = std::fs::remove_file(path);

    Ok(true)
}

#[tauri::command]
async fn get_cached_tile(cache_dir: String, z: u32, x: u32, y: u32) -> Result<Option<String>, String> {
    let tile_path = std::path::PathBuf::from(&cache_dir)
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{}.png", y));

    if tile_path.exists() {
        Ok(Some(tile_path.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

#[tauri::command]
async fn download_tile(cache_dir: String, z: u32, x: u32, y: u32, url: String) -> Result<String, String> {
    let tile_path = std::path::PathBuf::from(&cache_dir)
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{}.png", y));

    // Check if already cached
    if tile_path.exists() {
        return Ok(tile_path.to_string_lossy().to_string());
    }

    // Create directory structure
    if let Some(parent) = tile_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // Download tile
    let response = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;

    // Save to cache
    std::fs::write(&tile_path, &bytes).map_err(|e| e.to_string())?;

    Ok(tile_path.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .invoke_handler(tauri::generate_handler![
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
            import_all_ios_photos
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Import all authorized photos from iOS photo library
/// This is the iOS-specific implementation that uses PhotoKit directly
#[tauri::command]
async fn import_all_ios_photos(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<ImportResult, String> {
    #[cfg(target_os = "ios")]
    {
        let db_path = state.get_db_path()?;
        let thumb_dir = state.get_thumb_dir()?;

        // On iOS, we need to use the native PhotoKit API
        // For now, emit an event to trigger Swift code via JavaScript injection
        use tauri::Emitter;
        window.emit("trigger-ios-import-all", serde_json::json!({
            "dbPath": db_path,
            "thumbDir": thumb_dir
        })).map_err(|e| e.to_string())?;

        // Return a pending result - actual results come via events
        Ok(ImportResult {
            success: 0,
            failure: 0,
            duplicates: 0,
        })
    }

    #[cfg(not(target_os = "ios"))]
    {
        let _ = (window, state);
        Err("This command is only available on iOS".to_string())
    }
}
