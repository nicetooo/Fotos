use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

use crate::{
    error::CoreError,
    types::{MediaType, PhotoId, PhotoInfo, PhotoMetadata, PhotoPreview},
};

#[derive(uniffi::Object)]
pub struct PhotoIndex {
    conn: Mutex<Connection>,
}

impl PhotoIndex {
    /// Open the index with an explicit SQLite journal mode (`WAL` or `DELETE`).
    /// iOS should open with `DELETE` directly instead of converting from WAL on every launch.
    pub fn open_with_journal(
        db_path: String,
        journal_mode: &str,
    ) -> Result<std::sync::Arc<Self>, CoreError> {
        let journal = match journal_mode.to_ascii_uppercase().as_str() {
            "DELETE" => "DELETE",
            _ => "WAL",
        };
        let opened = std::time::Instant::now();
        let conn = Connection::open(Path::new(&db_path))?;
        let pragmas = format!(
            "PRAGMA journal_mode = {journal};
             PRAGMA synchronous = FULL;
             PRAGMA cache_size = -8000;
             PRAGMA temp_store = MEMORY;"
        );
        let _ = conn.execute_batch(&pragmas);

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS photos (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                hash TEXT NOT NULL,
                make TEXT,
                model TEXT,
                date_taken TEXT,
                width INTEGER,
                height INTEGER,
                lat REAL,
                lon REAL,
                iso INTEGER,
                f_number REAL,
                exposure_time TEXT,
                orientation INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_photos_hash ON photos (hash);",
        )?;

        // Skip ALTER/INDEX work after the schema has been applied once.
        const SCHEMA_VERSION: i64 = 3;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap_or(0);
        if version < SCHEMA_VERSION {
            let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN thumb_path TEXT;");
            let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN file_size INTEGER DEFAULT 0;");
            let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN media_type TEXT DEFAULT 'photo';");
            let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN duration_ms INTEGER;");
            let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN video_codec TEXT;");
            let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN audio_codec TEXT;");
            let _ = conn.execute_batch("ALTER TABLE photos ADD COLUMN video_path TEXT;");
            let _ = conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_photos_date_taken ON photos (date_taken);
                 CREATE INDEX IF NOT EXISTS idx_photos_media_type ON photos (media_type);",
            );
            let _ = conn.pragma_update(None, "user_version", SCHEMA_VERSION);
        }

        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap_or_else(|_| journal.to_string());
        eprintln!(
            "[footos] PhotoIndex opened journal={mode} schema={version} in {:?}",
            opened.elapsed()
        );

        Ok(std::sync::Arc::new(Self {
            conn: Mutex::new(conn),
        }))
    }
}

#[uniffi::export]
impl PhotoIndex {
    #[uniffi::constructor]
    pub fn open(db_path: String) -> Result<std::sync::Arc<Self>, CoreError> {
        Self::open_with_journal(db_path, "WAL")
    }

    pub fn insert(
        &self,
        path: String,
        hash: String,
        metadata: PhotoMetadata,
    ) -> Result<PhotoId, CoreError> {
        self.insert_media(path, hash, metadata, MediaType::Photo, None)
    }

    /// Insert a media item (photo, video, or live photo) into the index.
    pub fn insert_media(
        &self,
        path: String,
        hash: String,
        metadata: PhotoMetadata,
        media_type: MediaType,
        video_path: Option<String>,
    ) -> Result<PhotoId, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;

        // 1. Check if path already exists (same file, no change needed)
        let mut stmt = conn.prepare("SELECT id FROM photos WHERE path = ?1")?;
        let mut rows = stmt.query_map(params![path], |row| row.get::<_, i64>(0))?;

        if let Some(existing_id) = rows.next() {
            return Ok(PhotoId { id: existing_id? });
        }
        drop(rows);
        drop(stmt);

        // 2. Check if hash already exists (same photo, different path - update path)
        let mut stmt = conn.prepare("SELECT id FROM photos WHERE hash = ?1")?;
        let mut rows = stmt.query_map(params![hash], |row| row.get::<_, i64>(0))?;

