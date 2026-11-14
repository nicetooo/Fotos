//! What changed in a library folder since it was last scanned.
//!
//! A pure comparison of what is on disk with what the index knows. The caller
//! does the scanning and acts on the plan.

use std::collections::HashMap;

/// A file as found on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct FileStat {
    pub path: String,
    pub size: u64,
    /// Modification time, milliseconds since the Unix epoch.
    pub mtime_ms: i64,
}

/// A file as the index knows it. `mtime_ms` is 0 for rows written before
/// modification times were recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownFile {
    pub id: i64,
    pub path: String,
    pub size: u64,
    pub mtime_ms: i64,
}

#[derive(Debug, Default, PartialEq)]
pub struct RescanPlan {
    /// On disk, not in the index.
    pub new: Vec<String>,
    /// In both, but the file was modified since it was indexed.
    pub changed: Vec<String>,
    /// In both and unchanged, but the index has no modification time yet:
    /// `(id, size, mtime_ms)` to record without reprocessing the file.
    pub backfill: Vec<(i64, u64, i64)>,
    /// In the index under this folder, no longer on disk.
    pub missing: Vec<i64>,
}

/// Coarse file systems (FAT, some network shares) round modification times.
const MTIME_TOLERANCE_MS: i64 = 2_000;

/// Is `path` inside the folder `root` (not merely sharing a name prefix)?
pub fn is_under(root: &str, path: &str) -> bool {
    let root = root.trim_end_matches('/');
    path.len() > root.len() && path.starts_with(root) && path.as_bytes()[root.len()] == b'/'
}

/// Compare a scan of `root` with the index.
///
/// `known` may hold rows from anywhere; only those under `root` are considered.
/// Nothing is reported missing when the scan came back empty: an unmounted
/// drive or an unreadable folder looks exactly like that, and dropping the
/// whole folder from the library on that evidence would be wrong.
pub fn plan_rescan(root: &str, scanned: &[FileStat], known: &[KnownFile]) -> RescanPlan {
    let mut plan = RescanPlan::default();
    let mut on_disk: HashMap<&str, &FileStat> = HashMap::with_capacity(scanned.len());
    for file in scanned {
        on_disk.insert(file.path.as_str(), file);
    }

    let mut seen: HashMap<&str, ()> = HashMap::new();
    for row in known {
        if !is_under(root, &row.path) {
            continue;
        }
        match on_disk.get(row.path.as_str()) {
            None => {
                if !scanned.is_empty() {
                    plan.missing.push(row.id);
                }
            }
            Some(file) => {
                seen.insert(row.path.as_str(), ());
                let size_changed = row.size != 0 && row.size != file.size;
                let time_changed =
                    row.mtime_ms != 0 && (row.mtime_ms - file.mtime_ms).abs() > MTIME_TOLERANCE_MS;
                if size_changed || time_changed {
                    plan.changed.push(file.path.clone());
                } else if row.mtime_ms == 0 || row.size == 0 {
                    plan.backfill.push((row.id, file.size, file.mtime_ms));
                }
            }
        }
    }

    for file in scanned {
        if !seen.contains_key(file.path.as_str()) {
            plan.new.push(file.path.clone());
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(path: &str, size: u64, mtime_ms: i64) -> FileStat {
        FileStat { path: path.into(), size, mtime_ms }
    }

    fn row(id: i64, path: &str, size: u64, mtime_ms: i64) -> KnownFile {
        KnownFile { id, path: path.into(), size, mtime_ms }
    }

    #[test]
    fn sorts_files_into_new_changed_and_missing() {
        let scanned = vec![
            disk("/lib/a.jpg", 100, 5_000_000),
            disk("/lib/b.jpg", 222, 6_000_000),
            disk("/lib/sub/c.jpg", 300, 7_000_000),
            disk("/lib/new.jpg", 400, 8_000_000),
        ];
        let known = vec![
            row(1, "/lib/a.jpg", 100, 5_000_000),
            row(2, "/lib/b.jpg", 200, 6_000_000),     // size differs
            row(3, "/lib/sub/c.jpg", 300, 1_000_000), // mtime differs
            row(4, "/lib/gone.jpg", 50, 2_000_000),
        ];
        let plan = plan_rescan("/lib", &scanned, &known);
        assert_eq!(plan.new, vec!["/lib/new.jpg"]);
        assert_eq!(plan.changed, vec!["/lib/b.jpg", "/lib/sub/c.jpg"]);
        assert_eq!(plan.missing, vec![4]);
        assert!(plan.backfill.is_empty());
    }

    #[test]
    fn rows_without_a_recorded_time_are_backfilled_not_reprocessed() {
        let scanned = vec![disk("/lib/a.jpg", 100, 5_000_000)];
        let known = vec![row(1, "/lib/a.jpg", 100, 0)];
        let plan = plan_rescan("/lib", &scanned, &known);
        assert!(plan.changed.is_empty());
        assert_eq!(plan.backfill, vec![(1, 100, 5_000_000)]);
    }

    #[test]
    fn coarse_timestamps_are_not_a_change() {
        let scanned = vec![disk("/lib/a.jpg", 100, 5_001_500)];
        let known = vec![row(1, "/lib/a.jpg", 100, 5_000_000)];
        assert_eq!(plan_rescan("/lib", &scanned, &known), RescanPlan::default());
    }

    #[test]
    fn an_empty_scan_never_reports_missing_files() {
        let known = vec![row(1, "/lib/a.jpg", 100, 5_000_000)];
        assert_eq!(plan_rescan("/lib", &[], &known), RescanPlan::default());
    }

    #[test]
    fn rows_outside_the_folder_are_left_alone() {
        let scanned = vec![disk("/lib/a.jpg", 100, 5_000_000)];
        let known = vec![
            row(1, "/lib/a.jpg", 100, 5_000_000),
            row(2, "/library/other.jpg", 1, 1), // shares a prefix, different folder
            row(3, "/elsewhere/x.jpg", 1, 1),
            row(4, "ios-photo://ABC", 1, 1),
        ];
        let plan = plan_rescan("/lib/", &scanned, &known);
        assert!(plan.missing.is_empty());
        assert!(plan.new.is_empty());
    }
}
