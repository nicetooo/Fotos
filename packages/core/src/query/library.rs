//! The library list (desktop drawer): RAW+JPEG grouping, filtering, sorting.
//!
//! The frontend asks for one page of the ordered list at a time; it never
//! receives the whole library. Pure data + pure functions.

use std::cmp::Ordering;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{MediaType, PhotoInfo};

const RAW_EXTENSIONS: [&str; 13] = [
    "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "dng", "raf", "orf", "rw2", "pef", "raw",
];

/// One entry of the library list: a file, with its paired RAW folded in.
#[derive(Debug, Clone, Serialize)]
pub struct LibraryItem {
    #[serde(flatten)]
    pub info: PhotoInfo,
    /// A RAW file with the same name sits next to this file.
    #[serde(rename = "hasRaw")]
    pub has_raw: bool,
    #[serde(rename = "rawPath")]
    pub raw_path: Option<String>,
    #[serde(rename = "rawId")]
    pub raw_id: Option<i64>,
    #[serde(rename = "rawSize")]
    pub raw_size: u64,
    /// A RAW file with no JPEG next to it.
    #[serde(rename = "isRawOnly")]
    pub is_raw_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySort {
    Name,
    Date,
    /// Pixel count. `size` is the name older settings stored for it.
    #[serde(alias = "size")]
    Dimensions,
}

/// The grouped library. Build once per library change, then ask for orders.
pub struct LibraryView {
    items: Vec<LibraryItem>,
    /// Row id -> position in `items`. A paired RAW row points at its JPEG.
    by_id: HashMap<i64, u32>,
    files: usize,
}

fn extension(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[dot + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn is_raw(path: &str) -> bool {
    RAW_EXTENSIONS.contains(&extension(path).as_str())
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Directory + lowercased file name without extension: files that share it are one shot.
fn pair_key(path: &str) -> String {
    let (dir, name) = match path.rfind('/') {
        Some(slash) => (&path[..slash], &path[slash + 1..]),
        None => ("", path),
    };
    let stem = match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    };
    format!("{}/{}", dir, stem.to_lowercase())
}

impl LibraryView {
    pub fn build(photos: Vec<PhotoInfo>) -> Self {
        let files = photos.len();

        // First RAW per pair key, among files that can pair (videos and Live Photos never do).
        let mut raws: HashMap<String, usize> = HashMap::new();
        let mut has_other: HashMap<String, bool> = HashMap::new();
        let mut keys: Vec<Option<String>> = Vec::with_capacity(files);
        for (i, p) in photos.iter().enumerate() {
            if p.media_type != MediaType::Photo {
                keys.push(None);
                continue;
            }
            let key = pair_key(&p.path);
            if is_raw(&p.path) {
                raws.entry(key.clone()).or_insert(i);
            } else {
                has_other.insert(key.clone(), true);
            }
            keys.push(Some(key));
        }

        let mut items: Vec<LibraryItem> = Vec::with_capacity(files);
        let mut by_id: HashMap<i64, u32> = HashMap::with_capacity(files);
        // RAW rows folded into a JPEG: (raw row id, pair key).
        let mut folded: Vec<(i64, String)> = Vec::new();
        let mut first_item_of_key: HashMap<String, u32> = HashMap::new();

        for (i, p) in photos.iter().enumerate() {
            let mut item = LibraryItem {
                info: p.clone(),
                has_raw: false,
                raw_path: None,
                raw_id: None,
                raw_size: 0,
                is_raw_only: false,
            };
            if let Some(key) = &keys[i] {
                let raw = is_raw(&p.path);
                let paired = has_other.contains_key(key);
                if raw && paired {
                    folded.push((p.id.id, key.clone()));
                    continue;
                }
                if raw {
                    item.is_raw_only = true;
                } else if let Some(&r) = raws.get(key) {
                    item.has_raw = true;
                    item.raw_path = Some(photos[r].path.clone());
                    item.raw_id = Some(photos[r].id.id);
                    item.raw_size = photos[r].file_size;
                }
                first_item_of_key.entry(key.clone()).or_insert(items.len() as u32);
            }
            by_id.insert(p.id.id, items.len() as u32);
            items.push(item);
        }
        for (id, key) in folded {
            if let Some(&pos) = first_item_of_key.get(&key) {
                by_id.insert(id, pos);
            }
        }

        Self { items, by_id, files }
    }

    /// Entries after grouping.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Rows before grouping.
    pub fn files(&self) -> usize {
        self.files
    }

    pub fn get(&self, pos: u32) -> Option<&LibraryItem> {
        self.items.get(pos as usize)
    }

    /// The entry a row belongs to (a paired RAW resolves to its JPEG).
    pub fn by_id(&self, id: i64) -> Option<&LibraryItem> {
        self.by_id.get(&id).and_then(|&pos| self.get(pos))
    }

    /// Positions of the entries matching `filter`, in display order.
    /// `located`: `Some(true)` keeps only entries with a position, `Some(false)`
    /// only those without, `None` all.
    pub fn order(
        &self,
        sort: LibrarySort,
        descending: bool,
        filter: Option<MediaType>,
        located: Option<bool>,
    ) -> Vec<u32> {
        let has_position = |info: &PhotoInfo| info.metadata.lat.is_some() && info.metadata.lon.is_some();
        let mut order: Vec<u32> = (0..self.items.len() as u32)
            .filter(|&i| filter.map_or(true, |m| self.items[i as usize].info.media_type == m))
            .filter(|&i| located.map_or(true, |want| has_position(&self.items[i as usize].info) == want))
            .collect();

        let names: Vec<String> = if sort == LibrarySort::Name {
            self.items.iter().map(|it| file_name(&it.info.path).to_lowercase()).collect()
        } else {
            Vec::new()
        };

        order.sort_by(|&a, &b| {
            let (x, y) = (&self.items[a as usize].info, &self.items[b as usize].info);
            let primary = match sort {
                LibrarySort::Name => names[a as usize].cmp(&names[b as usize]),
                LibrarySort::Date => x
                    .metadata
                    .date_taken
                    .as_deref()
                    .unwrap_or("")
                    .cmp(y.metadata.date_taken.as_deref().unwrap_or("")),
                LibrarySort::Dimensions => (x.metadata.width as u64 * x.metadata.height as u64)
                    .cmp(&(y.metadata.width as u64 * y.metadata.height as u64)),
            };
            let primary = if descending { primary.reverse() } else { primary };
            match primary {
                Ordering::Equal => x.id.id.cmp(&y.id.id),
                other => other,
            }
        });
        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PhotoId, PhotoMetadata};

    fn photo(id: i64, path: &str, date: &str, w: u32, h: u32, media: MediaType) -> PhotoInfo {
        PhotoInfo {
            id: PhotoId { id },
            path: path.to_string(),
            hash: String::new(),
            metadata: PhotoMetadata {
                date_taken: if date.is_empty() { None } else { Some(date.to_string()) },
                width: w,
                height: h,
                ..Default::default()
            },
            thumb_path: None,
            file_size: 1000 * id as u64,
            media_type: media,
            video_path: None,
        }
    }

    fn sample() -> LibraryView {
        LibraryView::build(vec![
            photo(1, "/a/IMG_1.JPG", "2024-01-03 10:00:00", 400, 300, MediaType::Photo),
            photo(2, "/a/IMG_1.CR3", "2024-01-03 10:00:00", 6000, 4000, MediaType::Photo),
            photo(3, "/a/IMG_2.NEF", "2024-01-01 09:00:00", 6000, 4000, MediaType::Photo),
            photo(4, "/b/IMG_1.jpg", "2024-01-02 08:00:00", 100, 100, MediaType::Photo),
            photo(5, "/a/clip.mov", "", 1920, 1080, MediaType::Video),
            photo(6, "/a/clip.dng", "2024-01-05 08:00:00", 10, 10, MediaType::Photo),
        ])
    }

    #[test]
    fn folds_raw_into_the_jpeg_next_to_it() {
        let v = sample();
        assert_eq!(v.files(), 6);
        assert_eq!(v.len(), 5);
        let jpeg = v.by_id(1).unwrap();
        assert!(jpeg.has_raw);
        assert_eq!(jpeg.raw_path.as_deref(), Some("/a/IMG_1.CR3"));
        assert_eq!(jpeg.raw_id, Some(2));
        assert_eq!(jpeg.raw_size, 2000);
        // The RAW row resolves to the JPEG entry.
        assert_eq!(v.by_id(2).unwrap().info.id.id, 1);
    }

    #[test]
    fn same_name_in_another_folder_does_not_pair() {
        let v = sample();
        let other = v.by_id(4).unwrap();
        assert!(!other.has_raw);
    }

    #[test]
    fn lone_raw_is_raw_only_and_videos_never_pair() {
        let v = sample();
        assert!(v.by_id(3).unwrap().is_raw_only);
        // clip.dng sits next to clip.mov, but a video is not a JPEG.
        assert!(v.by_id(6).unwrap().is_raw_only);
        assert!(!v.by_id(5).unwrap().has_raw);
    }

    fn ids(v: &LibraryView, order: &[u32]) -> Vec<i64> {
        order.iter().map(|&i| v.get(i).unwrap().info.id.id).collect()
    }

    #[test]
    fn sorts_and_filters() {
        let v = sample();
        // Missing dates sort first ascending.
        assert_eq!(ids(&v, &v.order(LibrarySort::Date, false, None, None)), vec![5, 3, 4, 1, 6]);
        assert_eq!(ids(&v, &v.order(LibrarySort::Date, true, None, None)), vec![6, 1, 4, 3, 5]);
        assert_eq!(ids(&v, &v.order(LibrarySort::Name, false, None, None)), vec![6, 5, 1, 4, 3]);
        assert_eq!(ids(&v, &v.order(LibrarySort::Dimensions, true, None, None)), vec![3, 5, 1, 4, 6]);
        assert_eq!(ids(&v, &v.order(LibrarySort::Date, false, Some(MediaType::Video), None)), vec![5]);
        assert_eq!(v.order(LibrarySort::Date, false, Some(MediaType::LivePhoto), None).len(), 0);
    }

    #[test]
    fn sort_name_accepts_legacy_size_key() {
        let s: LibrarySort = serde_json::from_str("\"size\"").unwrap();
        assert_eq!(s, LibrarySort::Dimensions);
    }

    #[test]
    fn serializes_like_the_frontend_photo() {
        let v = sample();
        let json = serde_json::to_value(v.by_id(1).unwrap()).unwrap();
        assert_eq!(json["id"]["id"], 1);
        assert_eq!(json["hasRaw"], true);
        assert_eq!(json["rawPath"], "/a/IMG_1.CR3");
        assert_eq!(json["media_type"], "photo");
    }

    #[test]
    fn filters_by_whether_there_is_a_position() {
        let mut here = photo(1, "/a/here.jpg", "2024-01-01 10:00:00", 10, 10, MediaType::Photo);
        here.metadata.lat = Some(31.2);
        here.metadata.lon = Some(121.5);
        let nowhere = photo(2, "/a/nowhere.jpg", "2024-01-02 10:00:00", 10, 10, MediaType::Photo);
        let v = LibraryView::build(vec![here, nowhere]);
        let ids = |order: Vec<u32>| -> Vec<i64> { order.iter().map(|&i| v.get(i).unwrap().info.id.id).collect() };
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, None)), vec![1, 2]);
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, Some(true))), vec![1]);
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, Some(false))), vec![2]);
    }
}
