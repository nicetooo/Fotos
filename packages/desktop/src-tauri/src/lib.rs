use footos_core::{AlbumWindow, Bbox, GeoIndex, MapFeature, PhotoCoreConfig, PhotoIndex, ImportResult, PhotoInfo, PhotoPreview, DEFAULT_THUMBNAIL_SIZE, MediaType};
use tauri::Manager;
use rayon::prelude::*;
use std::collections::HashSet;
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
    /// Cached query index, keyed by (connection generation, DB change counter).
    geo: Mutex<Option<((u64, i64), Arc<GeoIndex>)>>,
    generation: std::sync::atomic::AtomicU64,
}

impl AppState {
    fn new() -> Self {
        Self {
            index: Mutex::new(None),
            db_path: Mutex::new(String::new()),
            thumb_dir: Mutex::new(String::new()),
            geo: Mutex::new(None),
            generation: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// The in-memory query index, rebuilt lazily whenever the DB has changed.
    /// Returns the index and a version token the frontend can compare.
    fn geo(&self) -> Result<(Arc<GeoIndex>, String), String> {
        let index = self.get_index()?;
        let key = (
            self.generation.load(Ordering::SeqCst),
            index.change_counter().map_err(|e| e.to_string())?,
        );
        let version = format!("{}-{}", key.0, key.1);
        let mut cached = self.geo.lock().map_err(|e| e.to_string())?;
        if let Some((k, geo)) = cached.as_ref() {
            if *k == key {
                return Ok((geo.clone(), version));
            }
        }
        let started = std::time::Instant::now();
        let thumb_dir = self.get_thumb_dir()?;
        let mut rows = index.list_previews_filtered(true).map_err(|e| e.to_string())?;
        for row in &mut rows {
            row.thumb_path = Some(resolve_thumb(&thumb_dir, &row.path, row.thumb_path.take()));
        }
        let geo = Arc::new(GeoIndex::build(rows));
        tracing::info!(rows = geo.len(), ms = started.elapsed().as_millis() as u64, "geo index rebuilt");
        *cached = Some((key, geo.clone()));
        Ok((geo, version))
    }

    /// Initialize (or re-initialize) the database connection and store paths.
    /// Called once at app startup from `init_app`, and again after `clear_app_data`.
    /// Reuses the live connection on WebView remount so an in-flight import is not rolled back.
    fn init(&self, db_path: String, thumb_dir: String) -> Result<(), String> {
        if let Some(parent) = std::path::Path::new(&db_path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;

        {
            let current_path = self.db_path.lock().map_err(|e| e.to_string())?;
            let index_guard = self.index.lock().map_err(|e| e.to_string())?;
            if *current_path == db_path {
                if let Some(index) = index_guard.as_ref() {
                    let rows = index.count().unwrap_or(-1);
                    drop(index_guard);
                    drop(current_path);
                    *self.thumb_dir.lock().map_err(|e| e.to_string())? = thumb_dir;
                    eprintln!("[footos] init_app reused live DB ({rows} rows) {db_path}");
                    return Ok(());
                }
            }
            if let Some(old) = index_guard.as_ref() {
                let _ = old.flush();
            }
        }

        let open_started = std::time::Instant::now();
        let index = {
            #[cfg(target_os = "ios")]
            {
                PhotoIndex::open_with_journal(db_path.clone(), "DELETE")
            }
            #[cfg(not(target_os = "ios"))]
            {
                PhotoIndex::open(db_path.clone())
            }
        }
        .map_err(|e| {
            tracing::error!(error = %e, "AppState::init: failed to open DB");
            e.to_string()
        })?;
        eprintln!("[footos] PhotoIndex::open returned in {:?}", open_started.elapsed());

        *self.db_path.lock().map_err(|e| e.to_string())? = db_path.clone();
        *self.thumb_dir.lock().map_err(|e| e.to_string())? = thumb_dir;
        *self.index.lock().map_err(|e| e.to_string())? = Some(index.clone());
        self.generation.fetch_add(1, Ordering::SeqCst);
        eprintln!(
            "[footos] init_app opened {} ({} rows)",
            db_path,
            index.count().unwrap_or(-1)
        );

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
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self.geo.lock().map_err(|e| e.to_string())? = None;
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
async fn init_app(app: tauri::AppHandle, db_path: String, thumb_dir: String) -> Result<(), String> {
    tracing::info!(db = %db_path, thumbs = %thumb_dir, "init_app");
    // iOS WebKit handles invoke on the UI thread. Do the SQLite open off-thread
    // or the WebView freezes for the entire journal/schema setup.
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let result = app.state::<AppState>().init(db_path, thumb_dir);
        eprintln!("[footos] init_app finished in {:?}", started.elapsed());
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn flush_photos_db(state: tauri::State<'_, AppState>) -> Result<i64, String> {
    let index = state.get_index()?;
    index.flush().map_err(|e| e.to_string())?;
    let rows = index.count().unwrap_or(-1);
    eprintln!("[footos] flush_photos_db rows={rows}");
    Ok(rows)
}

/// Lightweight preview list for map markers and timeline.
/// Returns only the fields needed for initial UI rendering.
/// Zero file I/O -- all data comes from the database.
/// Typically completes in <50ms even for 10K+ photos.
#[tauri::command]
async fn list_photos_preview(
    app: tauri::AppHandle,
    geotagged_only: Option<bool>,
) -> Result<Vec<PhotoPreview>, String> {
    tracing::trace!("list_photos_preview: called");
    let geotagged_only = geotagged_only.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let index = state.get_index()?;
        let started = std::time::Instant::now();
        let previews = index
            .list_previews_filtered(geotagged_only)
            .map_err(|e| {
                tracing::error!(error = %e, "list_photos_preview: query failed");
                e.to_string()
            })?;
        eprintln!(
            "[footos] list_photos_preview geotagged_only={geotagged_only} {} rows in {:?}",
            previews.len(),
            started.elapsed()
        );
        Ok(previews)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Columnar geotagged previews written to a file. Returning 5k+ objects through
/// Tauri invoke on iOS WKWebView took ~8s; JS fetch of the same JSON is faster.
#[derive(serde::Serialize)]
struct MapPreviewPack {
    id: Vec<i64>,
    path: Vec<String>,
    thumb: Vec<String>,
    lat: Vec<f64>,
    lon: Vec<f64>,
    date: Vec<String>,
    media: Vec<String>,
    dur: Vec<u64>,
}

#[derive(serde::Serialize)]
struct MapPreviewFile {
    path: String,
    rows: usize,
    bytes: usize,
    rust_ms: u64,
}

#[tauri::command]
async fn list_map_previews(app: tauri::AppHandle) -> Result<MapPreviewFile, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let state = app.state::<AppState>();
        let rows = state
            .get_index()?
            .list_previews_filtered(true)
            .map_err(|e| e.to_string())?;
        let n = rows.len();
        let mut pack = MapPreviewPack {
            id: Vec::with_capacity(n),
            path: Vec::with_capacity(n),
            thumb: Vec::with_capacity(n),
            lat: Vec::with_capacity(n),
            lon: Vec::with_capacity(n),
            date: Vec::with_capacity(n),
            media: Vec::with_capacity(n),
            dur: Vec::with_capacity(n),
        };
        for row in rows {
            let Some(lat) = row.lat else { continue };
            let Some(lon) = row.lon else { continue };
            pack.id.push(row.id);
            pack.path.push(row.path);
            pack.thumb.push(
                row.thumb_path
                    .as_deref()
                    .and_then(|p| std::path::Path::new(p).file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
            pack.lat.push(lat);
            pack.lon.push(lon);
            pack.date.push(row.date_taken.unwrap_or_default());
            pack.media.push(row.media_type);
            pack.dur.push(row.duration_ms.unwrap_or(0));
        }
        let bytes = serde_json::to_vec(&pack).map_err(|e| e.to_string())?;
        let thumb_dir = state.get_thumb_dir()?;
        let out = std::path::Path::new(&thumb_dir).join("map_previews.json");
        std::fs::write(&out, &bytes).map_err(|e| e.to_string())?;
        let rust_ms = started.elapsed().as_millis() as u64;
        eprintln!(
            "[footos] list_map_previews {} rows {} bytes in {rust_ms}ms",
            pack.path.len(),
            bytes.len()
        );
        Ok(MapPreviewFile {
            path: out.to_string_lossy().into_owned(),
            rows: pack.path.len(),
            bytes: bytes.len(),
            rust_ms,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Image to draw for an item. iOS library rows store an absolute thumbnail path
/// inside a container whose UUID changes on reinstall, so re-anchor those to
/// the current thumbnail directory. Rows without a thumbnail fall back to the
/// original file (the frontend scales it down).
fn resolve_thumb(thumb_dir: &str, path: &str, stored: Option<String>) -> String {
    let virtual_path = path.starts_with("ios-photo://") || path.starts_with("content://");
    match stored.filter(|s| !s.is_empty()) {
        Some(t) if !virtual_path || t.starts_with(thumb_dir) => t,
        Some(t) => std::path::Path::new(&t)
            .file_name()
            .map(|n| {
                std::path::Path::new(thumb_dir)
                    .join(n)
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or(t),
        None if virtual_path => String::new(),
        None => path.to_string(),
    }
}

#[derive(serde::Serialize)]
struct MapQueryResponse {
    features: Vec<MapFeature>,
    /// Items inside the box and time range, before clustering.
    matched: u32,
    /// Geotagged items in the whole library.
    total: u32,
    version: String,
}

/// Clusters and single items inside the viewport. Cost depends on what is on
/// screen plus one linear scan in Rust; nothing library-sized crosses IPC.
#[tauri::command]
async fn query_map(
    app: tauri::AppHandle,
    bbox: Bbox,
    zoom: f64,
    start_ms: Option<f64>,
    end_ms: Option<f64>,
) -> Result<MapQueryResponse, String> {
    let (geo, version) = app.state::<AppState>().geo()?;
    let result = geo.query_map(bbox, zoom, start_ms.map(|v| v as i64), end_ms.map(|v| v as i64));
    Ok(MapQueryResponse {
        features: result.features,
        matched: result.matched,
        total: geo.len() as u32,
        version,
    })
}

#[derive(serde::Serialize)]
struct GeoSummary {
    total: u32,
    /// `[west, south, east, north]` of all geotagged items.
    extent: Option<[f64; 4]>,
    version: String,
}

#[tauri::command]
async fn geo_summary(app: tauri::AppHandle) -> Result<GeoSummary, String> {
    let (geo, version) = app.state::<AppState>().geo()?;
    Ok(GeoSummary { total: geo.len() as u32, extent: geo.extent(), version })
}

/// Sorted wall-clock timestamps (little-endian f64 ms) for the timeline,
/// returned as raw bytes so no JSON is involved.
#[tauri::command]
async fn query_timeline(
    app: tauri::AppHandle,
    bbox: Option<Bbox>,
) -> Result<tauri::ipc::Response, String> {
    let (geo, _) = app.state::<AppState>().geo()?;
    let stamps = geo.timestamps(bbox);
    let mut bytes = Vec::with_capacity(stamps.len() * 8);
    for t in stamps {
        bytes.extend_from_slice(&t.to_le_bytes());
    }
    Ok(tauri::ipc::Response::new(bytes))
}

/// A window of the time-ordered album around `center_id`, for the viewer.
#[tauri::command]
async fn query_album(
    app: tauri::AppHandle,
    center_id: Option<i64>,
    radius: u32,
) -> Result<AlbumWindow, String> {
    let (geo, _) = app.state::<AppState>().geo()?;
    Ok(geo.album_window(center_id, radius.min(5_000)))
}

/// Agent test mode: set by launching with the `FOOTOS_AGENT_TEST` environment
/// variable (see `scripts/agent-test`). Returns its value, or `None` in normal runs.
#[tauri::command]
fn agent_test_enabled() -> Option<String> {
    std::env::var("FOOTOS_AGENT_TEST").ok().filter(|v| !v.is_empty())
}

/// Write the in-app self-test report next to the database so the test runner
/// can read it from the app container. Only works in agent test mode.
#[tauri::command]
fn agent_test_report(state: tauri::State<'_, AppState>, json: String) -> Result<String, String> {
    if agent_test_enabled().is_none() {
        return Err("not in agent test mode".to_string());
    }
    let db_path = state.get_db_path()?;
    let out = std::path::Path::new(&db_path)
        .parent()
        .map(|p| p.join("agent-report.json"))
        .ok_or_else(|| "no data dir".to_string())?;
    std::fs::write(&out, json).map_err(|e| e.to_string())?;
    Ok(out.to_string_lossy().into_owned())
}

/// Total rows in the library (geotagged or not).
#[tauri::command]
async fn photo_count(app: tauri::AppHandle) -> Result<i64, String> {
    app.state::<AppState>().get_index()?.count().map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_ios_known_ids(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let ids = app
            .state::<AppState>()
            .get_index()?
            .list_ios_identifiers()
            .map_err(|e| e.to_string())?;
        eprintln!(
            "[footos] list_ios_known_ids {} ids in {:?}",
            ids.len(),
            started.elapsed()
        );
        Ok(ids)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn list_photos(app: tauri::AppHandle) -> Result<Vec<PhotoInfo>, String> {
    tracing::trace!("list_photos: called");
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let index = state.get_index()?;
        let thumb_dir = state.get_thumb_dir()?;

        let mut photos = index.list().map_err(|e| {
            tracing::error!(error = %e, "list_photos: query failed");
            e.to_string()
        })?;

        let thumbnailer = footos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
        let spec = footos_core::ThumbnailSpec::default();
        let mut backfill_count = 0u32;
        let use_txn = index.begin_transaction().is_ok();
        for photo in &mut photos {
            if photo.thumb_path.is_some() && photo.file_size > 0 {
                continue;
            }
            // Virtual library URIs have no local file to stat.
            if photo.path.starts_with("ios-photo://") || photo.path.starts_with("content://") {
                continue;
            }

            let source_path = std::path::Path::new(&photo.path);
            if photo.thumb_path.is_none() {
                if let Ok(Some(path)) = thumbnailer.get_cached_path(source_path, &spec) {
                    photo.thumb_path = Some(path.to_string_lossy().to_string());
                }
            }
            if photo.file_size == 0 {
                if let Ok(metadata) = std::fs::metadata(source_path) {
                    photo.file_size = metadata.len();
                }
            }
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
        eprintln!(
            "[footos] list_photos {} rows backfill={backfill_count}",
            photos.len()
        );
        Ok(photos)
    })
    .await
    .map_err(|e| e.to_string())?
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
    media_type: MediaType,
    video_path: Option<String>,
}

fn strip_file_uri(path: &str) -> String {
    let mut root_path = path.to_string();
    loop {
        if root_path.starts_with("file://") {
            match urlencoding::decode(&root_path[7..]) {
                Ok(decoded) => root_path = decoded.into_owned(),
                Err(_) => break,
            }
        } else {
            break;
        }
    }
    root_path
}

fn photos_storage_dir(thumb_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(thumb_dir)
        .parent()
        .map(|p| p.join("Photos"))
        .unwrap_or_else(|| std::path::PathBuf::from(thumb_dir).join("Photos"))
}

/// Move a temp export into permanent Photos/ storage (copy if rename crosses volumes).
fn persist_import_file(
    src: &std::path::Path,
    photos_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    if !src.exists() {
        return Err(format!("File does not exist: {}", src.display()));
    }
    let filename = src.file_name().map(|n| n.to_os_string()).ok_or_else(|| {
        format!("Missing filename: {}", src.display())
    })?;
    let dest = photos_dir.join(filename);
    if src == dest {
        return Ok(dest);
    }
    if dest.exists() {
        return Ok(dest);
    }
    match std::fs::rename(src, &dest) {
        Ok(()) => Ok(dest),
        Err(_) => {
            std::fs::copy(src, &dest).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(src);
            Ok(dest)
        }
    }
}

#[tauri::command]
async fn import_photos(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    root_path: String,
) -> Result<ImportResultWithTiming, String> {
    let import_start = std::time::Instant::now();
    tracing::info!(path = %root_path, "Import: starting");

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
            tracing::debug!(from = %root_path, to = %decoded, "Import: decoded file URI");
            root_path = decoded;
        } else {
            break;
        }
    }
    tracing::debug!(path = %root_path, "Import: resolved path");

    // Reset cancellation flag at start
    IMPORT_CANCELLED.store(false, Ordering::SeqCst);

    let config = PhotoCoreConfig {
        thumbnail_dir: thumb_dir.clone(),
        thumbnail_size: DEFAULT_THUMBNAIL_SIZE,
    };

    let root_path_buf = std::path::Path::new(&root_path);

    // Check if path exists
    tracing::debug!(
        exists = root_path_buf.exists(),
        is_file = root_path_buf.is_file(),
        is_dir = root_path_buf.is_dir(),
        "Import: path check"
    );

    // Quick check: if file doesn't exist, return early with error
    if !root_path_buf.exists() {
        tracing::error!(path = %root_path, "Import: path does not exist");
        return Err(format!("File does not exist: {}", root_path));
    }

    // Create permanent storage directory for imported photos
    let photos_dir = std::path::Path::new(&config.thumbnail_dir).parent()
        .map(|p| p.join("Photos"))
        .unwrap_or_else(|| std::path::PathBuf::from(&config.thumbnail_dir).join("Photos"));
    std::fs::create_dir_all(&photos_dir).map_err(|e| {
        tracing::error!(error = %e, "Import: failed to create photos dir");
        e.to_string()
    })?;
    tracing::debug!(dir = ?photos_dir, "Import: photos storage dir ready");

    // Support both single file and directory import
    let photos = if root_path_buf.is_file() {
        tracing::debug!("Import: processing as single file");
        // Verify file is readable
        if let Err(e) = std::fs::metadata(&root_path_buf) {
            tracing::error!(error = %e, "Import: cannot read file metadata");
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
            tracing::debug!(dest = ?dest_path, "Import: copying to permanent storage");
            std::fs::copy(&root_path_buf, &dest_path).map_err(|e| {
                tracing::error!(error = %e, "Import: failed to copy file");
                e.to_string()
            })?;
            vec![dest_path]
        } else {
            vec![root_path_buf.to_path_buf()]
        }
    } else {
        tracing::debug!("Import: processing as directory");
        footos_core::scan_photos(root_path_buf).map_err(|e| {
            tracing::error!(error = %e, "Import: scan failed");
            e.to_string()
        })?
    };
    run_import_pipeline(&window, index, thumb_dir, photos, import_start)
}

fn run_import_pipeline(
    window: &tauri::Window,
    index: std::sync::Arc<PhotoIndex>,
    thumb_dir: String,
    photos: Vec<std::path::PathBuf>,
    import_start: std::time::Instant,
) -> Result<ImportResultWithTiming, String> {
    let config = PhotoCoreConfig {
        thumbnail_dir: thumb_dir.clone(),
        thumbnail_size: DEFAULT_THUMBNAIL_SIZE,
    };
    let total = photos.len();
    tracing::info!(count = total, "Import: found photos to process");

    let existing_paths: HashSet<String> = index
        .list_previews()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.path)
        .collect();

    let mut result = ImportResult::default();

    // If no photos found, return early
    if total == 0 {
        tracing::info!("Import: no photos to import");
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
        if existing_paths.contains(&path_str) {
            skipped += 1;
        } else {
            new_photos.push(path.clone());
        }
    }
    let new_count = new_photos.len();
    let phase1_ms = phase1_start.elapsed().as_millis();
    tracing::info!(
        duration_ms = %phase1_ms,
        new = new_count,
        skipped = skipped,
        "Import: Phase 1 (filter) complete"
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

    // ── Live Photo pairing detection ──
    let live_pairs = footos_core::detect_live_photo_pairs(&new_photos);
    let live_video_set: std::collections::HashSet<std::path::PathBuf> = live_pairs
        .iter()
        .map(|p| p.video_path.clone())
        .collect();
    let live_photo_map: std::collections::HashMap<std::path::PathBuf, std::path::PathBuf> =
        live_pairs
            .iter()
            .map(|p| (p.image_path.clone(), p.video_path.clone()))
            .collect();

    tracing::info!(
        pairs = live_pairs.len(),
        "Import: Live Photo pairs detected"
    );

    // Remove Live Photo videos from processing list (they don't get indexed separately)
    new_photos.retain(|p| !live_video_set.contains(p));
    let new_count = new_photos.len();

    // ── Phase 2 (Parallel): CPU+IO heavy work via rayon ──
    // compute_hash (~50-200ms) + read_metadata (~10-50ms) +
    // generate_thumbnail (~100-500ms) + fs::metadata (<1ms)
    // All pure functions from footos_core — no DB access, no shared mutable state.
    let phase2_start = std::time::Instant::now();
    let processed_counter = AtomicUsize::new(0);
    let window_clone = window.clone();

    let thumb_dir_clone = thumb_dir.clone();
    let processed: Vec<ProcessResult> = new_photos
        .par_iter()
        .map(|path| {
            // Check cancellation early
            if IMPORT_CANCELLED.load(Ordering::Relaxed) {
                return ProcessResult::Cancelled;
            }

            let path_str = path.to_string_lossy().to_string();
            let is_video = footos_core::is_supported_video(path);

            // Compute hash (video uses content hash, photo uses perceptual hash)
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

            // Check cancellation between expensive steps
            if IMPORT_CANCELLED.load(Ordering::Relaxed) {
                return ProcessResult::Cancelled;
            }

            // Read metadata (video uses mp4 parser + ffprobe, photo uses EXIF)
            let metadata = if is_video {
                let mut meta = footos_core::read_video_metadata(path).unwrap_or_default();
                // Always call ffprobe for videos:
                // - Core mp4 crate doesn't extract GPS at all
                // - Non-MP4 formats (MKV, AVI, WebM) need ffprobe for everything
                if let Some(probe) = ffprobe_metadata(&path_str) {
                    if meta.duration_ms.is_none() {
                        meta.duration_ms = probe.duration_ms;
                    }
                    if meta.width == 0 && meta.height == 0 {
                        meta.width = probe.width;
                        meta.height = probe.height;
                    }
                    if meta.video_codec.is_none() {
                        meta.video_codec = probe.video_codec;
                    }
                    if meta.audio_codec.is_none() {
                        meta.audio_codec = probe.audio_codec;
                    }
                    if meta.date_taken.is_none() {
                        meta.date_taken = probe.date_taken;
                    }
                    if meta.lat.is_none() {
                        meta.lat = probe.lat;
                    }
                    if meta.lon.is_none() {
                        meta.lon = probe.lon;
                    }
                }
                meta
            } else {
                match footos_core::read_metadata(path) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::error!(path = %path_str, error = %e, "Import: metadata read failed");
                        return ProcessResult::Failed(path_str);
                    }
                }
            };

            // Generate thumbnail (video uses FFmpeg CLI, photo uses image decode)
            let thumb_path_str = if is_video {
                extract_video_thumbnail_sync(path, &thumb_dir_clone)
            } else {
                footos_core::generate_thumbnail(path, &config)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            };

            // Get file size
            let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            let media_type = if is_video { MediaType::Video } else { MediaType::Photo };

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
                media_type,
                video_path: None,
            })
        })
        .collect();

    let phase2_ms = phase2_start.elapsed().as_millis();
    tracing::info!(
        duration_ms = %phase2_ms,
        count = processed.len(),
        "Import: Phase 2 (parallel processing) complete"
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
                    tracing::warn!(path = %photo.path_str, "Import: duplicate by hash, skipping");
                    // Clean up copied file for single-file imports (iOS)
                    let _ = std::fs::remove_file(&photo.path);
                    result.duplicates += 1;
                } else {
                    // Check if this is a Live Photo image (has a paired video)
                    let (final_media_type, final_video_path) =
                        if let Some(vp) = live_photo_map.get(&photo.path) {
                            (
                                MediaType::LivePhoto,
                                Some(vp.to_string_lossy().to_string()),
                            )
                        } else {
                            (photo.media_type, photo.video_path.clone())
                        };

                    // Insert into DB with media type
                    match index.insert_media(
                        photo.path_str.clone(),
                        photo.hash,
                        photo.metadata,
                        final_media_type,
                        final_video_path,
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
            }
        }

        // Intermediate commit every BATCH_SIZE writes for crash safety
        if use_txn && batch_writes >= BATCH_SIZE {
            let _ = index.commit_transaction();
            let _ = index.begin_transaction();
            batch_writes = 0;
        }

        // Phase 3 inserts are fast; emit every 20 to cut IPC overhead
        if i == 0 || (i + 1) % 20 == 0 {
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
    tracing::info!(duration_ms = phase3_ms, "Import: Phase 3 (DB insert) complete");
    tracing::info!(
        total_ms,
        phase1_ms,
        phase2_ms,
        phase3_ms,
        "Import: all phases complete"
    );
    tracing::info!(
        success = result.success,
        failure = result.failure,
        duplicates = result.duplicates,
        skipped = skipped,
        "Import: results"
    );
    {
        use tauri::Emitter;
        let _ = window.emit(
            "import-progress",
            serde_json::json!({
                "current": total,
                "total": total,
                "success": result.success,
                "failure": result.failure,
                "duplicates": result.duplicates,
                "skipped": skipped,
                "phase": "importing"
            }),
        );
    }
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

/// Import an explicit list of files (iOS PhotoKit exports).
/// Persists each file into Photos/ then runs the same 3-phase pipeline as folder import.
#[tauri::command]
async fn import_photo_paths(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
) -> Result<ImportResultWithTiming, String> {
    if paths.is_empty() {
        return Ok(ImportResultWithTiming::default());
    }

    let thumb_dir = state.get_thumb_dir()?;
    let photos_dir = photos_storage_dir(&thumb_dir);
    std::fs::create_dir_all(&photos_dir).map_err(|e| e.to_string())?;

    let mut photos = Vec::with_capacity(paths.len());
    for path in &paths {
        let src = std::path::PathBuf::from(strip_file_uri(path));
        match persist_import_file(&src, &photos_dir) {
            Ok(dest) => photos.push(dest),
            Err(e) => tracing::warn!(path = %src.display(), error = %e, "Import: skip unreadable path"),
        }
    }
    if photos.is_empty() {
        return Ok(ImportResultWithTiming {
            failure: paths.len() as u32,
            total: paths.len() as u32,
            ..Default::default()
        });
    }

    let index = state.get_index()?;
    IMPORT_CANCELLED.store(false, Ordering::SeqCst);
    tracing::info!(
        received = paths.len(),
        persisted = photos.len(),
        "Import: batch paths persisted"
    );
    run_import_pipeline(&window, index, thumb_dir, photos, std::time::Instant::now())
}

#[derive(serde::Deserialize)]
struct IosPreviewItem {
    path: String,
    thumb_path: Option<String>,
    date_taken: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    width: u32,
    height: u32,
    file_size: u64,
    media_type: Option<String>,
    duration_ms: Option<u64>,
}

/// Insert PhotoKit preview rows (no original file, no perceptual hash).
#[tauri::command]
fn upsert_ios_previews(
    state: tauri::State<'_, AppState>,
    items: Vec<IosPreviewItem>,
) -> Result<ImportResultWithTiming, String> {
    if items.is_empty() {
        return Ok(ImportResultWithTiming::default());
    }
    let index = state.get_index()?;
    if index.begin_transaction().is_err() {
        let _ = index.flush();
        index.begin_transaction().map_err(|e| e.to_string())?;
    }
    let mut result = ImportResult::default();
    for item in items {
        let media_type = MediaType::from_str(item.media_type.as_deref().unwrap_or("photo"));
        let metadata = footos_core::PhotoMetadata {
            date_taken: item.date_taken,
            width: item.width,
            height: item.height,
            lat: item.lat,
            lon: item.lon,
            duration_ms: item.duration_ms,
            orientation: 1,
            ..Default::default()
        };
        let hash = format!("ios-preview:{}", item.path);
        match index.insert_preview(
            item.path,
            hash,
            metadata,
            media_type,
            item.thumb_path,
            item.file_size,
        ) {
            Ok(true) => result.success += 1,
            Ok(false) => result.duplicates += 1,
            Err(e) => {
                eprintln!("[footos] upsert insert failed: {e}");
                result.failure += 1;
            }
        }
    }
    if let Err(e) = index.commit_transaction() {
        eprintln!("[footos] upsert commit failed: {e}");
        return Err(e.to_string());
    }
    eprintln!(
        "[footos] upsert batch ok={} fail={} dup={}",
        result.success, result.failure, result.duplicates
    );
    Ok(ImportResultWithTiming {
        success: result.success,
        failure: result.failure,
        duplicates: result.duplicates,
        total: result.success + result.failure + result.duplicates,
        ..Default::default()
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
        let is_video = photo.media_type == MediaType::Video || photo.media_type == MediaType::LivePhoto
            || footos_core::is_supported_video(&path);

        // Video: use FFmpeg; Photo: use image decoder
        let thumb_result: Option<String> = if is_video {
            extract_video_thumbnail_sync(&path, &config.thumbnail_dir)
        } else {
            footos_core::generate_thumbnail(&path, &config)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        };

        match thumb_result {
            Some(thumb_str) => {
                success += 1;
                batch_writes += 1;
                let file_size = std::fs::metadata(&path)
                    .map(|m| m.len())
                    .unwrap_or(photo.file_size);
                let _ = index.update_thumb_info(
                    photo.path.clone(),
                    Some(thumb_str),
                    file_size,
                );
            }
            None => failure += 1,
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

/// Check if FFmpeg is available on the system
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

/// Extract a video thumbnail using FFmpeg CLI.
/// Returns the cached thumbnail path, or error if FFmpeg is unavailable.
#[tauri::command]
async fn extract_video_thumbnail(
    state: tauri::State<'_, AppState>,
    video_path: String,
) -> Result<String, String> {
    let thumb_dir = state.get_thumb_dir()?;
    let source = std::path::Path::new(&video_path);

    let spec = footos_core::ThumbnailSpec::default();
    let key = footos_core::image::thumbnail::thumbnail_key(source, &spec)
        .map_err(|e| e.to_string())?;
    let dest = footos_core::image::thumbnail::cache_path(
        std::path::Path::new(&thumb_dir),
        &key,
    );

    // Return cached if exists
    if dest.exists() {
        return Ok(dest.to_string_lossy().to_string());
    }

    // Ensure parent directory exists
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let dest_str = dest.to_str().ok_or("Invalid thumbnail path")?;
    let scale_filter = format!(
        "scale={}:{}:force_original_aspect_ratio=decrease",
        spec.width, spec.height
    );

    let output = std::process::Command::new("ffmpeg")
        .args([
            "-ss", "1",
            "-i", &video_path,
            "-vframes", "1",
            "-vf", &scale_filter,
            "-q:v", "5",
            "-y",
            dest_str,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("FFmpeg execution failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_file(&dest);
        return Err(format!("FFmpeg failed: {}", stderr));
    }

    if !dest.exists() || std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0) == 0 {
        let _ = std::fs::remove_file(&dest);
        return Err("FFmpeg produced empty output".to_string());
    }

    Ok(dest.to_string_lossy().to_string())
}

/// Open a file with the system's default application (e.g., video player)
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
        return Err("Use native iOS API to open videos".to_string());
    }
    #[cfg(target_os = "android")]
    {
        return Err("Use native Android API to open videos".to_string());
    }
    Ok(())
}

/// Use ffprobe to read video metadata for non-MP4 formats.
/// Fallback for formats that the Core mp4 crate cannot parse.
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

/// Metadata extracted from ffprobe (platform fallback for non-MP4 formats)
struct FfprobeMetadata {
    duration_ms: Option<u64>,
    width: u32,
    height: u32,
    video_codec: Option<String>,
    audio_codec: Option<String>,
    date_taken: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
}

/// Parse ISO 6709 location string into (lat, lon).
/// Formats: "+35.6762+139.6503/" or "+35.6762+139.6503+026.000/"
fn parse_iso6709(s: &str) -> Option<(Option<f64>, Option<f64>)> {
    // Strip trailing "/" or other suffixes
    let s = s.trim_end_matches('/');
    if s.is_empty() {
        return None;
    }

    // Find the second +/- sign (start of longitude)
    // First char is always +/- for latitude
    let lon_start = s[1..].find(|c: char| c == '+' || c == '-').map(|i| i + 1)?;

    // Longitude ends at the next +/- (altitude) or end of string
    let lon_end = s[lon_start + 1..]
        .find(|c: char| c == '+' || c == '-')
        .map(|i| lon_start + 1 + i)
        .unwrap_or(s.len());

    let lat: f64 = s[..lon_start].parse().ok()?;
    let lon: f64 = s[lon_start..lon_end].parse().ok()?;

    // Sanity check
    if lat.abs() > 90.0 || lon.abs() > 180.0 {
        return None;
    }

    Some((Some(lat), Some(lon)))
}

/// Extract video metadata using ffprobe CLI.
/// Used as fallback when Core's mp4 parser returns empty results.
fn ffprobe_metadata(path: &str) -> Option<FfprobeMetadata> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v", "quiet",
            "-print_format", "json",
            "-show_format",
            "-show_streams",
            path,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;

    // Duration from format section (seconds as string)
    let duration_ms = json["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|secs| (secs * 1000.0) as u64);

    // Find video stream for dimensions and codec
    let streams = json["streams"].as_array();
    let video_stream = streams.and_then(|arr| {
        arr.iter().find(|s| s["codec_type"].as_str() == Some("video"))
    });
    let audio_stream = streams.and_then(|arr| {
        arr.iter().find(|s| s["codec_type"].as_str() == Some("audio"))
    });

    let width = video_stream
        .and_then(|s| s["width"].as_u64())
        .unwrap_or(0) as u32;
    let height = video_stream
        .and_then(|s| s["height"].as_u64())
        .unwrap_or(0) as u32;
    let video_codec = video_stream
        .and_then(|s| s["codec_name"].as_str())
        .map(|s| s.to_string());
    let audio_codec = audio_stream
        .and_then(|s| s["codec_name"].as_str())
        .map(|s| s.to_string());

    // Creation time from format tags
    let tags = &json["format"]["tags"];
    let date_taken = tags["creation_time"]
        .as_str()
        .and_then(|s| {
            // ffprobe returns ISO 8601: "2024-01-15T09:30:00.000000Z"
            // Convert to EXIF format: "2024-01-15 09:30:00"
            s.get(..19).map(|t| t.replace('T', " "))
        });

    // GPS from format tags
    // Apple: com.apple.quicktime.location.ISO6709 = "+35.6762+139.6503+026.000/"
    // Android: location = "+35.6762+139.6503/"
    let (lat, lon) = tags["com.apple.quicktime.location.ISO6709"]
        .as_str()
        .or_else(|| tags["location"].as_str())
        .and_then(parse_iso6709)
        .unwrap_or((None, None));

    Some(FfprobeMetadata {
        duration_ms,
        width,
        height,
        video_codec,
        audio_codec,
        date_taken,
        lat,
        lon,
    })
}

/// FFmpeg video thumbnail extraction (sync version, for rayon parallel processing)
fn extract_video_thumbnail_sync(
    video_path: &std::path::Path,
    thumb_dir: &str,
) -> Option<String> {
    let spec = footos_core::ThumbnailSpec::default();
    let key = footos_core::image::thumbnail::thumbnail_key(video_path, &spec).ok()?;
    let dest = footos_core::image::thumbnail::cache_path(
        std::path::Path::new(thumb_dir),
        &key,
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "footos=info".into()),
        )
        .init();

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
            import_photo_paths,
            upsert_ios_previews,
            flush_photos_db,
            cancel_import,
            list_photos,
            list_photos_preview,
            list_map_previews,
            query_map,
            geo_summary,
            query_timeline,
            query_album,
            photo_count,
            agent_test_enabled,
            agent_test_report,
            list_ios_known_ids,
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
            import_all_ios_photos,
            extract_video_thumbnail,
            open_in_system_player,
            check_ffmpeg_available,
            read_video_metadata_ffprobe,
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
