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
            let path_str: String = row.get(1)?;
            Ok(PhotoInfo {
                id: PhotoId(row.get(0)?),
                path: std::path::PathBuf::from(path_str),
                width: row.get::<_, i64>(2)? as u32,
                height: row.get::<_, i64>(3)? as u32,
            })
        })?;

        Ok(rows.filter_map(Result::ok).collect())
    }
}
