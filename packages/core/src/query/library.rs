//! The library list (desktop drawer): RAW+JPEG grouping, filtering, sorting.
//!
//! The frontend asks for one page of the ordered list at a time; it never
//! receives the whole library. Pure data + pure functions.

use std::cmp::Ordering;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::query::{parse_date_ms, Bbox};
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
    /// When this was taken, as wall-clock ms: the capture time when the file
    /// records one, otherwise when the file was last modified. Photos passed
    /// around in chat apps carry no capture time, and without a stand-in they
    /// have no place on a timeline and no time span ever includes them.
    #[serde(rename = "takenMs")]
    pub taken_ms: Option<i64>,
    /// `taken_ms` is the file's date, not a capture time.
    #[serde(rename = "takenFromFile")]
    pub taken_from_file: bool,
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
/// Limits the library list to what the map is showing.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionFilter {
    pub bbox: Bbox,
    /// The map's time filter (wall-clock ms), when one is set.
    pub start_ms: Option<f64>,
    pub end_ms: Option<f64>,
    /// The span of time the list itself is set to. Without one, the span of
    /// the placed entries in the region is used (`LibraryView::region_span`).
    #[serde(default)]
    pub span_start_ms: Option<f64>,
    #[serde(default)]
    pub span_end_ms: Option<f64>,
}

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
        Self::build_with_file_times(photos, &HashMap::new())
    }

    /// `file_times`: row id -> the file's modification time (ms since the
    /// epoch), used as the date of entries that record no capture time.
    pub fn build_with_file_times(photos: Vec<PhotoInfo>, file_times: &HashMap<i64, i64>) -> Self {
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
                taken_ms: None,
                taken_from_file: false,
            };
            let captured = p.metadata.date_taken.as_deref().and_then(parse_date_ms);
            let from_file = file_times.get(&p.id.id).copied().filter(|&t| t > 0);
            item.taken_ms = captured.or(from_file);
            item.taken_from_file = captured.is_none() && from_file.is_some();
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
    /// Whether a placed entry is inside the region's box and the map's time filter.
    fn in_view(&self, info: &PhotoInfo, lat: f64, lon: f64, region: &RegionFilter) -> bool {
        if !region.bbox.contains(lat, lon) {
            return false;
        }
        // As on the map: an entry without a date passes the time filter.
        match info.metadata.date_taken.as_deref().and_then(parse_date_ms) {
            Some(t) => {
                !(region.start_ms.map_or(false, |s| (t as f64) < s) || region.end_ms.map_or(false, |e| (t as f64) > e))
            }
            None => true,
        }
    }

    /// The span of time (wall-clock ms) covered by the placed entries the
    /// region shows: what the list's timeline selects when it follows the map.
    pub fn region_span(&self, region: &RegionFilter) -> Option<(i64, i64)> {
        let mut span: Option<(i64, i64)> = None;
        for item in &self.items {
            let (Some(lat), Some(lon)) = (item.info.metadata.lat, item.info.metadata.lon) else { continue };
            if !self.in_view(&item.info, lat, lon, region) {
                continue;
            }
            if let Some(t) = item.taken_ms {
                span = Some(span.map_or((t, t), |(lo, hi)| (lo.min(t), hi.max(t))));
            }
        }
        span
    }

    /// Ascending wall-clock timestamps (ms) of every dated entry: what the
    /// list's timeline draws.
    pub fn timestamps(&self) -> Vec<f64> {
        let mut stamps: Vec<f64> = self
            .items
            .iter()
            .filter_map(|item| item.taken_ms)
            .map(|t| t as f64)
            .collect();
        stamps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        stamps
    }

    /// `located`: `Some(true)` keeps only entries with a position, `Some(false)`
    /// only those without, `None` all. `region`: see `RegionFilter`.
    pub fn order(
        &self,
        sort: LibrarySort,
        descending: bool,
        filter: Option<MediaType>,
        located: Option<bool>,
        region: Option<RegionFilter>,
    ) -> Vec<u32> {
        let position = |info: &PhotoInfo| match (info.metadata.lat, info.metadata.lon) {
            (Some(lat), Some(lon)) => Some((lat, lon)),
            _ => None,
        };
        let has_position = |info: &PhotoInfo| position(info).is_some();

        // What the region lets through: placed entries inside the box (and
        // the map's time filter), plus entries without a position, all within
        // one span of time. By default that span is the one the placed
        // entries cover: what has no position was most likely shot on the
        // same outings, by a camera that records none. The list can set its
        // own span instead.
        //
        // A view that holds every placed entry, with no time filter and no
        // span set, is the whole library: nothing is left out then,
        // including entries with neither a position nor a date, which no
        // region could ever reach.
        let in_region: Option<Vec<bool>> = region.and_then(|region| {
            let own_span = match (region.span_start_ms, region.span_end_ms) {
                (Some(lo), Some(hi)) => Some((lo as i64, hi as i64)),
                _ => None,
            };
            let mut keep = vec![false; self.items.len()];
            let mut covered: Option<(i64, i64)> = None;
            let mut outside = 0usize;
            for (i, item) in self.items.iter().enumerate() {
                let Some((lat, lon)) = position(&item.info) else { continue };
                if !self.in_view(&item.info, lat, lon, &region) {
                    outside += 1;
                    continue;
                }
                let taken = item.taken_ms;
                if let Some(t) = taken {
                    covered = Some(covered.map_or((t, t), |(lo, hi)| (lo.min(t), hi.max(t))));
                }
                // An entry without a date is in no span the list sets.
                keep[i] = match own_span {
                    Some((lo, hi)) => taken.map_or(false, |t| t >= lo && t <= hi),
                    None => true,
                };
            }
            if let Some((lo, hi)) = own_span.or(covered) {
                for (i, item) in self.items.iter().enumerate() {
                    if !has_position(&item.info) {
                        keep[i] = item.taken_ms.map_or(false, |t| t >= lo && t <= hi);
                    }
                }
            }
            (outside > 0 || own_span.is_some()).then_some(keep)
        });

        let mut order: Vec<u32> = (0..self.items.len() as u32)
            .filter(|&i| filter.map_or(true, |m| self.items[i as usize].info.media_type == m))
            .filter(|&i| located.map_or(true, |want| has_position(&self.items[i as usize].info) == want))
            .filter(|&i| in_region.as_ref().map_or(true, |keep| keep[i as usize]))
            .collect();

        let names: Vec<String> = if sort == LibrarySort::Name {
            self.items.iter().map(|it| file_name(&it.info.path).to_lowercase()).collect()
        } else {
            Vec::new()
        };

        order.sort_by(|&a, &b| {
            let (x, y) = (&self.items[a as usize].info, &self.items[b as usize].info);
            // Entries with no date at all sort before everything else.
            let taken = |i: u32| self.items[i as usize].taken_ms.unwrap_or(i64::MIN);
            let primary = match sort {
                LibrarySort::Name => names[a as usize].cmp(&names[b as usize]),
                LibrarySort::Date => taken(a).cmp(&taken(b)),
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
        assert_eq!(ids(&v, &v.order(LibrarySort::Date, false, None, None, None)), vec![5, 3, 4, 1, 6]);
        assert_eq!(ids(&v, &v.order(LibrarySort::Date, true, None, None, None)), vec![6, 1, 4, 3, 5]);
        assert_eq!(ids(&v, &v.order(LibrarySort::Name, false, None, None, None)), vec![6, 5, 1, 4, 3]);
        assert_eq!(ids(&v, &v.order(LibrarySort::Dimensions, true, None, None, None)), vec![3, 5, 1, 4, 6]);
        assert_eq!(ids(&v, &v.order(LibrarySort::Date, false, Some(MediaType::Video), None, None)), vec![5]);
        assert_eq!(v.order(LibrarySort::Date, false, Some(MediaType::LivePhoto), None, None).len(), 0);
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
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, None, None)), vec![1, 2]);
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, Some(true), None)), vec![1]);
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, Some(false), None)), vec![2]);
    }

    #[test]
    fn a_region_brings_along_unplaced_photos_from_the_same_span() {
        let at = |id: i64, date: &str, pos: Option<(f64, f64)>| {
            let mut p = photo(id, &format!("/a/{id}.jpg"), date, 10, 10, MediaType::Photo);
            if let Some((lat, lon)) = pos {
                p.metadata.lat = Some(lat);
                p.metadata.lon = Some(lon);
            }
            p
        };
        let v = LibraryView::build(vec![
            at(1, "2024-05-01 10:00:00", Some((31.2, 121.5))), // in the box
            at(2, "2024-05-03 10:00:00", Some((31.3, 121.4))), // in the box
            at(3, "2024-05-02 10:00:00", Some((48.8, 2.3))),   // elsewhere
            at(4, "2024-05-02 12:00:00", None),                // no position, inside the span
            at(5, "2024-06-01 12:00:00", None),                // no position, after the span
            at(6, "", None),                                   // no position, no date
        ]);
        let ids = |order: Vec<u32>| -> Vec<i64> { order.iter().map(|&i| v.get(i).unwrap().info.id.id).collect() };
        let shanghai = Bbox { west: 121.0, south: 31.0, east: 122.0, north: 32.0 };
        let region = |start_ms, end_ms| Some(RegionFilter { bbox: shanghai, start_ms, end_ms, span_start_ms: None, span_end_ms: None });

        assert_eq!(ids(v.order(LibrarySort::Date, false, None, None, region(None, None))), vec![1, 4, 2]);
        // With the other filters on top.
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, Some(false), region(None, None))), vec![4]);
        // The map's time filter narrows the placed ones, and with them the span.
        let from = parse_date_ms("2024-05-02 00:00:00").map(|t| t as f64);
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, None, region(from, None))), vec![2]);
        // Nothing placed in the box: nothing comes along either.
        let empty = Bbox { west: 0.0, south: 0.0, east: 1.0, north: 1.0 };
        assert!(v.order(LibrarySort::Date, false, None, None, Some(RegionFilter { bbox: empty, start_ms: None, end_ms: None, span_start_ms: None, span_end_ms: None })).is_empty());
        // The whole world in view: the whole library, undated strays included.
        let world = Bbox { west: -180.0, south: -85.0, east: 180.0, north: 85.0 };
        let everything = v.order(LibrarySort::Date, false, None, None, Some(RegionFilter { bbox: world, start_ms: None, end_ms: None, span_start_ms: None, span_end_ms: None }));
        assert_eq!(everything.len(), 6);

        // What the list's timeline selects by default, and what it draws.
        let follow = RegionFilter { bbox: shanghai, start_ms: None, end_ms: None, span_start_ms: None, span_end_ms: None };
        let ms = |d: &str| parse_date_ms(d).unwrap();
        assert_eq!(v.region_span(&follow), Some((ms("2024-05-01 10:00:00"), ms("2024-05-03 10:00:00"))));
        assert_eq!(v.timestamps().len(), 5);
        // The list sets its own span: placed ones in the box are cut to it,
        // unplaced ones come from it, wherever the placed ones are in time.
        let own = |lo: &str, hi: &str| Some(RegionFilter {
            span_start_ms: Some(ms(lo) as f64),
            span_end_ms: Some(ms(hi) as f64),
            ..follow
        });
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, None, own("2024-05-02 00:00:00", "2024-06-30 00:00:00"))), vec![4, 2, 5]);
        // Even over the whole world, a span that is set is honoured.
        let world_span = Some(RegionFilter {
            bbox: world, start_ms: None, end_ms: None,
            span_start_ms: Some(ms("2024-05-01 00:00:00") as f64), span_end_ms: Some(ms("2024-05-01 23:00:00") as f64),
        });
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, None, world_span)), vec![1]);

        // A photo with no capture time takes its file's date: it then has a
        // place in time like the others, and a span can include it.
        let photos = vec![
            at(1, "2024-05-01 10:00:00", Some((31.2, 121.5))),
            at(2, "2024-05-03 10:00:00", Some((31.3, 121.4))),
            at(3, "2024-05-02 10:00:00", Some((48.8, 2.3))),
            at(6, "", None),
            at(7, "", None),
        ];
        let file_times = HashMap::from([(6, ms("2024-05-02 08:00:00")), (7, ms("2012-12-26 12:13:24"))]);
        let v = LibraryView::build_with_file_times(photos, &file_times);
        let ids = |order: Vec<u32>| -> Vec<i64> { order.iter().map(|&i| v.get(i).unwrap().info.id.id).collect() };
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, None, Some(follow))), vec![1, 6, 2]);
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, None, None)), vec![7, 1, 6, 3, 2]);
        assert_eq!(v.timestamps().len(), 5);
        assert!(v.get(v.order(LibrarySort::Date, false, None, None, None)[0]).unwrap().taken_from_file);
        let old = Some(RegionFilter {
            span_start_ms: Some(ms("2012-01-01 00:00:00") as f64),
            span_end_ms: Some(ms("2013-01-01 00:00:00") as f64),
            ..follow
        });
        assert_eq!(ids(v.order(LibrarySort::Date, false, None, None, old)), vec![7]);
    }
}