        if let Some(existing_id) = rows.next() {
            let id = existing_id?;
            drop(rows);
            drop(stmt);
            // Update path to new location
            conn.execute(
                "UPDATE photos SET path = ?1 WHERE id = ?2",
                params![path, id],
            )?;
            return Ok(PhotoId { id });
        }
        drop(rows);
        drop(stmt);

        // 3. Insert new record
        conn.execute(
            "INSERT INTO photos (
                path, hash, make, model, date_taken, width, height,
                lat, lon, iso, f_number, exposure_time, orientation,
                media_type, duration_ms, video_codec, audio_codec, video_path
            )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18)",
            params![
                path,
                hash,
                metadata.make,
                metadata.model,
                metadata.date_taken,
                metadata.width,
                metadata.height,
                metadata.lat,
                metadata.lon,
                metadata.iso,
                metadata.f_number,
                metadata.exposure_time,
                metadata.orientation,
                media_type.as_str(),
                metadata.duration_ms.map(|v| v as i64),
                metadata.video_codec,
                metadata.audio_codec,
                video_path,
            ],
        )?;

        Ok(PhotoId {
            id: conn.last_insert_rowid(),
        })
    }

    /// One-statement preview insert. `path` is UNIQUE; duplicates are ignored.
    pub fn insert_preview(
        &self,
        path: String,
        hash: String,
        metadata: PhotoMetadata,
        media_type: MediaType,
        thumb_path: Option<String>,
        file_size: u64,
    ) -> Result<bool, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        let changed = conn.execute(
            "INSERT OR IGNORE INTO photos (
                path, hash, date_taken, width, height, lat, lon, orientation,
                media_type, duration_ms, thumb_path, file_size
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10, ?11)",
            params![
                path,
                hash,
                metadata.date_taken,
                metadata.width,
                metadata.height,
                metadata.lat,
                metadata.lon,
                media_type.as_str(),
                metadata.duration_ms.map(|v| v as i64),
                thumb_path,
                file_size as i64,
            ],
        )?;
        Ok(changed > 0)
    }

    /// Begin an explicit transaction for batch operations.
    ///
    /// All subsequent insert/update/delete calls will execute within this transaction
    /// until `commit_transaction()` or `rollback_transaction()` is called.
    /// This provides 10-50x speed improvement for bulk imports by batching disk syncs
    /// into a single operation at commit time.
    pub fn begin_transaction(&self) -> Result<(), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(())
    }

    pub fn count(&self) -> Result<i64, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        conn.query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))
            .map_err(|e| CoreError::Database(e.to_string()))
    }

    /// Commit any leftover transaction and flush the journal to the main file.
    pub fn flush(&self) -> Result<(), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        let _ = conn.execute_batch("COMMIT");
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(FULL)");
        let _ = conn.execute_batch("PRAGMA synchronous = FULL");
        Ok(())
    }

    /// Prefer rollback-journal so a force-quit cannot drop an un-checkpointed WAL.
    pub fn use_delete_journal(&self) -> Result<(), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        let _ = conn.execute_batch("COMMIT");
        conn.execute_batch("PRAGMA journal_mode = DELETE; PRAGMA synchronous = FULL;")?;
        Ok(())
    }

    /// Flush the WAL into the main database file so a force-quit cannot drop it.
    pub fn checkpoint(&self) -> Result<(), CoreError> {
        self.flush()
    }

    /// Commit the current transaction, persisting all batched writes to disk.
    pub fn commit_transaction(&self) -> Result<(), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        conn.execute_batch("COMMIT")?;
        Ok(())
    }

    /// Rollback the current transaction, discarding all batched writes.
    pub fn rollback_transaction(&self) -> Result<(), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        conn.execute_batch("ROLLBACK")?;
        Ok(())
    }

    pub fn get_by_path(&self, path: String) -> Result<Option<PhotoInfo>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT 
                id, path, hash, make, model, date_taken, width, height,
                lat, lon, iso, f_number, exposure_time, orientation,
                media_type, duration_ms, video_codec, audio_codec, video_path
             FROM photos WHERE path = ?1",
        )?;

        let mut rows = stmt.query_map(params![path], |row| {
            Ok(PhotoInfo {
                id: PhotoId { id: row.get(0)? },
                path: row.get(1)?,
                hash: row.get(2)?,
                metadata: PhotoMetadata {
                    make: row.get(3)?,
                    model: row.get(4)?,
                    date_taken: row.get(5)?,
                    width: row.get::<_, i64>(6)? as u32,
                    height: row.get::<_, i64>(7)? as u32,
                    lat: row.get(8)?,
                    lon: row.get(9)?,
                    iso: row.get::<_, Option<i64>>(10)?.map(|x| x as u32),
                    f_number: row.get::<_, Option<f64>>(11)?.map(|x| x as f32),
                    exposure_time: row.get(12)?,
                    orientation: row.get::<_, i64>(13)? as u32,
                    duration_ms: row.get::<_, Option<i64>>(15)?.map(|x| x as u64),
                    video_codec: row.get(16)?,
                    audio_codec: row.get(17)?,
                },
                thumb_path: None,
                file_size: 0,
                media_type: MediaType::from_str(
                    &row.get::<_, Option<String>>(14)?.unwrap_or_default(),
                ),
                video_path: row.get(18)?,
            })
        })?;

        if let Some(res) = rows.next() {
            Ok(Some(res?))
        } else {
            Ok(None)
        }
    }

    /// Check if a photo with the given hash already exists.
    /// Returns true if exists, false otherwise.
    pub fn exists_by_hash(&self, hash: &str) -> Result<bool, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM photos WHERE hash = ?1",
            params![hash],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get a photo by its ID.
    pub fn get_by_id(&self, id: i64) -> Result<Option<PhotoInfo>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT
                id, path, hash, make, model, date_taken, width, height,
                lat, lon, iso, f_number, exposure_time, orientation,
                media_type, duration_ms, video_codec, audio_codec, video_path
             FROM photos WHERE id = ?1",
        )?;

        let mut rows = stmt.query_map(params![id], |row| {
            Ok(PhotoInfo {
                id: PhotoId { id: row.get(0)? },
                path: row.get(1)?,
                hash: row.get(2)?,
                metadata: PhotoMetadata {
                    make: row.get(3)?,
                    model: row.get(4)?,
                    date_taken: row.get(5)?,
                    width: row.get::<_, i64>(6)? as u32,
                    height: row.get::<_, i64>(7)? as u32,
                    lat: row.get(8)?,
                    lon: row.get(9)?,
                    iso: row.get::<_, Option<i64>>(10)?.map(|x| x as u32),
                    f_number: row.get::<_, Option<f64>>(11)?.map(|x| x as f32),
                    exposure_time: row.get(12)?,
                    orientation: row.get::<_, i64>(13)? as u32,
                    duration_ms: row.get::<_, Option<i64>>(15)?.map(|x| x as u64),
                    video_codec: row.get(16)?,
                    audio_codec: row.get(17)?,
                },
                thumb_path: None,
                file_size: 0,
                media_type: MediaType::from_str(
                    &row.get::<_, Option<String>>(14)?.unwrap_or_default(),
                ),
                video_path: row.get(18)?,
            })
        })?;

        if let Some(res) = rows.next() {
            Ok(Some(res?))
        } else {
            Ok(None)
        }
    }

    /// Delete a photo by its ID.
    /// Returns the deleted photo's info (including path) if found, None if not found.
    pub fn delete_by_id(&self, id: i64) -> Result<Option<PhotoInfo>, CoreError> {
        // First get the photo info
        let photo = self.get_by_id(id)?;

        if photo.is_some() {
            let conn = self
                .conn
                .lock()
                .map_err(|e| CoreError::Database(e.to_string()))?;
            conn.execute("DELETE FROM photos WHERE id = ?1", params![id])?;
        }

        Ok(photo)
    }

    /// Delete multiple photos by their IDs.
    /// Returns the list of deleted photos' info.
    /// Wrapped in a transaction for batch performance.
    pub fn delete_by_ids(&self, ids: Vec<i64>) -> Result<Vec<PhotoInfo>, CoreError> {
        let use_txn = self.begin_transaction().is_ok();
        let mut deleted = Vec::new();

        for id in ids {
            match self.delete_by_id(id) {
                Ok(Some(photo)) => deleted.push(photo),
                Ok(None) => {}
                Err(e) => {
                    if use_txn {
                        let _ = self.rollback_transaction();
                    }
                    return Err(e);
                }
            }
        }

        if use_txn {
            self.commit_transaction()?;
        }
        Ok(deleted)
    }

    /// Update thumbnail path and file size for a photo (by path).
    /// Called after import to persist preview data in the database,
    /// avoiding per-photo file I/O on subsequent list calls.
    pub fn update_thumb_info(
        &self,
        path: String,
        thumb_path: Option<String>,
        file_size: u64,
    ) -> Result<(), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        conn.execute(
            "UPDATE photos SET thumb_path = ?1, file_size = ?2 WHERE path = ?3",
            params![thumb_path, file_size as i64, path],
        )?;
        Ok(())
    }

    /// Update media type and video path for a record (used for Live Photo pairing).
    pub fn update_media_info(
        &self,
        path: String,
        media_type: MediaType,
        video_path: Option<String>,
    ) -> Result<(), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        conn.execute(
            "UPDATE photos SET media_type = ?1, video_path = ?2 WHERE path = ?3",
            params![media_type.as_str(), video_path, path],
        )?;
        Ok(())
    }

    /// Lightweight list for map markers and timeline rendering.
    /// Returns only the fields needed for initial UI display.
    /// **Zero file I/O** — all data comes directly from the database.
    ///
    /// This is 5-10x faster than `list()` for large photo libraries because:
    /// 1. Fewer columns to read and serialize (7 vs 17+)
    /// 2. No nested struct construction
    /// 3. No per-photo filesystem operations
    pub fn list_previews(&self) -> Result<Vec<PhotoPreview>, CoreError> {
        self.list_previews_filtered(false)
    }

    /// Paths already imported from PhotoKit, without the `ios-photo://` prefix.
    pub fn list_ios_identifiers(&self) -> Result<Vec<String>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT path FROM photos WHERE path LIKE 'ios-photo://%'")?;
        let rows = stmt.query_map([], |row| {
            let path: String = row.get(0)?;
            Ok(path
                .strip_prefix("ios-photo://")
                .unwrap_or(path.as_str())
                .to_string())
        })?;
        let mut results = Vec::new();
        for row in rows {
            match row {
                Ok(id) => results.push(id),
                Err(e) => tracing::warn!(error = %e, "PhotoIndex::list_ios_identifiers row error"),
            }
        }
        Ok(results)
    }

    pub fn list_previews_filtered(&self, geotagged_only: bool) -> Result<Vec<PhotoPreview>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;

        let sql = if geotagged_only {
            "SELECT id, path, thumb_path, lat, lon, date_taken, file_size,
                    media_type, duration_ms
             FROM photos
             WHERE lat IS NOT NULL AND lon IS NOT NULL"
        } else {
            "SELECT id, path, thumb_path, lat, lon, date_taken, file_size,
                    media_type, duration_ms
             FROM photos"
        };

        let mut stmt = conn.prepare(sql)?;

        let rows = stmt.query_map([], |row| {
            Ok(PhotoPreview {
                id: row.get(0)?,
                path: row.get(1)?,
                thumb_path: row.get(2)?,
                lat: row.get(3)?,
                lon: row.get(4)?,
                date_taken: row.get(5)?,
                file_size: row.get::<_, Option<i64>>(6)?.unwrap_or(0) as u64,
                media_type: row
                    .get::<_, Option<String>>(7)?
                    .unwrap_or_else(|| "photo".to_string()),
                duration_ms: row.get::<_, Option<i64>>(8)?.map(|x| x as u64),
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            match row {
                Ok(preview) => results.push(preview),
                Err(e) => {
                    tracing::warn!(error = %e, "PhotoIndex::list_previews row error");
                }
            }
        }

        Ok(results)
    }

    /// Returns a list of all photos in the index.
    ///
    /// Now reads `thumb_path` and `file_size` from the database (persisted during import),
    /// eliminating per-photo file I/O that previously made this O(N × disk_io).
    ///
    /// ### ⚠️ Performance & Scale Note
    /// For initial map/timeline rendering, prefer `list_previews()` which returns
    /// a lightweight struct with only the fields needed for display.
    /// Use `list()` only when full metadata is needed (e.g., photo detail view).
    pub fn list(&self) -> Result<Vec<PhotoInfo>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CoreError::Database(e.to_string()))?;

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))
            .unwrap_or(-1);
        tracing::debug!(count, "PhotoIndex::list raw count from DB");

        let mut stmt = conn.prepare(
            "SELECT 
                id, path, hash, make, model, date_taken, width, height,
                lat, lon, iso, f_number, exposure_time, orientation,
                thumb_path, file_size,
                media_type, duration_ms, video_codec, audio_codec, video_path
             FROM photos",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(PhotoInfo {
                id: PhotoId { id: row.get(0)? },
                path: row.get(1)?,
                hash: row.get(2)?,
                metadata: PhotoMetadata {
                    make: row.get(3)?,
                    model: row.get(4)?,
                    date_taken: row.get(5)?,
                    width: row.get::<_, Option<i64>>(6)?.unwrap_or(0) as u32,
                    height: row.get::<_, Option<i64>>(7)?.unwrap_or(0) as u32,
                    lat: row.get(8)?,
                    lon: row.get(9)?,
                    iso: row.get::<_, Option<i64>>(10)?.map(|x| x as u32),
                    f_number: row.get::<_, Option<f64>>(11)?.map(|x| x as f32),
                    exposure_time: row.get(12)?,
                    orientation: row.get::<_, Option<i64>>(13)?.unwrap_or(1) as u32,
                    duration_ms: row.get::<_, Option<i64>>(17)?.map(|x| x as u64),
                    video_codec: row.get(18)?,
                    audio_codec: row.get(19)?,
                },
                // Read thumb_path and file_size from DB (persisted during import)
                thumb_path: row.get(14)?,
                file_size: row.get::<_, Option<i64>>(15)?.unwrap_or(0) as u64,
                media_type: MediaType::from_str(
                    &row.get::<_, Option<String>>(16)?.unwrap_or_default(),
                ),
                video_path: row.get(20)?,
            })
        })?;

        let mut success_count = 0;
        let mut error_count = 0;
        let mut results = Vec::new();

        for row in rows {
            match row {
                Ok(photo) => {
                    success_count += 1;
                    results.push(photo);
                }
                Err(e) => {
                    error_count += 1;
                    if error_count <= 3 {
                        tracing::warn!(error = %e, "PhotoIndex::list row error");
                    }
                }
            }
        }

        tracing::debug!(success_count, error_count, "PhotoIndex::list completed");
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_index() -> std::sync::Arc<PhotoIndex> {
        // Use in-memory database for deterministic, file-system-independent testing
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE photos (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                hash TEXT NOT NULL,
                make TEXT,
                model TEXT,
                date_taken TEXT,
                width INTEGER,
                height INTEGER,
                lat REAL,
                lon REAL,
                iso INTEGER,
                f_number REAL,
                exposure_time TEXT,
                orientation INTEGER,
                thumb_path TEXT,
                file_size INTEGER DEFAULT 0,
                media_type TEXT DEFAULT 'photo',
                duration_ms INTEGER,
                video_codec TEXT,
                audio_codec TEXT,
                video_path TEXT
            );
            CREATE INDEX idx_photos_hash ON photos (hash);
            CREATE INDEX idx_photos_media_type ON photos (media_type);",
        )
        .unwrap();
        std::sync::Arc::new(PhotoIndex {
            conn: Mutex::new(conn),
        })
    }

    #[test]
    fn test_index_uniqueness_invariant() {
        let index = setup_test_index();
        let metadata = PhotoMetadata::default();
        let path = "/test/photo.jpg".to_string();
        let hash = "hash1".to_string();

        let id1 = index
            .insert(path.clone(), hash.clone(), metadata.clone())
            .expect("First insert failed");
        let id2 = index
            .insert(path, hash, metadata)
            .expect("Second insert failed");

        // Contract: Same path returns same ID
        assert_eq!(id1, id2);

        let conn = index.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_hash_deduplication_updates_path() {
        let index = setup_test_index();
        let metadata = PhotoMetadata::default();
        let hash = "same_hash".to_string();
        let old_path = "/sdcard/photo.jpg".to_string();
        let new_path = "/local/photo.jpg".to_string();

        // Insert with old path
        let id1 = index
            .insert(old_path.clone(), hash.clone(), metadata.clone())
            .expect("First insert failed");

        // Insert same hash with new path - should update, not create new
        let id2 = index
            .insert(new_path.clone(), hash.clone(), metadata)
            .expect("Second insert failed");

        // Contract: Same hash returns same ID
        assert_eq!(id1, id2);

        // Only one record in database
        let conn = index.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Path should be updated to new location
        let stored_path: String = conn
            .query_row("SELECT path FROM photos WHERE id = ?1", [id1.id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored_path, new_path);
    }

    #[test]
    fn test_index_scale_performance_degradation() {
        let index = setup_test_index();
        let metadata = PhotoMetadata::default();

        // Scale test: Insertion of many items (each with unique hash)
        for i in 0..1000 {
            let path = format!("/path/photo_{}.jpg", i);
            let hash = format!("hash_{}", i);
            index
                .insert(path, hash, metadata.clone())
                .expect("Bulk insert failed");
        }

        let list = index.list().expect("List failed");
        assert_eq!(list.len(), 1000);
    }

    #[test]
    fn test_update_thumb_info_persists_to_list() {
        let index = setup_test_index();
        let metadata = PhotoMetadata {
            lat: Some(35.6762),
            lon: Some(139.6503),
            date_taken: Some("2024-01-15 10:30:00".to_string()),
            ..Default::default()
        };

        let path = "/photos/tokyo.jpg".to_string();
        index
            .insert(path.clone(), "hash_tokyo".to_string(), metadata)
            .expect("Insert failed");

        // Before update: thumb_path and file_size should be None/0
        let list = index.list().expect("List failed");
        assert_eq!(list.len(), 1);
        assert!(list[0].thumb_path.is_none());
        assert_eq!(list[0].file_size, 0);

        // Update thumb info
        index
            .update_thumb_info(
                path.clone(),
                Some("/cache/ab/thumb.jpg".to_string()),
                1024 * 1024, // 1MB
            )
            .expect("Update thumb info failed");

        // After update: values should be persisted
        let list = index.list().expect("List after update failed");
        assert_eq!(list[0].thumb_path.as_deref(), Some("/cache/ab/thumb.jpg"));
        assert_eq!(list[0].file_size, 1024 * 1024);
    }

    #[test]
    fn test_list_previews_returns_lightweight_data() {
        let index = setup_test_index();

        // Insert photos with various metadata
        for i in 0..100 {
            let metadata = PhotoMetadata {
                lat: if i % 2 == 0 {
                    Some(35.0 + i as f64 * 0.1)
                } else {
                    None
                },
                lon: if i % 2 == 0 {
                    Some(139.0 + i as f64 * 0.1)
                } else {
                    None
                },
                date_taken: Some(format!("2024-01-{:02} 10:30:00", (i % 28) + 1)),
                make: Some("Canon".to_string()),
                model: Some("EOS R5".to_string()),
                width: 8192,
                height: 5464,
                ..Default::default()
            };

            let path = format!("/photos/photo_{}.jpg", i);
            index
                .insert(path.clone(), format!("hash_{}", i), metadata)
                .expect("Insert failed");

            // Set thumb_path for even-numbered photos
            if i % 2 == 0 {
                index
                    .update_thumb_info(
                        path,
                        Some(format!("/cache/{}.jpg", i)),
                        (i as u64 + 1) * 1000,
                    )
                    .expect("Update failed");
            }
        }

        let previews = index.list_previews().expect("list_previews failed");
        assert_eq!(previews.len(), 100);

        // Check that preview data is correct
        let first_preview = &previews[0];
        assert_eq!(first_preview.path, "/photos/photo_0.jpg");
        assert_eq!(first_preview.thumb_path.as_deref(), Some("/cache/0.jpg"));
        assert!(first_preview.lat.is_some());
        assert!(first_preview.date_taken.is_some());
        assert_eq!(first_preview.file_size, 1000);

        // Check photo without thumb_path
        let odd_preview = previews
            .iter()
            .find(|p| p.path == "/photos/photo_1.jpg")
            .unwrap();
        assert!(odd_preview.thumb_path.is_none());
        assert!(odd_preview.lat.is_none());
    }
}
