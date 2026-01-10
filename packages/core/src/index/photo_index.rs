use rusqlite::{Connection, params};
use std::path::Path;

use crate::{error::CoreError, types::{PhotoId, PhotoInfo, PhotoMetadata}};

pub struct PhotoIndex {
    conn: Connection,
}

impl PhotoIndex {
    pub fn open(db_path: &Path) -> Result<Self, CoreError> {
        let conn = Connection::open(db_path)?;
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
        Ok(Self { conn })
    }

    pub fn insert(&self, path: &str, hash: &str, metadata: &PhotoMetadata) -> Result<PhotoId, CoreError> {
        // 1. Check if path already exists
        let mut stmt = self.conn.prepare("SELECT id FROM photos WHERE path = ?1")?;
        let mut rows = stmt.query_map(params![path], |row| row.get::<_, i64>(0))?;
        
        if let Some(existing_id) = rows.next() {
            return Ok(PhotoId(existing_id?));
        }

        // 2. Insert new record
        self.conn.execute(
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

        Ok(PhotoId(self.conn.last_insert_rowid()))
    }

    pub fn get_by_path(&self, path: &str) -> Result<Option<PhotoInfo>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT 
                id, path, hash, make, model, date_taken, width, height,
                lat, lon, iso, f_number, exposure_time, orientation 
             FROM photos WHERE path = ?1",
        )?;
        
        let mut rows = stmt.query_map(params![path], |row| {
            Ok(PhotoInfo {
                id: PhotoId(row.get(0)?),
                path: std::path::PathBuf::from(row.get::<_, String>(1)?),
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
        let mut stmt = self.conn.prepare(
            "SELECT 
                id, path, hash, make, model, date_taken, width, height,
                lat, lon, iso, f_number, exposure_time, orientation 
             FROM photos",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(PhotoInfo {
                id: PhotoId(row.get(0)?),
                path: std::path::PathBuf::from(row.get::<_, String>(1)?),
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
            })
        })?;

        Ok(rows.filter_map(Result::ok).collect())
    }
}
