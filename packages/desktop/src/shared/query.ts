/**
 * Query-driven data access.
 *
 * The UI never holds the whole library. It asks the Rust side for what is on
 * screen: clusters inside the viewport, sorted timestamps for the timeline,
 * and a window of the time-ordered album for the viewer.
 *
 * Times are "wall-clock milliseconds": the instant the photo's local clock
 * reading would be if it were UTC. Format them with `timeZone: "UTC"`.
 */

import { invoke } from "@tauri-apps/api/core";
import type { PhotoInfo, SortBy } from "./types";

export interface Bbox {
    west: number;
    south: number;
    east: number;
    north: number;
}

/** A cluster (count > 1) or a single item (count === 1) to draw on the map. */
export interface MapFeature {
    /** Stable key: `c{zoom}_{x}_{y}` for clusters, `p{id}` for single items. */
    key: string;
    lat: number;
    lon: number;
    count: number;
    /** Representative item: the most recent one in the cluster. */
    id: number;
    path: string;
    thumb: string;
    media_type: string;
    duration_ms: number;
    /** Extent of the members: [west, south, east, north]. */
    bounds: [number, number, number, number];
}

export interface MapQueryResponse {
    features: MapFeature[];
    /** Items inside the box and time range, before clustering. */
    matched: number;
    /** Geotagged items in the whole library. */
    total: number;
    /** Changes whenever the library changes. */
    version: string;
}

export interface GeoSummary {
    total: number;
    extent: [number, number, number, number] | null;
    version: string;
}

export interface AlbumItem {
    id: number;
    path: string;
    thumb: string;
    lat: number;
    lon: number;
    date_taken: string;
    media_type: string;
    duration_ms: number;
}

export interface AlbumWindow {
    /** Index of items[0] in the full album. */
    start: number;
    total: number;
    items: AlbumItem[];
}

export function queryMap(
    bbox: Bbox,
    zoom: number,
    startMs: number | null,
    endMs: number | null,
): Promise<MapQueryResponse> {
    return invoke("query_map", { bbox, zoom, startMs, endMs });
}

export function geoSummary(): Promise<GeoSummary> {
    return invoke("geo_summary");
}

/** Ascending wall-clock timestamps of dated items, optionally limited to a box. */
export async function queryTimeline(bbox: Bbox | null): Promise<Float64Array> {
    const raw = await invoke<ArrayBuffer | number[]>("query_timeline", { bbox });
    if (raw instanceof ArrayBuffer) {
        return new Float64Array(raw.slice(0, raw.byteLength - (raw.byteLength % 8)));
    }
    return new Float64Array(new Uint8Array(raw).buffer);
}

/** Ascending wall-clock timestamps of every dated item in the library list. */
export async function libraryTimeline(): Promise<Float64Array> {
    const raw = await invoke<ArrayBuffer | number[]>("library_timeline");
    if (raw instanceof ArrayBuffer) {
        return new Float64Array(raw.slice(0, raw.byteLength - (raw.byteLength % 8)));
    }
    return new Float64Array(new Uint8Array(raw).buffer);
}

export function queryAlbum(centerId: number | null, radius: number): Promise<AlbumWindow> {
    return invoke("query_album", { centerId, radius });
}

/** The time-ordered album of what is inside `bbox` and the time range; the newest `limit` when more match. */
export function queryAlbumIn(
    bbox: Bbox,
    startMs: number | null,
    endMs: number | null,
    limit: number,
): Promise<AlbumWindow> {
    return invoke("query_album_in", { bbox, startMs, endMs, limit });
}

export type MediaFilter = "all" | "photo" | "video" | "live_photo";

/** What the map is showing: its viewport and, when set, its time filter. */
export interface MapViewState {
    bbox: Bbox;
    startMs: number | null;
    endMs: number | null;
}

/** What the library list is limited to: the map's view, and the list's own span of time when it has one. */
export interface LibraryRegion extends MapViewState {
    spanStartMs: number | null;
    spanEndMs: number | null;
}

/** Whether an item has a position: any, only with one, only without. */
export type LocationFilter = "all" | "located" | "unlocated";

/** A library entry: a file with its paired RAW folded in. */
export interface LibraryItem extends PhotoInfo {
    rawId?: number | null;
    /** Wall-clock ms: the capture time, or the file's date when it records none. */
    takenMs?: number | null;
    /** `takenMs` is the file's date, not a capture time. */
    takenFromFile?: boolean;
    rawSize?: number;
}

export interface LibraryPage {
    /** Entries matching the filter. */
    total: number;
    /** Entries after RAW+JPEG grouping, before the filter. */
    grouped_total: number;
    /** Rows in the database. */
    files_total: number;
    offset: number;
    items: LibraryItem[];
    /** `[start, end]` (ms) covered by the placed items the map shows. */
    region_span: [number, number] | null;
}

/** One page of the library list; grouping, filtering and sorting happen in Rust. */
export function queryLibrary(
    sortBy: SortBy,
    descending: boolean,
    mediaFilter: MediaFilter,
    locationFilter: LocationFilter,
    /** Only what the map is showing, plus unplaced items from the same span of time. */
    region: LibraryRegion | null,
    offset: number,
    limit: number,
): Promise<LibraryPage> {
    return invoke("query_library", {
        sortBy,
        descending,
        mediaFilter: mediaFilter === "all" ? null : mediaFilter,
        located: locationFilter === "all" ? null : locationFilter === "located",
        region,
        offset,
        limit,
    });
}

/** Full details of one item (a paired RAW row resolves to its JPEG). */
export function photoDetail(id: number): Promise<LibraryItem | null> {
    return invoke("photo_detail", { id });
}

export function photoCount(): Promise<number> {
    return invoke("photo_count");
}

/** Viewport box grown by `pad` (fraction of its size) so short pans need no refetch. */
export function paddedBbox(
    west: number,
    south: number,
    east: number,
    north: number,
    pad = 0.5,
): Bbox {
    const w = east - west;
    const h = north - south;
    if (w >= 360 / (1 + 2 * pad)) {
        return {
            west: -180,
            east: 180,
            south: Math.max(-90, south - h * pad),
            north: Math.min(90, north + h * pad),
        };
    }
    const wrap = (lon: number) => ((((lon + 180) % 360) + 360) % 360) - 180;
    return {
        west: wrap(west - w * pad),
        east: wrap(east + w * pad),
        south: Math.max(-90, south - h * pad),
        north: Math.min(90, north + h * pad),
    };
}

/** Shape the album item the way the viewers expect a photo. */
export function albumItemToPhoto(item: AlbumItem): PhotoInfo {
    return {
        id: { id: item.id },
        path: item.path,
        thumb_path: item.thumb,
        hash: "",
        file_size: 0,
        media_type: (item.media_type || "photo") as PhotoInfo["media_type"],
        metadata: {
            lat: item.lat,
            lon: item.lon,
            date_taken: item.date_taken || undefined,
            width: 0,
            height: 0,
            duration_ms: item.duration_ms || undefined,
        },
    };
}

/** Compact count label for cluster badges. */
export function abbreviateCount(n: number): string {
    if (n < 1000) return String(n);
    if (n < 10000) return (n / 1000).toFixed(1).replace(/\.0$/, "") + "k";
    return Math.round(n / 1000) + "k";
}
