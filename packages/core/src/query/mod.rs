//! In-memory query index for geotagged media.
//!
//! The frontend never holds the whole library. It asks this index for what is
//! on screen: clusters inside the viewport, timestamps for the timeline, and a
//! window of the time-ordered album for the viewer.
//!
//! Pure data + pure functions: no global state, no I/O, no platform code.

pub mod library;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::PhotoPreview;

/// Sentinel for entries without a parsable `date_taken`. Sorts before every real timestamp.
const NO_TS: i64 = i64::MIN;

/// MapLibre renders the world as 512px * 2^zoom.
const WORLD_PX: f64 = 512.0;
/// Cluster grid cell size in screen pixels (at integer zoom).
const CELL_PX: f64 = 64.0;
const MAX_ZOOM: i32 = 22;
/// Neighbouring clusters closer than this fraction of a cell are merged.
/// The largest marker is ~76px across and a cell is 64px, so this must exceed 1.
const MERGE_FACTOR: f64 = 1.2;

/// Parse a `date_taken` string into wall-clock milliseconds.
///
/// Accepts `YYYY-MM-DD HH:MM:SS`, `YYYY:MM:DD HH:MM:SS`, `YYYY-MM-DDTHH:MM:SS`
/// with optional fractional seconds / zone suffix (both ignored), optional
/// surrounding quotes, or a bare date. No timezone conversion is applied: the
/// result is the instant the wall clock would show if it were UTC.
pub fn parse_date_ms(raw: &str) -> Option<i64> {
    let s = raw.trim().trim_matches('"').trim();
    let b = s.as_bytes();
    if b.len() < 10 {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> {
        let part = b.get(from..to)?;
        if part.is_empty() || !part.iter().all(|c| c.is_ascii_digit()) {
            return None;
        }
        std::str::from_utf8(part).ok()?.parse::<i64>().ok()
    };
    let year = num(0, 4)?;
    let month = num(5, 7)?;
    let day = num(8, 10)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 1 {
        return None;
    }
    let (mut h, mut mi, mut sec) = (0i64, 0i64, 0i64);
    if b.len() >= 19 && (b[10] == b' ' || b[10] == b'T') {
        h = num(11, 13)?;
        mi = num(14, 16)?;
        sec = num(17, 19)?;
        if h > 23 || mi > 59 || sec > 60 {
            return None;
        }
    }
    Some((days_from_civil(year, month, day) * 86_400 + h * 3_600 + mi * 60 + sec) * 1_000)
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn mercator_x(lon: f64) -> f64 {
    (lon + 180.0) / 360.0
}

fn mercator_y(lat: f64) -> f64 {
    let lat = lat.clamp(-85.051_128, 85.051_128).to_radians();
    (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0
}

/// Geographic bounding box. `west > east` means the box crosses the antimeridian.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bbox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl Bbox {
    pub(crate) fn contains(&self, lat: f64, lon: f64) -> bool {
        if lat < self.south || lat > self.north {
            return false;
        }
        if self.east - self.west >= 360.0 {
            return true;
        }
        if self.west <= self.east {
            lon >= self.west && lon <= self.east
        } else {
            lon >= self.west || lon <= self.east
        }
    }
}

/// A cluster or a single item to draw on the map.
#[derive(Debug, Clone, Serialize)]
pub struct MapFeature {
    /// Stable key: `c{zoom}_{cx}_{cy}` for clusters, `p{id}` for single items.
    pub key: String,
    pub lat: f64,
    pub lon: f64,
    pub count: u32,
    /// Representative item (the most recent one in the cluster).
    pub id: i64,
    pub path: String,
    pub thumb: String,
    pub media_type: String,
    pub duration_ms: u64,
    /// Extent of the members: `[west, south, east, north]`.
    pub bounds: [f64; 4],
}

#[derive(Debug, Clone, Serialize)]
pub struct MapQueryResult {
    pub features: Vec<MapFeature>,
    /// Items inside the box and time range (before clustering).
    pub matched: u32,
}

/// One entry of the time-ordered album.
#[derive(Debug, Clone, Serialize)]
pub struct AlbumItem {
    pub id: i64,
    pub path: String,
    pub thumb: String,
    pub lat: f64,
    pub lon: f64,
    pub date_taken: String,
    pub media_type: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumWindow {
    /// Index of `items[0]` in the full album.
    pub start: u32,
    /// Total number of items in the album.
    pub total: u32,
    pub items: Vec<AlbumItem>,
}

/// Columnar index of geotagged items, sorted by timestamp (undated first).
#[derive(Debug, Default)]
pub struct GeoIndex {
    ids: Vec<i64>,
    lat: Vec<f64>,
    lon: Vec<f64>,
    mx: Vec<f64>,
    my: Vec<f64>,
    ts: Vec<i64>,
    path: Vec<String>,
    thumb: Vec<String>,
    date: Vec<String>,
    media: Vec<String>,
    dur: Vec<u64>,
    /// Number of leading entries without a timestamp.
    undated: usize,
    by_id: HashMap<i64, u32>,
}

impl GeoIndex {
    /// Build from preview rows. Rows without coordinates are skipped.
    pub fn build(rows: Vec<PhotoPreview>) -> Self {
        let mut tmp: Vec<(i64, PhotoPreview, f64, f64)> = rows
            .into_iter()
            .filter_map(|r| {
                let lat = r.lat?;
                let lon = r.lon?;
                if !lat.is_finite() || !lon.is_finite() || (lat == 0.0 && lon == 0.0) {
                    return None;
                }
                let ts = r
                    .date_taken
                    .as_deref()
                    .and_then(parse_date_ms)
                    .unwrap_or(NO_TS);
                Some((ts, r, lat, lon))
            })
            .collect();
        tmp.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.id.cmp(&b.1.id)));

        let n = tmp.len();
        let mut idx = GeoIndex {
            ids: Vec::with_capacity(n),
            lat: Vec::with_capacity(n),
            lon: Vec::with_capacity(n),
            mx: Vec::with_capacity(n),
            my: Vec::with_capacity(n),
            ts: Vec::with_capacity(n),
            path: Vec::with_capacity(n),
            thumb: Vec::with_capacity(n),
            date: Vec::with_capacity(n),
            media: Vec::with_capacity(n),
            dur: Vec::with_capacity(n),
            undated: 0,
            by_id: HashMap::with_capacity(n),
        };
        for (i, (ts, r, lat, lon)) in tmp.into_iter().enumerate() {
            if ts == NO_TS {
                idx.undated += 1;
            }
            idx.by_id.insert(r.id, i as u32);
            idx.ids.push(r.id);
            idx.lat.push(lat);
            idx.lon.push(lon);
            idx.mx.push(mercator_x(lon));
            idx.my.push(mercator_y(lat));
            idx.ts.push(ts);
            idx.thumb.push(r.thumb_path.unwrap_or_default());
            idx.path.push(r.path);
            idx.date.push(r.date_taken.unwrap_or_default());
            idx.media.push(r.media_type);
            idx.dur.push(r.duration_ms.unwrap_or(0));
        }
        idx
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// `[west, south, east, north]` of every item, or `None` when empty.
    pub fn extent(&self) -> Option<[f64; 4]> {
        if self.is_empty() {
            return None;
        }
        let mut e = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for i in 0..self.len() {
            e[0] = e[0].min(self.lon[i]);
            e[1] = e[1].min(self.lat[i]);
            e[2] = e[2].max(self.lon[i]);
            e[3] = e[3].max(self.lat[i]);
        }
        Some(e)
    }

    /// Index range of dated entries whose timestamp lies in `[start, end]`.
    fn dated_range(&self, start: Option<i64>, end: Option<i64>) -> (usize, usize) {
        let dated = &self.ts[self.undated..];
        let lo = match start {
            Some(s) => dated.partition_point(|&t| t < s),
            None => 0,
        };
        let hi = match end {
            Some(e) => dated.partition_point(|&t| t <= e),
            None => dated.len(),
        };
        (self.undated + lo, self.undated + hi.max(lo))
    }

    /// Cluster the items inside `bbox` (and the optional time range) on a
    /// world-aligned grid for the given zoom. Undated items always pass the
    /// time filter.
    pub fn query_map(
        &self,
        bbox: Bbox,
        zoom: f64,
        start_ms: Option<i64>,
        end_ms: Option<i64>,
    ) -> MapQueryResult {
        struct Acc {
            count: u32,
            sum_x: f64,
            sum_y: f64,
            rep: usize,
            bounds: [f64; 4],
        }

        let z = (zoom.floor() as i32).clamp(0, MAX_ZOOM);
        let cell = CELL_PX / (WORLD_PX * 2f64.powi(z));
        let mut cells: HashMap<(i64, i64), Acc> = HashMap::new();
        let mut matched = 0u32;

        let (lo, hi) = self.dated_range(start_ms, end_ms);
        let filtered = start_ms.is_some() || end_ms.is_some();
        let ranges: [(usize, usize); 2] = if filtered {
            [(0, self.undated), (lo, hi)]
        } else {
            [(0, self.len()), (0, 0)]
        };

        // Whole cells only: every cell the box touches is counted in full,
        // plus a margin of cells around it so that merging near the edge sees
        // the same neighbours it would in any other viewport. Otherwise a
        // cluster cut by the edge of the box changes its count and position
        // (and who it merges with) as the map is dragged.
        const MARGIN: i64 = 2;
        let cell_of = |v: f64| (v / cell).floor() as i64;
        let (y_a, y_b) = (cell_of(mercator_y(bbox.north)), cell_of(mercator_y(bbox.south)));
        let (cy_min, cy_max) = (y_a.min(y_b) - MARGIN, y_a.max(y_b) + MARGIN);
        let columns = (1.0 / cell).ceil() as i64;
        let cx_west = cell_of(mercator_x(bbox.west)) - MARGIN;
        let cx_east = cell_of(mercator_x(bbox.east)) + MARGIN;
        let whole_width = bbox.east - bbox.west >= 360.0
            || (bbox.west <= bbox.east && cx_east - cx_west + 1 >= columns);
        let in_columns = |cx: i64| {
            if whole_width {
                true
            } else if bbox.west <= bbox.east {
                // The margin may reach past the antimeridian on either side.
                (cx >= cx_west && cx <= cx_east)
                    || (cx + columns >= cx_west && cx + columns <= cx_east)
                    || (cx - columns >= cx_west && cx - columns <= cx_east)
            } else {
                cx >= cx_west || cx <= cx_east
            }
        };

        for (from, to) in ranges {
            for i in from..to {
                let (lat, lon) = (self.lat[i], self.lon[i]);
                let key = (cell_of(self.mx[i]), cell_of(self.my[i]));
                if key.1 < cy_min || key.1 > cy_max || !in_columns(key.0) {
                    continue;
                }
                if bbox.contains(lat, lon) {
                    matched += 1;
                }
                let acc = cells.entry(key).or_insert(Acc {
                    count: 0,
                    sum_x: 0.0,
                    sum_y: 0.0,
                    rep: i,
                    bounds: [lon, lat, lon, lat],
                });
                acc.count += 1;
                acc.sum_x += lon;
                acc.sum_y += lat;
                // Entries are time-ordered, so the last one seen is the most recent.
                if self.ts[i] >= self.ts[acc.rep] {
                    acc.rep = i;
                }
                acc.bounds[0] = acc.bounds[0].min(lon);
                acc.bounds[1] = acc.bounds[1].min(lat);
                acc.bounds[2] = acc.bounds[2].max(lon);
                acc.bounds[3] = acc.bounds[3].max(lat);
            }
        }

        let mut keyed: Vec<((i64, i64), Acc)> = cells.into_iter().collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0));

        // A grid splits groups that straddle a cell border, which shows up as
        // two markers on top of each other. Fold each small cluster into a
        // bigger neighbour whose centre is closer than a marker's width.
        // Largest first, so the result (and the surviving key) is deterministic.
        let centre = |a: &Acc| {
            let n = a.count as f64;
            (mercator_x(a.sum_x / n), mercator_y(a.sum_y / n))
        };
        let mut order: Vec<usize> = (0..keyed.len()).collect();
        order.sort_by(|&a, &b| keyed[b].1.count.cmp(&keyed[a].1.count).then(a.cmp(&b)));
        let slot: HashMap<(i64, i64), usize> =
            keyed.iter().enumerate().map(|(i, (k, _))| (*k, i)).collect();
        let mut absorbed = vec![false; keyed.len()];
        let merge_dist = cell * MERGE_FACTOR;
        for &i in &order {
            if absorbed[i] {
                continue;
            }
            let (cx, cy) = keyed[i].0;
            for dx in -1..=1i64 {
                for dy in -1..=1i64 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let Some(&j) = slot.get(&(cx + dx, cy + dy)) else { continue };
                    if absorbed[j] || keyed[j].1.count > keyed[i].1.count {
                        continue;
                    }
                    let (ax, ay) = centre(&keyed[i].1);
                    let (bx, by) = centre(&keyed[j].1);
                    if (ax - bx).hypot(ay - by) >= merge_dist {
                        continue;
                    }
                    absorbed[j] = true;
                    let (b_count, b_sx, b_sy, b_rep, b_bounds) = {
                        let b = &keyed[j].1;
                        (b.count, b.sum_x, b.sum_y, b.rep, b.bounds)
                    };
                    let a = &mut keyed[i].1;
                    a.count += b_count;
                    a.sum_x += b_sx;
                    a.sum_y += b_sy;
                    if self.ts[b_rep] > self.ts[a.rep]
                        || (self.ts[b_rep] == self.ts[a.rep] && b_rep > a.rep)
                    {
                        a.rep = b_rep;
                    }
                    a.bounds[0] = a.bounds[0].min(b_bounds[0]);
                    a.bounds[1] = a.bounds[1].min(b_bounds[1]);
                    a.bounds[2] = a.bounds[2].max(b_bounds[2]);
                    a.bounds[3] = a.bounds[3].max(b_bounds[3]);
                }
            }
        }
        let keyed: Vec<((i64, i64), Acc)> = keyed
            .into_iter()
            .zip(absorbed)
            .filter(|(_, gone)| !gone)
            .map(|(entry, _)| entry)
            .collect();

        let features = keyed
            .into_iter()
            .map(|((cx, cy), acc)| {
                let r = acc.rep;
                let single = acc.count == 1;
                MapFeature {
                    key: if single {
                        format!("p{}", self.ids[r])
                    } else {
                        format!("c{z}_{cx}_{cy}")
                    },
                    lat: if single { self.lat[r] } else { acc.sum_y / acc.count as f64 },
                    lon: if single { self.lon[r] } else { acc.sum_x / acc.count as f64 },
                    count: acc.count,
                    id: self.ids[r],
                    path: self.path[r].clone(),
                    thumb: self.thumb[r].clone(),
                    media_type: self.media[r].clone(),
                    duration_ms: self.dur[r],
                    bounds: acc.bounds,
                }
            })
            .collect();

        MapQueryResult { features, matched }
    }

    /// Ascending wall-clock timestamps (ms) of dated items, optionally limited to a box.
    pub fn timestamps(&self, bbox: Option<Bbox>) -> Vec<f64> {
        let range = self.undated..self.len();
        match bbox {
            None => self.ts[range].iter().map(|&t| t as f64).collect(),
            Some(b) => range
                .filter(|&i| b.contains(self.lat[i], self.lon[i]))
                .map(|i| self.ts[i] as f64)
                .collect(),
        }
    }

    /// A slice of the time-ordered album centred on `center_id` (or the newest
    /// item when it is unknown), `radius` items to each side.
    pub fn album_window(&self, center_id: Option<i64>, radius: u32) -> AlbumWindow {
        let total = self.len();
        if total == 0 {
            return AlbumWindow { start: 0, total: 0, items: Vec::new() };
        }
        let center = center_id
            .and_then(|id| self.by_id.get(&id).copied())
            .map(|i| i as usize)
            .unwrap_or(total - 1);
        let start = center.saturating_sub(radius as usize);
        let end = (center + radius as usize + 1).min(total);
        let items = (start..end).map(|i| self.album_item(i)).collect();
        AlbumWindow { start: start as u32, total: total as u32, items }
    }

    /// The time-ordered album of what is inside `bbox` (and the time range, if
    /// any; undated items always pass, as on the map). At most `limit` items:
    /// when more match, the newest are kept. `total` is how many matched.
    pub fn album_in(
        &self,
        bbox: Bbox,
        start_ms: Option<i64>,
        end_ms: Option<i64>,
        limit: u32,
    ) -> AlbumWindow {
        let (lo, hi) = self.dated_range(start_ms, end_ms);
        let filtered = start_ms.is_some() || end_ms.is_some();
        let ranges: [(usize, usize); 2] = if filtered {
            [(0, self.undated), (lo, hi)]
        } else {
            [(0, self.len()), (0, 0)]
        };
        let mut hits: Vec<usize> = Vec::new();
        for (from, to) in ranges {
            hits.extend((from..to).filter(|&i| bbox.contains(self.lat[i], self.lon[i])));
        }
        let total = hits.len();
        let start = total.saturating_sub(limit as usize);
        AlbumWindow {
            start: start as u32,
            total: total as u32,
            items: hits[start..].iter().map(|&i| self.album_item(i)).collect(),
        }
    }

    fn album_item(&self, i: usize) -> AlbumItem {
        AlbumItem {
            id: self.ids[i],
            path: self.path[i].clone(),
            thumb: self.thumb[i].clone(),
            lat: self.lat[i],
            lon: self.lon[i],
            date_taken: self.date[i].clone(),
            media_type: self.media[i].clone(),
            duration_ms: self.dur[i],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_in_keeps_newest_inside_the_box() {
        let rows = vec![
            row(1, 10.0, 10.0, Some("2024-01-01 00:00:00")),
            row(2, 10.1, 10.1, Some("2024-01-02 00:00:00")),
            row(3, 50.0, 50.0, Some("2024-01-03 00:00:00")),
            row(4, 10.2, 10.2, Some("2024-01-04 00:00:00")),
            row(5, 10.3, 10.3, None),
        ];
        let geo = GeoIndex::build(rows);
        let bbox = Bbox { west: 9.0, south: 9.0, east: 11.0, north: 11.0 };
        let all = geo.album_in(bbox, None, None, 100);
        assert_eq!(all.total, 4);
        assert_eq!(all.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![5, 1, 2, 4]);
        let newest = geo.album_in(bbox, None, None, 2);
        assert_eq!((newest.start, newest.total), (2, 4));
        assert_eq!(newest.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![2, 4]);
        let from = parse_date_ms("2024-01-02 00:00:00");
        let ranged = geo.album_in(bbox, from, None, 100);
        assert_eq!(ranged.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![5, 2, 4]);
    }

    fn row(id: i64, lat: f64, lon: f64, date: Option<&str>) -> PhotoPreview {
        PhotoPreview {
            id,
            path: format!("/p/{id}.jpg"),
            thumb_path: Some(format!("/t/{id}.jpg")),
            lat: Some(lat),
            lon: Some(lon),
            date_taken: date.map(|s| s.to_string()),
            file_size: 0,
            media_type: "photo".to_string(),
            duration_ms: None,
        }
    }

    const WORLD: Bbox = Bbox { west: -180.0, south: -85.0, east: 180.0, north: 85.0 };

    #[test]
    fn parses_supported_date_formats() {
        let expected = 1_704_103_200_000; // 2024-01-01 10:00:00
        assert_eq!(parse_date_ms("2024-01-01 10:00:00"), Some(expected));
        assert_eq!(parse_date_ms("2024:01:01 10:00:00"), Some(expected));
        assert_eq!(parse_date_ms("2024-01-01T10:00:00"), Some(expected));
        assert_eq!(parse_date_ms("\"2024-01-01 10:00:00\""), Some(expected));
        assert_eq!(parse_date_ms("2024-01-01T10:00:00.000000Z"), Some(expected));
        assert_eq!(parse_date_ms("2024-01-01"), Some(1_704_067_200_000));
        assert_eq!(parse_date_ms("1970-01-01 00:00:00"), Some(0));
        assert_eq!(parse_date_ms(""), None);
        assert_eq!(parse_date_ms("not a date"), None);
        assert_eq!(parse_date_ms("2024-13-01 10:00:00"), None);
    }

    #[test]
    fn skips_rows_without_coordinates_and_sorts_by_time() {
        let mut no_gps = row(9, 0.0, 0.0, Some("2024-01-01 00:00:00"));
        no_gps.lat = None;
        let idx = GeoIndex::build(vec![
            row(1, 10.0, 10.0, Some("2024-03-01 00:00:00")),
            no_gps,
            row(2, 10.0, 10.0, Some("2024-01-01 00:00:00")),
            row(3, 10.0, 10.0, None),
        ]);
        assert_eq!(idx.len(), 3);
        assert_eq!(idx.ids, vec![3, 2, 1]);
        assert_eq!(idx.timestamps(None).len(), 2);
    }

    #[test]
    fn clusters_nearby_points_and_separates_when_zoomed_in() {
        let idx = GeoIndex::build(vec![
            row(1, 48.8566, 2.3522, Some("2024-01-01 00:00:00")),
            row(2, 48.8570, 2.3530, Some("2024-01-02 00:00:00")),
            row(3, 35.6762, 139.6503, Some("2024-01-03 00:00:00")),
        ]);
        let low = idx.query_map(WORLD, 2.0, None, None);
        assert_eq!(low.matched, 3);
        assert_eq!(low.features.len(), 2);
        let paris = low.features.iter().find(|f| f.count == 2).unwrap();
        assert_eq!(paris.id, 2, "representative is the most recent member");
        assert!(paris.key.starts_with("c2_"));
        assert!(paris.bounds[0] <= 2.3522 && paris.bounds[2] >= 2.3530);

        let high = idx.query_map(WORLD, 20.0, None, None);
        assert_eq!(high.features.len(), 3);
        assert!(high.features.iter().all(|f| f.count == 1 && f.key.starts_with('p')));
    }

    #[test]
    fn merges_neighbours_split_by_a_cell_border() {
        // Find a longitude pair that is close together yet lands in two cells.
        let z = 6.0;
        let cell = CELL_PX / (WORLD_PX * 2f64.powi(6));
        let border_lon = (mercator_x(10.0) / cell).ceil() * cell * 360.0 - 180.0;
        let eps = cell * 360.0 * 0.05;
        let idx = GeoIndex::build(vec![
            row(1, 20.0, border_lon - eps, Some("2024-01-01 00:00:00")),
            row(2, 20.0, border_lon - eps, Some("2024-01-02 00:00:00")),
            row(3, 20.0, border_lon + eps, Some("2024-01-03 00:00:00")),
        ]);
        let r = idx.query_map(WORLD, z, None, None);
        assert_eq!(r.features.len(), 1, "close neighbours across a border become one marker");
        assert_eq!(r.features[0].count, 3);
        assert_eq!(r.features[0].id, 3, "representative is still the most recent");
        assert_eq!(r.matched, 3);
    }

    #[test]
    fn cluster_keys_are_stable_across_viewports() {
        let idx = GeoIndex::build(vec![
            row(1, 48.8566, 2.3522, Some("2024-01-01 00:00:00")),
            row(2, 48.8570, 2.3530, Some("2024-01-02 00:00:00")),
        ]);
        let a = idx.query_map(WORLD, 3.4, None, None);
        let b = idx.query_map(
            Bbox { west: 0.0, south: 40.0, east: 10.0, north: 55.0 },
            3.9,
            None,
            None,
        );
        assert_eq!(a.features[0].key, b.features[0].key);
    }

    #[test]
    fn a_cluster_cut_by_the_viewport_edge_is_still_whole() {
        // Two photos 0.02 degrees apart: one cluster at zoom 5.
        let idx = GeoIndex::build(vec![
            row(1, 48.85, 2.34, Some("2024-01-01 00:00:00")),
            row(2, 48.85, 2.36, Some("2024-01-02 00:00:00")),
        ]);
        let whole = idx.query_map(WORLD, 5.0, None, None);
        assert_eq!(whole.features.len(), 1);
        // The box ends between the two photos.
        let cut = idx.query_map(Bbox { west: 2.35, south: 40.0, east: 10.0, north: 55.0 }, 5.0, None, None);
        assert_eq!(cut.features.len(), 1);
        assert_eq!(cut.features[0].key, whole.features[0].key);
        assert_eq!(cut.features[0].count, 2);
        assert_eq!(cut.features[0].lon, whole.features[0].lon);
        assert_eq!(cut.matched, 1);
        // Far away, nothing.
        let away = idx.query_map(Bbox { west: 100.0, south: 0.0, east: 120.0, north: 20.0 }, 5.0, None, None);
        assert!(away.features.is_empty());
    }

    #[test]
    fn filters_by_viewport_and_time_and_keeps_undated() {
        let idx = GeoIndex::build(vec![
            row(1, 10.0, 10.0, Some("2024-01-01 00:00:00")),
            row(2, 10.0, 10.0, Some("2024-06-01 00:00:00")),
            row(3, 10.0, 10.0, None),
            row(4, -30.0, 100.0, Some("2024-06-01 00:00:00")),
        ]);
        let box_a = Bbox { west: 0.0, south: 0.0, east: 20.0, north: 20.0 };
        assert_eq!(idx.query_map(box_a, 5.0, None, None).matched, 3);

        let start = parse_date_ms("2024-05-01 00:00:00");
        let end = parse_date_ms("2024-07-01 00:00:00");
        let r = idx.query_map(box_a, 5.0, start, end);
        assert_eq!(r.matched, 2, "one dated match plus the undated item");
        assert_eq!(idx.timestamps(Some(box_a)).len(), 2);
    }

    #[test]
    fn bbox_handles_antimeridian() {
        let idx = GeoIndex::build(vec![
            row(1, 0.0, 179.0, Some("2024-01-01 00:00:00")),
            row(2, 0.0, -179.0, Some("2024-01-01 00:00:00")),
            row(3, 0.0, 0.0001, Some("2024-01-01 00:00:00")),
        ]);
        let wrap = Bbox { west: 170.0, south: -10.0, east: -170.0, north: 10.0 };
        assert_eq!(idx.query_map(wrap, 10.0, None, None).matched, 2);
    }

    #[test]
    fn album_window_is_time_ordered_and_clamped() {
        let rows = (0..10)
            .map(|i| row(i, 1.0, 1.0, Some(&format!("2024-01-{:02} 00:00:00", i + 1))))
            .collect();
        let idx = GeoIndex::build(rows);
        let w = idx.album_window(Some(5), 2);
        assert_eq!((w.start, w.total), (3, 10));
        assert_eq!(w.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![3, 4, 5, 6, 7]);
        let edge = idx.album_window(Some(0), 3);
        assert_eq!(edge.start, 0);
        assert_eq!(edge.items.len(), 4);
        let unknown = idx.album_window(Some(999), 1);
        assert_eq!(unknown.items.last().unwrap().id, 9);
    }

    #[test]
    fn handles_large_library_quickly() {
        let rows = (0..200_000)
            .map(|i| {
                let lat = -60.0 + (i % 1200) as f64 * 0.1;
                let lon = -170.0 + (i % 3400) as f64 * 0.1;
                row(i, lat, lon, Some("2024-01-01 00:00:00"))
            })
            .collect();
        let idx = GeoIndex::build(rows);
        let started = std::time::Instant::now();
        let r = idx.query_map(WORLD, 3.0, None, None);
        assert_eq!(r.matched, 200_000);
        assert!(r.features.len() < 5_000);
        assert!(started.elapsed().as_millis() < 500, "took {:?}", started.elapsed());
    }
}
