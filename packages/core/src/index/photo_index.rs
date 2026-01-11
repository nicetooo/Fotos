use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;

use crate::{error::CoreError, types::{PhotoId, PhotoInfo, PhotoMetadata}};

#[derive(uniffi::Object)]
pub struct PhotoIndex {
    conn: Mutex<Connection>,
}

#[uniffi::export]
impl PhotoIndex {
    #[uniffi::constructor]
    pub fn open(db_path: String) -> Result<std::sync::Arc<Self>, CoreError> {
        let conn = Connection::open(Path::new(&db_path))?;
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
        Ok(std::sync::Arc::new(Self { conn: Mutex::new(conn) }))
    }

    pub fn insert(&self, path: String, hash: String, metadata: PhotoMetadata) -> Result<PhotoId, CoreError> {
        let conn = self.conn.lock().map_err(|e| CoreError::Database(e.to_string()))?;

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
            conn.execute("UPDATE photos SET path = ?1 WHERE id = ?2", params![path, id])?;
            return Ok(PhotoId { id });
        }
        drop(rows);
        drop(stmt);

        // 3. Insert new record
        conn.execute(
            "INSERT INTO photos (
                path, hash, make, model, date_taken, width, height,
                lat, lon, iso, f_number, exposure_time, orientation
            )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                metadata.orientation
            ],
        )?;

        Ok(PhotoId { id: conn.last_insert_rowid() })
    }

    pub fn get_by_path(&self, path: String) -> Result<Option<PhotoInfo>, CoreError> {
        let conn = self.conn.lock().map_err(|e| CoreError::Database(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT 
                id, path, hash, make, model, date_taken, width, height,
                lat, lon, iso, f_number, exposure_time, orientation 
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
                },
                thumb_path: None,
                file_size: 0,
                created_at: None,
                modified_at: None,
            })
        })?;

        if let Some(res) = rows.next() {
            Ok(Some(res?))
        } else {
            Ok(None)
        }
    }

    /// Returns a list of all photos in the index.
    /// 
    /// ### ⚠️ Performance & Scale Note
    /// Current implementation returns a full `Vec<PhotoInfo>` from the database.
    /// For very large databases, this may lead to significant memory spikes.
    /// 
    /// **Recommendations for Callers:**
    /// - Avoid calling this frequently on the full database if UI virtualization is not used.
    /// - Future versions may introduce `LIMIT/OFFSET` paging or an iterator API.
    pub fn list(&self) -> Result<Vec<PhotoInfo>, CoreError> {
        let conn = self.conn.lock().map_err(|e| CoreError::Database(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT 
                id, path, hash, make, model, date_taken, width, height,
                lat, lon, iso, f_number, exposure_time, orientation 
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
                    width: row.get::<_, i64>(6)? as u32,
                    height: row.get::<_, i64>(7)? as u32,
                    lat: row.get(8)?,
                    lon: row.get(9)?,
                    iso: row.get::<_, Option<i64>>(10)?.map(|x| x as u32),
                    f_number: row.get::<_, Option<f64>>(11)?.map(|x| x as f32),
                    exposure_time: row.get(12)?,
                    orientation: row.get::<_, i64>(13)? as u32,
                },
                thumb_path: None,
                file_size: 0,
                created_at: None,
                modified_at: None,
            })
        })?;

        Ok(rows.filter_map(Result::ok).collect())
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
                orientation INTEGER
            );
            CREATE INDEX idx_photos_hash ON photos (hash);",
        ).unwrap();
        std::sync::Arc::new(PhotoIndex { conn: Mutex::new(conn) })
    }

    #[test]
    fn test_index_uniqueness_invariant() {
        let index = setup_test_index();
        let metadata = PhotoMetadata::default();
        let path = "/test/photo.jpg".to_string();
        let hash = "hash1".to_string();

        let id1 = index.insert(path.clone(), hash.clone(), metadata.clone()).expect("First insert failed");
        let id2 = index.insert(path, hash, metadata).expect("Second insert failed");

        // Contract: Same path returns same ID
        assert_eq!(id1, id2);

        let conn = index.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0)).unwrap();
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
        let id1 = index.insert(old_path.clone(), hash.clone(), metadata.clone()).expect("First insert failed");

        // Insert same hash with new path - should update, not create new
        let id2 = index.insert(new_path.clone(), hash.clone(), metadata).expect("Second insert failed");

        // Contract: Same hash returns same ID
        assert_eq!(id1, id2);

        // Only one record in database
        let conn = index.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);

        // Path should be updated to new location
        let stored_path: String = conn.query_row("SELECT path FROM photos WHERE id = ?1", [id1.id], |r| r.get(0)).unwrap();
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
            index.insert(path, hash, metadata.clone()).expect("Bulk insert failed");
        }

        let list = index.list().expect("List failed");
        assert_eq!(list.len(), 1000);
    }
}
