use footos_core::{AlbumWindow, Bbox, GeoIndex, LibraryItem, LibrarySort, LibraryView, MapFeature, PhotoCoreConfig, PhotoIndex, ImportResult, PhotoInfo, PhotoPreview, DEFAULT_THUMBNAIL_SIZE, MediaType};
use tauri::Manager;
use rayon::prelude::*;
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
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
    geo: Mutex<Option<((u64, i64, u8), Arc<GeoIndex>)>>,
    /// Media type the map, timeline and album are limited to (mobile filter).
    media_filter: Mutex<Option<MediaType>>,
    /// Cached library list (desktop drawer), same key as `geo`.
    library: Mutex<Option<((u64, i64), Arc<LibraryView>)>>,
    /// The order last asked for, so paging through it does not re-sort.
    library_order: Mutex<Option<(LibraryOrderKey, Arc<Vec<u32>>)>>,
    generation: std::sync::atomic::AtomicU64,
}

type LibraryOrderKey = ((u64, i64), LibrarySort, bool, Option<MediaType>);

impl AppState {
    fn new() -> Self {
        Self {
            index: Mutex::new(None),
            db_path: Mutex::new(String::new()),
            thumb_dir: Mutex::new(String::new()),
            geo: Mutex::new(None),
            media_filter: Mutex::new(None),
            library: Mutex::new(None),
            library_order: Mutex::new(None),
            generation: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// The in-memory query index, rebuilt lazily whenever the DB has changed.
    /// Returns the index and a version token the frontend can compare.
    fn geo(&self) -> Result<(Arc<GeoIndex>, String), String> {
        let index = self.get_index()?;
        let filter = *self.media_filter.lock().map_err(|e| e.to_string())?;
        let filter_code = match filter {
            None => 0u8,
            Some(MediaType::Photo) => 1,
            Some(MediaType::Video) => 2,
            Some(MediaType::LivePhoto) => 3,
        };
        let key = (
            self.generation.load(Ordering::SeqCst),
            index.change_counter().map_err(|e| e.to_string())?,
            filter_code,
        );
        let version = format!("{}-{}-{}", key.0, key.1, key.2);
        let mut cached = self.geo.lock().map_err(|e| e.to_string())?;
        if let Some((k, geo)) = cached.as_ref() {
            if *k == key {
                return Ok((geo.clone(), version));
            }
        }
        let started = std::time::Instant::now();
        let thumb_dir = self.get_thumb_dir()?;
        let mut rows = index.list_previews_filtered(true).map_err(|e| e.to_string())?;
        if let Some(kind) = filter {
            rows.retain(|row| row.media_type == kind.as_str());
        }
        for row in &mut rows {
            row.thumb_path = Some(resolve_thumb(&thumb_dir, &row.path, row.thumb_path.take()));
        }
        let geo = Arc::new(GeoIndex::build(rows));
        tracing::info!(rows = geo.len(), ms = started.elapsed().as_millis() as u64, "geo index rebuilt");
        *cached = Some((key, geo.clone()));
        Ok((geo, version))
    }

    /// The grouped library list, rebuilt lazily whenever the DB has changed.
    fn library(&self) -> Result<((u64, i64), Arc<LibraryView>), String> {
        let index = self.get_index()?;
        let generation = self.generation.load(Ordering::SeqCst);
        let key = (generation, index.change_counter().map_err(|e| e.to_string())?);
        let mut cached = self.library.lock().map_err(|e| e.to_string())?;
        if let Some((k, view)) = cached.as_ref() {
            if *k == key {
                return Ok((key, view.clone()));
            }
        }
        let started = std::time::Instant::now();
        let photos = load_photos_backfilled(&index, &self.get_thumb_dir()?)?;
        let view = Arc::new(LibraryView::build(photos));
        // Backfilling writes to the DB, so read the counter again: the cache
        // key must describe the state the view was built from.
        let key = (generation, index.change_counter().map_err(|e| e.to_string())?);
        tracing::info!(rows = view.files(), ms = started.elapsed().as_millis() as u64, "library view rebuilt");
        *cached = Some((key, view.clone()));
        Ok((key, view))
    }

    /// Display order for the library list; the last one asked for is cached.
    fn library_order(
        &self,
        sort: LibrarySort,
        descending: bool,
        filter: Option<MediaType>,
    ) -> Result<(Arc<LibraryView>, Arc<Vec<u32>>), String> {
        let (view_key, view) = self.library()?;
        let key: LibraryOrderKey = (view_key, sort, descending, filter);
        let mut cached = self.library_order.lock().map_err(|e| e.to_string())?;
        if let Some((k, order)) = cached.as_ref() {
            if *k == key {
                return Ok((view, order.clone()));
            }
        }
        let order = Arc::new(view.order(sort, descending, filter));
        *cached = Some((key, order.clone()));
        Ok((view, order))
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
        *self.library.lock().map_err(|e| e.to_string())? = None;
        *self.library_order.lock().map_err(|e| e.to_string())? = None;
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

/// Limit the map, timeline and album to one media type (`null` = everything).
#[tauri::command]
fn set_media_filter(state: tauri::State<'_, AppState>, filter: Option<MediaType>) -> Result<(), String> {
    *state.media_filter.lock().map_err(|e| e.to_string())? = filter;
    Ok(())
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

/// The time-ordered album of what the map shows: inside `bbox` and the time filter.
#[tauri::command]
async fn query_album_in(
    app: tauri::AppHandle,
    bbox: Bbox,
    start_ms: Option<f64>,
    end_ms: Option<f64>,
    limit: u32,
) -> Result<AlbumWindow, String> {
    let (geo, _) = app.state::<AppState>().geo()?;
    Ok(geo.album_in(bbox, start_ms.map(|v| v as i64), end_ms.map(|v| v as i64), limit.min(5_000)))
}

/// Agent test mode: set by launching with the `FOOTOS_AGENT_TEST` environment
/// variable (see `scripts/agent-test`). Returns its value, or `None` in normal runs.
#[tauri::command]
fn agent_test_enabled() -> Option<String> {
    std::env::var("FOOTOS_AGENT_TEST").ok().filter(|v| !v.is_empty())
}

/// A folder of real image files for the self-test to import (see
/// `scripts/agent-test/make-import-fixture.py`). Agent test mode only.
#[tauri::command]
fn agent_test_import_dir() -> Option<String> {
    agent_test_enabled()?;
    std::env::var("FOOTOS_AGENT_IMPORT_DIR").ok().filter(|v| !v.is_empty())
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

/// Every row, with missing thumbnail paths and file sizes filled in (and
/// written back so the next read finds them).
fn load_photos_backfilled(index: &PhotoIndex, thumb_dir: &str) -> Result<Vec<PhotoInfo>, String> {
    let mut photos = index.list().map_err(|e| {
        tracing::error!(error = %e, "list photos: query failed");
        e.to_string()
    })?;

    let thumbnailer = footos_core::Thumbnailer::new(std::path::PathBuf::from(thumb_dir));
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
        let mut changed = false;
        if photo.thumb_path.is_none() {
            if let Ok(Some(path)) = thumbnailer.get_cached_path(source_path, &spec) {
                photo.thumb_path = Some(path.to_string_lossy().to_string());
                changed = true;
            }
        }
        if photo.file_size == 0 {
            if let Ok(metadata) = std::fs::metadata(source_path) {
                photo.file_size = metadata.len();
                changed = true;
            }
        }
        if changed {
            let _ = index.update_thumb_info(
                photo.path.clone(),
                photo.thumb_path.clone(),
                photo.file_size,
            );
            backfill_count += 1;
        }
    }
    if use_txn {
        let _ = index.commit_transaction();
    }
    eprintln!("[footos] list photos {} rows backfill={backfill_count}", photos.len());
    Ok(photos)
}

#[tauri::command]
async fn list_photos(app: tauri::AppHandle) -> Result<Vec<PhotoInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let index = state.get_index()?;
        load_photos_backfilled(&index, &state.get_thumb_dir()?)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Serialize)]
struct LibraryPage {
    /// Entries matching the filter.
    total: u32,
    /// Entries after RAW+JPEG grouping, before the filter.
    grouped_total: u32,
    /// Rows in the database.
    files_total: u32,
    offset: u32,
    items: Vec<LibraryItem>,
}

/// One page of the library list, grouped, filtered and sorted in Rust.
#[tauri::command]
async fn query_library(
    app: tauri::AppHandle,
    sort_by: LibrarySort,
    descending: bool,
    media_filter: Option<MediaType>,
    offset: u32,
    limit: u32,
) -> Result<LibraryPage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (view, order) = state.library_order(sort_by, descending, media_filter)?;
        let from = (offset as usize).min(order.len());
        let to = (from + limit.min(1_000) as usize).min(order.len());
        Ok(LibraryPage {
            total: order.len() as u32,
            grouped_total: view.len() as u32,
            files_total: view.files() as u32,
            offset: from as u32,
            items: order[from..to].iter().filter_map(|&i| view.get(i).cloned()).collect(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Full details of one item, as the library list would show it (a paired RAW
/// row resolves to its JPEG).
#[tauri::command]
async fn photo_detail(app: tauri::AppHandle, id: i64) -> Result<Option<LibraryItem>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (_, view) = app.state::<AppState>().library()?;
        Ok(view.by_id(id).cloned())
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
    /// Rows rewritten because the file changed on disk (folder rescans).
    updated: u32,
    /// Rows that followed a moved or renamed file.
    moved: u32,
    /// Rows dropped because the file is gone (folder rescans).
    removed: u32,
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
    mtime_ms: i64,
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

        // Mobile hands over temporary files that disappear, so they are copied
        // into the app's storage. On desktop the file stays where the user
        // keeps it and is referenced in place, like files found in a folder.
        let copy_into_app = cfg!(any(target_os = "ios", target_os = "android"));
        if copy_into_app && root_path_buf != dest_path {
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
        // Remember the folder: later scans pick up what changes in it.
        let root = std::fs::canonicalize(root_path_buf)
            .unwrap_or_else(|_| root_path_buf.to_path_buf());
        let _ = index.add_root(&root.to_string_lossy(), now_ms());
        footos_core::scan_photos(&root).map_err(|e| {
            tracing::error!(error = %e, "Import: scan failed");
            e.to_string()
        })?
    };
    run_import_pipeline(&window, index, thumb_dir, photos, import_start)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Size and modification time (ms since the epoch) of a file; zeros if unreadable.
fn file_stat(path: &std::path::Path) -> (u64, i64) {
    match std::fs::metadata(path) {
        Ok(m) => {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            (m.len(), mtime)
        }
        Err(_) => (0, 0),
    }
}

/// Files are processed and committed in batches of this size, so photos show
/// up while a large import is still running and a cancel or crash costs one
/// batch at most.
const IMPORT_BATCH: usize = 256;

type Emit<'a> = &'a (dyn Fn(&str, serde_json::Value) + Sync);

fn run_import_pipeline(
    window: &tauri::Window,
    index: std::sync::Arc<PhotoIndex>,
    thumb_dir: String,
    photos: Vec<std::path::PathBuf>,
    import_start: std::time::Instant,
) -> Result<ImportResultWithTiming, String> {
    use tauri::Emitter;
    let emit = |event: &str, payload: serde_json::Value| {
        let _ = window.emit(event, payload);
    };
    import_files(&emit, &index, &thumb_dir, photos, false, import_start)
}

/// Read metadata and make the thumbnail of one file (the expensive part).
fn process_file(
    path: &std::path::Path,
    hash: String,
    config: &PhotoCoreConfig,
    thumb_dir: &str,
) -> ProcessResult {
    let path_str = path.to_string_lossy().to_string();
    let is_video = footos_core::is_supported_video(path);

    // Video: mp4 parser, then ffprobe for what it cannot read (GPS, other
    // containers). Photo: EXIF.
    let metadata = if is_video {
        let mut meta = footos_core::read_video_metadata(path).unwrap_or_default();
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

    let thumb_path_str = if is_video {
        extract_video_thumbnail_sync(path, thumb_dir)
    } else {
        footos_core::generate_thumbnail(path, config)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    };

    let (file_size, mtime_ms) = file_stat(path);
    ProcessResult::New(ProcessedPhoto {
        path: path.to_path_buf(),
        path_str,
        hash,
        metadata,
        thumb_path_str,
        file_size,
        mtime_ms,
        media_type: if is_video { MediaType::Video } else { MediaType::Photo },
        video_path: None,
    })
}

/// Import files into the index, batch by batch.
///
/// Per batch: hash in parallel, decide what is a duplicate, read metadata and
/// make thumbnails in parallel for the rest, write them in one transaction,
/// report progress.
///
/// A file whose content is already in the library under another path is a
/// duplicate: it is counted and left alone on disk. (If that other path no
/// longer exists the file was moved, and its row follows it.) Different
/// photos are never treated as duplicates, however alike they look.
///
/// `update` rewrites the rows of files that are already indexed (they were
/// modified on disk) instead of adding new ones.
fn import_files(
    emit: Emit,
    index: &PhotoIndex,
    thumb_dir: &str,
    photos: Vec<std::path::PathBuf>,
    update: bool,
    import_start: std::time::Instant,
) -> Result<ImportResultWithTiming, String> {
    let config = PhotoCoreConfig {
        thumbnail_dir: thumb_dir.to_string(),
        thumbnail_size: DEFAULT_THUMBNAIL_SIZE,
    };
    let total = photos.len();
    tracing::info!(count = total, update, "Import: files to consider");
    if total == 0 {
        return Ok(ImportResultWithTiming {
            total_ms: import_start.elapsed().as_millis() as u64,
            ..Default::default()
        });
    }

    // ── Filter: what is already indexed (by path) ──
    let filter_start = std::time::Instant::now();
    let mut todo: Vec<std::path::PathBuf> = if update {
        photos
    } else {
        let known: HashSet<String> = index
            .path_stats()
            .unwrap_or_default()
            .into_iter()
            .map(|f| f.path)
            .collect();
        photos
            .into_iter()
            .filter(|p| !known.contains(p.to_string_lossy().as_ref()))
            .collect()
    };
    let skipped = total - todo.len();
    let phase1_ms = filter_start.elapsed().as_millis() as u64;

    // ── Live Photo pairs: the video belongs to its image, not to the library ──
    let live_pairs = footos_core::detect_live_photo_pairs(&todo);
    let live_videos: HashSet<std::path::PathBuf> =
        live_pairs.iter().map(|p| p.video_path.clone()).collect();
    let live_video_of: std::collections::HashMap<std::path::PathBuf, std::path::PathBuf> = live_pairs
        .iter()
        .map(|p| (p.image_path.clone(), p.video_path.clone()))
        .collect();
    todo.retain(|p| !live_videos.contains(p));

    let app_photos_dir = photos_storage_dir(thumb_dir);
    let mut result = ImportResultWithTiming {
        skipped: skipped as u32,
        total: total as u32,
        phase1_ms,
        ..Default::default()
    };
    let mut done = skipped + live_videos.len();
    let mut cancelled = false;

    let progress = |done: usize, r: &ImportResultWithTiming, last: &str| {
        emit(
            "import-progress",
            serde_json::json!({
                "current": done.min(total),
                "total": total,
                "success": r.success + r.updated + r.moved,
                "failure": r.failure,
                "duplicates": r.duplicates,
                "skipped": skipped,
                "phase": "importing",
                "last_path": last,
            }),
        );
    };
    progress(done, &result, "");

    for batch in todo.chunks(IMPORT_BATCH) {
        if IMPORT_CANCELLED.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }

        // ── Hash (parallel, cheap: 128 KB per file) ──
        let work_start = std::time::Instant::now();
        let hashed: Vec<(&std::path::PathBuf, Option<String>)> = batch
            .par_iter()
            .map(|path| (path, footos_core::compute_content_hash(path).ok()))
            .collect();

        // ── Decide what is new (sequential: needs the index and this batch) ──
        let mut in_batch: HashSet<String> = HashSet::new();
        let mut keep: Vec<(&std::path::PathBuf, String)> = Vec::with_capacity(batch.len());
        for (path, hash) in hashed {
            let Some(hash) = hash else {
                tracing::error!(path = %path.display(), "Import: cannot read file");
                result.failure += 1;
                continue;
            };
            if update {
                keep.push((path, hash));
                continue;
            }
            let existing = if in_batch.contains(&hash) {
                Some((-1, String::new()))
            } else {
                index.find_by_hash(&hash).unwrap_or(None)
            };
            match existing {
                None => {
                    in_batch.insert(hash.clone());
                    keep.push((path, hash));
                }
                Some((id, old_path)) => {
                    let old_is_file = id >= 0
                        && !old_path.contains("://")
                        && !std::path::Path::new(&old_path).exists();
                    if old_is_file {
                        // Same content, and the indexed path is gone: the file
                        // was moved or renamed. Its row follows it.
                        let (_, mtime) = file_stat(path);
                        let _ = index.move_path(id, &path.to_string_lossy(), mtime);
                        result.moved += 1;
                    } else {
                        // A second copy of a file the library already has.
                        // Only a copy the app made itself is cleaned up; the
                        // user's files are never touched.
                        if path.starts_with(&app_photos_dir) {
                            let _ = std::fs::remove_file(path);
                        }
                        result.duplicates += 1;
                    }
                }
            }
        }

        // ── Metadata + thumbnails (parallel, the expensive part) ──
        let processed: Vec<ProcessResult> = keep
            .par_iter()
            .map(|(path, hash)| {
                if IMPORT_CANCELLED.load(Ordering::Relaxed) {
                    return ProcessResult::Cancelled;
                }
                process_file(path, hash.clone(), &config, thumb_dir)
            })
            .collect();
        result.phase2_ms += work_start.elapsed().as_millis() as u64;

        // ── Write the batch in one transaction ──
        let write_start = std::time::Instant::now();
        let use_txn = index.begin_transaction().is_ok();
        let mut last_path = String::new();
        for item in processed {
            match item {
                ProcessResult::Cancelled => cancelled = true,
                ProcessResult::Failed(_) => result.failure += 1,
                ProcessResult::New(photo) => {
                    let (media_type, video_path) = match live_video_of.get(&photo.path) {
                        Some(video) => (
                            MediaType::LivePhoto,
                            Some(video.to_string_lossy().to_string()),
                        ),
                        None => (photo.media_type, photo.video_path.clone()),
                    };
                    let written = if update {
                        index.update_file(
                            &photo.path_str,
                            &photo.hash,
                            &photo.metadata,
                            media_type,
                            video_path.as_deref(),
                            photo.thumb_path_str.as_deref(),
                            photo.file_size,
                            photo.mtime_ms,
                        )
                    } else {
                        index.insert_file(
                            &photo.path_str,
                            &photo.hash,
                            &photo.metadata,
                            media_type,
                            video_path.as_deref(),
                            photo.thumb_path_str.as_deref(),
                            photo.file_size,
                            photo.mtime_ms,
                        )
                    };
                    match written {
                        Ok(true) if update => result.updated += 1,
                        Ok(true) => result.success += 1,
                        Ok(false) => result.skipped += 1,
                        Err(e) => {
                            tracing::error!(error = %e, "Import: DB write failed");
                            result.failure += 1;
                        }
                    }
                    last_path = photo.path_str;
                }
            }
        }
        if use_txn {
            let _ = index.commit_transaction();
        }
        result.phase3_ms += write_start.elapsed().as_millis() as u64;

        done += batch.len();
        progress(done, &result, &last_path);
        if cancelled {
            break;
        }
    }

    if cancelled {
        emit(
            "import-cancelled",
            serde_json::json!({
                "current": done.min(total),
                "total": total,
                "success": result.success,
                "failure": result.failure
            }),
        );
    } else {
        progress(total, &result, "");
    }

    result.total_ms = import_start.elapsed().as_millis() as u64;
    tracing::info!(
        success = result.success,
        updated = result.updated,
        moved = result.moved,
        failure = result.failure,
        duplicates = result.duplicates,
        skipped = result.skipped,
        total_ms = result.total_ms,
        "Import: done"
    );
    Ok(result)
}

#[derive(serde::Serialize, Default)]
struct RescanSummary {
    /// Folders scanned.
    roots: u32,
    /// Folders skipped because they could not be read (e.g. a drive that is
    /// not connected). Nothing of theirs is touched.
    offline: Vec<String>,
    added: u32,
    updated: u32,
    moved: u32,
    removed: u32,
    duplicates: u32,
    failure: u32,
    total_ms: u64,
}

/// Bring the index in line with one remembered folder: add what is new,
/// refresh what was modified, drop what is gone.
fn rescan_root(
    emit: Emit,
    index: &PhotoIndex,
    thumb_dir: &str,
    root: &str,
    summary: &mut RescanSummary,
) -> Result<(), String> {
    let root_path = std::path::Path::new(root);
    // An unreadable folder is not an empty folder. Leave its photos alone.
    if !root_path.is_dir() || std::fs::read_dir(root_path).is_err() {
        summary.offline.push(root.to_string());
        return Ok(());
    }

    let files = footos_core::scan_photos(root_path).map_err(|e| e.to_string())?;
    let scanned: Vec<footos_core::FileStat> = files
        .par_iter()
        .map(|path| {
            let (size, mtime_ms) = file_stat(path);
            footos_core::FileStat {
                path: path.to_string_lossy().to_string(),
                size,
                mtime_ms,
            }
        })
        .collect();
    let known = index.path_stats().map_err(|e| e.to_string())?;
    let plan = footos_core::plan_rescan(root, &scanned, &known);
    tracing::info!(
        root,
        on_disk = scanned.len(),
        new = plan.new.len(),
        changed = plan.changed.len(),
        missing = plan.missing.len(),
        backfill = plan.backfill.len(),
        "Rescan: plan"
    );

    if !plan.backfill.is_empty() {
        let use_txn = index.begin_transaction().is_ok();
        for (id, size, mtime_ms) in &plan.backfill {
            let _ = index.set_file_stat(*id, *size, *mtime_ms);
        }
        if use_txn {
            let _ = index.commit_transaction();
        }
    }

    // New files first: a file that was moved inside the folder shows up as
    // "new" at its new path, and its existing row follows it there.
    if !plan.new.is_empty() {
        let paths = plan.new.iter().map(std::path::PathBuf::from).collect();
        let r = import_files(emit, index, thumb_dir, paths, false, std::time::Instant::now())?;
        summary.added += r.success;
        summary.moved += r.moved;
        summary.duplicates += r.duplicates;
        summary.failure += r.failure;
    }
    if !plan.changed.is_empty() && !IMPORT_CANCELLED.load(Ordering::SeqCst) {
        let paths = plan.changed.iter().map(std::path::PathBuf::from).collect();
        let r = import_files(emit, index, thumb_dir, paths, true, std::time::Instant::now())?;
        summary.updated += r.updated;
        summary.failure += r.failure;
    }

    // Gone from disk. Check each row again: one that was just moved has a
    // path that exists now.
    if !plan.missing.is_empty() && !IMPORT_CANCELLED.load(Ordering::SeqCst) {
        let mut gone = Vec::new();
        for id in &plan.missing {
            if let Ok(Some(photo)) = index.get_by_id(*id) {
                if !std::path::Path::new(&photo.path).exists() {
                    gone.push(*id);
                }
            }
        }
        if !gone.is_empty() {
            let thumbnailer = footos_core::Thumbnailer::new(std::path::PathBuf::from(thumb_dir));
            let spec = footos_core::ThumbnailSpec::default();
            let deleted = index.delete_by_ids(gone).map_err(|e| e.to_string())?;
            for photo in &deleted {
                if let Ok(Some(thumb)) =
                    thumbnailer.get_cached_path(std::path::Path::new(&photo.path), &spec)
                {
                    let _ = std::fs::remove_file(thumb);
                }
            }
            summary.removed += deleted.len() as u32;
        }
    }

    let _ = index.touch_root(root, now_ms());
    summary.roots += 1;
    Ok(())
}

/// Scan every remembered folder again and bring the library up to date.
#[tauri::command]
async fn rescan_library(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<RescanSummary, String> {
    use tauri::Emitter;
    let started = std::time::Instant::now();
    let index = state.get_index()?;
    let thumb_dir = state.get_thumb_dir()?;
    IMPORT_CANCELLED.store(false, Ordering::SeqCst);

    let emit = |event: &str, payload: serde_json::Value| {
        let _ = window.emit(event, payload);
    };
    let mut summary = RescanSummary::default();
    for (root, _) in index.list_roots().map_err(|e| e.to_string())? {
        if IMPORT_CANCELLED.load(Ordering::SeqCst) {
            break;
        }
        rescan_root(&emit, &index, &thumb_dir, &root, &mut summary)?;
    }
    summary.total_ms = started.elapsed().as_millis() as u64;
    tracing::info!(
        roots = summary.roots,
        added = summary.added,
        updated = summary.updated,
        moved = summary.moved,
        removed = summary.removed,
        offline = summary.offline.len(),
        total_ms = summary.total_ms,
        "Rescan: done"
    );
    Ok(summary)
}

#[derive(serde::Serialize)]
struct LibraryRoot {
    path: String,
    /// Milliseconds since the epoch; 0 if never scanned.
    last_scan: i64,
    /// The folder can be read right now.
    available: bool,
    /// Rows of the library that live in it.
    photos: u32,
}

/// The folders the library is built from.
#[tauri::command]
async fn list_library_roots(state: tauri::State<'_, AppState>) -> Result<Vec<LibraryRoot>, String> {
    let index = state.get_index()?;
    let known = index.path_stats().map_err(|e| e.to_string())?;
    Ok(index
        .list_roots()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(path, last_scan)| LibraryRoot {
            available: std::path::Path::new(&path).is_dir(),
            photos: known
                .iter()
                .filter(|f| footos_core::is_under(&path, &f.path))
                .count() as u32,
            path,
            last_scan,
        })
        .collect())
}

/// Stop following a folder. With `remove_photos` its photos leave the library
/// too (rows and thumbnails only; files on disk are never touched).
#[tauri::command]
async fn remove_library_root(
    state: tauri::State<'_, AppState>,
    path: String,
    remove_photos: bool,
) -> Result<u32, String> {
    let index = state.get_index()?;
    let mut removed = 0u32;
    if remove_photos {
        let ids: Vec<i64> = index
            .path_stats()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|f| footos_core::is_under(&path, &f.path))
            .map(|f| f.id)
            .collect();
        let thumb_dir = state.get_thumb_dir()?;
        let thumbnailer = footos_core::Thumbnailer::new(std::path::PathBuf::from(&thumb_dir));
        let spec = footos_core::ThumbnailSpec::default();
        let deleted = index.delete_by_ids(ids).map_err(|e| e.to_string())?;
        for photo in &deleted {
            if let Ok(Some(thumb)) = thumbnailer.get_cached_path(std::path::Path::new(&photo.path), &spec) {
                let _ = std::fs::remove_file(thumb);
            }
        }
        removed = deleted.len() as u32;
    }
    index.remove_root(&path).map_err(|e| e.to_string())?;
    Ok(removed)
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

/// Drop the rows of library assets the app can no longer see, with their
/// thumbnails. Returns how many rows were removed.
#[tauri::command]
fn remove_ios_assets(state: tauri::State<'_, AppState>, ids: Vec<String>) -> Result<u32, String> {
    let index = state.get_index()?;
    let thumb_dir = state.get_thumb_dir()?;
    let mut removed = 0u32;
    for id in ids {
        let path = format!("ios-photo://{id}");
        let Some(photo) = index.get_by_path(path).map_err(|e| e.to_string())? else {
            continue;
        };
        if index.delete_by_id(photo.id.id).map_err(|e| e.to_string())?.is_some() {
            removed += 1;
            // The native side names the thumbnail after the asset identifier
            // (PhotoLibraryBridge.m, safeIdForAsset).
            let name = format!("{}.jpg", id.replace(['/', ':'], "_"));
            let _ = std::fs::remove_file(std::path::Path::new(&thumb_dir).join(name));
        }
    }
    Ok(removed)
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
        let path = item.path.clone();
        let thumb_path = item.thumb_path.clone();
        match index.insert_preview(
            item.path,
            hash,
            metadata,
            media_type,
            item.thumb_path,
            item.file_size,
        ) {
            Ok(true) => result.success += 1,
            Ok(false) => {
                // Already known. A row that was stored without a thumbnail
                // (it could not be produced at the time) gets it now.
                if thumb_path.is_some() {
                    let _ = index.update_thumb_info(path, thumb_path, item.file_size);
                }
                result.duplicates += 1;
            }
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
            rescan_library,
            list_library_roots,
            remove_library_root,
            upsert_ios_previews,
            remove_ios_assets,
            flush_photos_db,
            cancel_import,
            list_photos,
            list_photos_preview,
            list_map_previews,
            query_map,
            geo_summary,
            set_media_filter,
            query_timeline,
            query_album,
            query_album_in,
            query_library,
            photo_detail,
            photo_count,
            agent_test_enabled,
            agent_test_import_dir,
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


/// End-to-end import and rescan over real files.
///
/// Needs a fixture built by `scripts/agent-test/make-import-fixture.py`:
///
///     python3 scripts/agent-test/make-import-fixture.py /tmp/fx
///     FOOTOS_IMPORT_FIXTURE=/tmp/fx cargo test import_fixture -- --nocapture
///
/// Without the variable the test does nothing.
#[cfg(test)]
mod import_fixture {
    use super::*;

    fn count_files(dir: &std::path::Path) -> usize {
        footos_core::scan_photos(dir).map(|v| v.len()).unwrap_or(0)
    }

    #[test]
    fn imports_and_rescans_real_files() {
        let Ok(fixture) = std::env::var("FOOTOS_IMPORT_FIXTURE") else {
            eprintln!("FOOTOS_IMPORT_FIXTURE not set, skipping");
            return;
        };
        let expected: u32 = std::env::var("FOOTOS_IMPORT_EXPECTED")
            .ok()
            .and_then(|v| v.parse().ok())
            .expect("FOOTOS_IMPORT_EXPECTED");
        let base = std::fs::canonicalize(&fixture).unwrap();
        let lib = base.join("lib");
        let root = lib.to_string_lossy().to_string();
        let thumbs = base.join("thumbnails");
        std::fs::create_dir_all(&thumbs).unwrap();
        let thumb_dir = thumbs.to_string_lossy().to_string();
        let db = base.join("footos.db");
        let _ = std::fs::remove_file(&db);
        let index = PhotoIndex::open(db.to_string_lossy().to_string()).unwrap();
        let emit = |_: &str, _: serde_json::Value| {};
        let now = std::time::Instant::now;

        // ── First import ──
        let on_disk = count_files(&lib);
        let files = footos_core::scan_photos(&lib).unwrap();
        let r = import_files(&emit, &index, &thumb_dir, files.clone(), false, now()).unwrap();
        eprintln!("first import: ok={} dup={} fail={} in {} ms", r.success, r.duplicates, r.failure, r.total_ms);
        assert_eq!(r.failure, 0, "nothing should fail");
        assert_eq!(r.success, expected, "every distinct file is in the library");
        assert_eq!(r.duplicates, 1, "the byte-for-byte copy is the only duplicate");
        assert_eq!(count_files(&lib), on_disk, "no file was deleted from disk");
        assert_eq!(index.count().unwrap() as u32, expected);

        // Lookalikes and the RAW next to its JPEG are separate rows.
        for name in ["lookalike_a.jpg", "lookalike_b.jpg", "pair.jpg", "pair.dng"] {
            let path = lib.join(name).to_string_lossy().to_string();
            assert!(index.get_by_path(path).unwrap().is_some(), "{name} is in the library");
        }
        // Thumbnails were made.
        let with_thumb = index.list().unwrap().iter().filter(|p| p.thumb_path.is_some()).count();
        eprintln!("rows with thumbnail: {with_thumb}/{expected}");
        assert!(with_thumb as u32 >= expected - 1);

        // ── Importing the same folder again adds nothing ──
        let r = import_files(&emit, &index, &thumb_dir, files, false, now()).unwrap();
        assert_eq!((r.success, r.failure), (0, 0));
        assert_eq!(index.count().unwrap() as u32, expected);

        // ── Rescan with nothing changed ──
        index.add_root(&root, now_ms()).unwrap();
        let mut s = RescanSummary::default();
        rescan_root(&emit, &index, &thumb_dir, &root, &mut s).unwrap();
        assert_eq!((s.added, s.updated, s.removed, s.moved), (0, 0, 0, 0), "a second look changes nothing");

        // ── Change the folder: modify, delete, add, move ──
        let modified = lib.join("photo_01.jpg");
        let mut bytes = std::fs::read(&modified).unwrap();
        bytes.extend_from_slice(&[0u8; 4096]);
        std::fs::write(&modified, bytes).unwrap();

        std::fs::remove_file(lib.join("photo_02.jpg")).unwrap();

        let mut added = std::fs::read(lib.join("photo_03.jpg")).unwrap();
        added.extend_from_slice(b"another file");
        std::fs::write(lib.join("brand_new.jpg"), added).unwrap();

        let moved_from = lib.join("photo_04.jpg");
        let moved_to = lib.join("sub").join("renamed_04.jpg");
        let moved_id = index.get_by_path(moved_from.to_string_lossy().to_string()).unwrap().unwrap().id.id;
        std::fs::rename(&moved_from, &moved_to).unwrap();

        let mut s = RescanSummary::default();
        rescan_root(&emit, &index, &thumb_dir, &root, &mut s).unwrap();
        eprintln!("rescan: added={} updated={} moved={} removed={} dup={}", s.added, s.updated, s.moved, s.removed, s.duplicates);
        assert_eq!(s.added, 1, "the new file");
        assert_eq!(s.updated, 1, "the modified file");
        assert_eq!(s.moved, 1, "the renamed file");
        assert_eq!(s.removed, 1, "the deleted file");
        assert_eq!(index.count().unwrap() as u32, expected, "one out, one in");
        let after_move = index.get_by_path(moved_to.to_string_lossy().to_string()).unwrap().unwrap();
        assert_eq!(after_move.id.id, moved_id, "the row followed the file");
        assert!(index.get_by_path(lib.join("photo_02.jpg").to_string_lossy().to_string()).unwrap().is_none());

        // ── The folder goes away (a drive that is not connected) ──
        let parked = base.join("lib-offline");
        std::fs::rename(&lib, &parked).unwrap();
        let mut s = RescanSummary::default();
        rescan_root(&emit, &index, &thumb_dir, &root, &mut s).unwrap();
        std::fs::rename(&parked, &lib).unwrap();
        assert_eq!(s.offline, vec![root.clone()]);
        assert_eq!(s.removed, 0);
        assert_eq!(index.count().unwrap() as u32, expected, "an unreadable folder loses nothing");
    }
}
