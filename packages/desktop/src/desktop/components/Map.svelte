<script lang="ts">
    import { onMount, onDestroy, untrack } from "svelte";
    import maplibregl from "maplibre-gl";
    import "maplibre-gl/dist/maplibre-gl.css";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import TimelineSlider from "./TimelineSlider.svelte";
    import { baseMapStyle } from "../../shared/map-style";
    import {
        queryMap,
        geoSummary,
        queryTimeline,
        photoDetail,
        paddedBbox,
        abbreviateCount,
        type Bbox,
        type MapFeature,
    } from "../../shared/query";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";

    let { refreshKey = 0, libraryCount = 0, onOpenPreview, theme = "dark", tileConfig, t, locale = 'en' as Locale } = $props<{
        /** Bumped whenever the library changes (import progress, delete, clear). */
        refreshKey?: number;
        /** Items in the whole library, geotagged or not. */
        libraryCount?: number;
        onOpenPreview?: (item: { id: number; path: string }) => void;
        theme?: "dark" | "light";
        tileConfig?: { tiles: string[]; tileSize: number };
        t: Translations;
        locale?: Locale;
    }>();

    let mapContainer: HTMLDivElement;
    let map = $state<maplibregl.Map | null>(null);
    let mapLoaded = $state(false);
    let isReady = $state(false);

    // Geotagged items in the library (from the last query).
    let total = $state(0);
    let summaryLoaded = $state(false);
    let hasGeotaggedPhotos = $derived(total > 0);

    // Time filter (wall-clock ms). Set only by the user through the timeline;
    // map movement never writes it, so there is no map <-> timeline loop to guard.
    let timeFilterStart = $state<number | null>(null);
    let timeFilterEnd = $state<number | null>(null);

    // Sorted timestamps the timeline draws. Follows the map viewport until the
    // user picks a time range; then it stays put until the range is cleared.
    let timelineStamps = $state.raw<Float64Array>(new Float64Array(0));
    let mapConstrained = $state(false);

    // Box selection state
    let isBoxSelectMode = $state(false);
    let isDrawingBox = $state(false);
    let boxStart: { x: number; y: number } | null = null;
    let selectionBox: HTMLDivElement | null = null;

    // Hover popup
    let hoverPopup: maplibregl.Popup | null = null;
    let hoveredPhotoPath: string | null = null;

    // Resize observer
    let resizeObserver: ResizeObserver | null = null;

    // Drag/zoom state — suppress mousemove during active interaction
    let isDragging = false;
    let mouseMoveRafPending = false;

    function handleDragStart() { isDragging = true; hideHoverPopup(); }
    function handleDragEnd() { isDragging = false; }

    function handleTimeRangeChange(start: number | null, end: number | null) {
        if (start === timeFilterStart && end === timeFilterEnd) return;
        timeFilterStart = start;
        timeFilterEnd = end;
        refresh();
        if (start === null) scheduleTimelineSync();
    }

    function formatTimeRange(start: number | null, end: number | null): string {
        if (start === null || end === null) return '';
        const a = new Date(start);
        const b = new Date(end);
        const sameDay = a.toISOString().slice(0, 10) === b.toISOString().slice(0, 10);
        const formatDate = (d: Date) => d.toLocaleDateString(locale, {
            month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC'
        });
        const formatTime = (d: Date) => d.toLocaleTimeString(locale, {
            hour: '2-digit', minute: '2-digit', timeZone: 'UTC'
        });
        if (sameDay) {
            return `${formatDate(a)} ${formatTime(a)} - ${formatTime(b)}`;
        }
        return `${formatDate(a)} - ${formatDate(b)}`;
    }

    // The pill at the top: the range the user picked, or else the span of
    // what is in view (the timeline's data). Display only; it filters nothing.
    let timeRangeDisplay = $derived.by(() => {
        if (timeFilterStart !== null && timeFilterEnd !== null) {
            return formatTimeRange(timeFilterStart, timeFilterEnd);
        }
        if (timelineStamps.length === 0) return '';
        return formatTimeRange(timelineStamps[0], timelineStamps[timelineStamps.length - 1]);
    });

    function getThumbnailUrl(path: string): string {
        return convertFileSrc(path);
    }

    // ============================================================
    // GeoJSON Clustering Implementation
    // ============================================================

    const SOURCE_ID = 'photos-source';
    const CLUSTER_LAYER_ID = 'photo-clusters';
    const CLUSTER_THUMB_LAYER_ID = 'photo-cluster-thumb';
    const CLUSTER_COUNT_LAYER_ID = 'photo-cluster-count';
    const UNCLUSTERED_LAYER_ID = 'photo-unclustered';
    const UNCLUSTERED_BORDER_LAYER_ID = 'photo-unclustered-border';
    const UNCLUSTERED_THUMB_LAYER_ID = 'photo-unclustered-thumb';

    // Thumbnail rendering constants
    const THUMB_CANVAS_SIZE = 88;    // 88px canvas -> 44px display at pixelRatio 2
    const THUMB_BORDER_WIDTH = 4;    // Border width in canvas pixels
    const THUMB_PIXEL_RATIO = 2;

    const IS_CLUSTER: any = ['>', ['get', 'count'], 1];
    const IS_SINGLE: any = ['==', ['get', 'count'], 1];

    // Image ids are derived from the thumbnail path, so they survive re-queries:
    // `thumb:<path>` (bordered, single item) and `cthumb:<path>` (borderless, cluster).
    let loadedImages = new Set<string>();
    let loadingImages = new Set<string>();
    // LRU of rendered ImageData. Map insertion order = oldest first.
    let thumbDataCache = new Map<string, ImageData>();
    const THUMB_CACHE_MAX = 200;   // Desktop has more headroom than mobile
    const ATLAS_BUDGET = 300;      // Max images in MapLibre GPU atlas

    // What is currently in the source.
    let currentFeatures: MapFeature[] = [];
    let currentImageIds = new Set<string>();
    let lastFingerprint = '';

    function thumbCacheGet(key: string): ImageData | undefined {
        const val = thumbDataCache.get(key);
        if (val !== undefined) {
            thumbDataCache.delete(key);
            thumbDataCache.set(key, val);
        }
        return val;
    }

    function thumbCacheSet(key: string, value: ImageData) {
        if (thumbDataCache.has(key)) thumbDataCache.delete(key);
        thumbDataCache.set(key, value);
        while (thumbDataCache.size > THUMB_CACHE_MAX) {
            const oldest = thumbDataCache.keys().next().value;
            if (oldest !== undefined) thumbDataCache.delete(oldest);
            else break;
        }
    }

    /** Drop atlas images that no current feature uses, down to the budget. */
    function evictUnusedImages() {
        if (!map) return;
        if (loadedImages.size <= ATLAS_BUDGET) return;
        let removed = 0;
        const target = loadedImages.size - ATLAS_BUDGET;
        for (const imageId of loadedImages) {
            if (removed >= target) break;
            if (currentImageIds.has(imageId)) continue;
            try {
                if (map.hasImage(imageId)) map.removeImage(imageId);
            } catch (_) { /* ignore */ }
            loadedImages.delete(imageId);
            removed++;
        }
    }

    function imageIdFor(f: MapFeature): string {
        if (!f.thumb) return '';
        return (f.count > 1 ? 'cthumb:' : 'thumb:') + f.thumb;
    }

    function setupClusterLayers() {
        if (!map) return;
        if (map.getSource(SOURCE_ID)) return;

        // Plain (non-clustering) source: Rust already clustered the viewport.
        map.addSource(SOURCE_ID, {
            type: 'geojson',
            data: { type: 'FeatureCollection', features: [] },
        });

        const isDark = theme === 'dark';

        // Cluster border circle. Radius = thumbnail apparent radius + 3px.
        map.addLayer({
            id: CLUSTER_LAYER_ID,
            type: 'circle',
            source: SOURCE_ID,
            filter: IS_CLUSTER,
            paint: {
                'circle-radius': ['step', ['get', 'count'], 25, 10, 28, 50, 32, 100, 35, 500, 38],
                'circle-color': isDark ? '#ffffff' : '#1e293b',
                'circle-stroke-width': 0,
            }
        });

        // Cluster thumbnail: the most recent photo in the cluster.
        map.addLayer({
            id: CLUSTER_THUMB_LAYER_ID,
            type: 'symbol',
            source: SOURCE_ID,
            filter: IS_CLUSTER,
            layout: {
                'icon-image': ['get', 'imageId'],
                'icon-size': ['step', ['get', 'count'], 1.0, 10, 1.15, 50, 1.3, 100, 1.45, 500, 1.6],
                'icon-allow-overlap': true,
                'icon-ignore-placement': true,
            }
        });

        map.addLayer({
            id: CLUSTER_COUNT_LAYER_ID,
            type: 'symbol',
            source: SOURCE_ID,
            filter: IS_CLUSTER,
            layout: {
                'text-field': ['get', 'label'],
                'text-font': ['Open Sans Semibold'],
                'text-size': ['step', ['get', 'count'], 13, 10, 14, 100, 15, 500, 16],
                'text-allow-overlap': true,
            },
            paint: {
                'text-color': '#ffffff',
                'text-halo-color': 'rgba(0, 0, 0, 0.65)',
                'text-halo-width': 1.5,
            }
        });

        map.addLayer({
            id: UNCLUSTERED_BORDER_LAYER_ID,
            type: 'circle',
            source: SOURCE_ID,
            filter: IS_SINGLE,
            paint: {
                'circle-radius': 22,
                'circle-color': isDark ? '#ffffff' : '#1e293b',
                'circle-stroke-width': 0,
            }
        });

        map.addLayer({
            id: UNCLUSTERED_LAYER_ID,
            type: 'circle',
            source: SOURCE_ID,
            filter: IS_SINGLE,
            paint: {
                'circle-radius': 20,
                'circle-color': isDark ? 'rgba(99, 102, 241, 0.9)' : 'rgba(79, 70, 229, 0.9)',
                'circle-stroke-width': 2,
                'circle-stroke-color': isDark ? '#ffffff' : '#1e293b',
            }
        });

        // Circular photo thumbnail; the circle layers are the placeholder until it loads.
        map.addLayer({
            id: UNCLUSTERED_THUMB_LAYER_ID,
            type: 'symbol',
            source: SOURCE_ID,
            filter: IS_SINGLE,
            layout: {
                'icon-image': ['get', 'imageId'],
                'icon-size': 1,
                'icon-allow-overlap': true,
                'icon-ignore-placement': true,
            }
        });
    }

    // ============================================================
    // Viewport query
    // ============================================================

    let querySeq = 0;
    let lastVersion = '';

    // Counters for the agent self-test (shared/agent-test.ts).
    const stats = { queries: 0, setData: 0, lastQueryMs: 0 };

    // One query at a time. Requests that arrive meanwhile collapse into a
    // single follow-up, so a fast drag never queues up stale work.
    let queryInFlight = false;
    let queryQueued = false;

    /** Ask Rust for the clusters in (a padded) viewport and draw them. */
    async function refresh() {
        if (queryInFlight) {
            queryQueued = true;
            return;
        }
        queryInFlight = true;
        try {
            await runQuery();
        } finally {
            queryInFlight = false;
            if (queryQueued) {
                queryQueued = false;
                refresh();
            }
        }
    }

    async function runQuery() {
        const m = map;
        if (!m || !mapLoaded) return;
        const seq = ++querySeq;
        const b = m.getBounds();
        const bbox = paddedBbox(b.getWest(), b.getSouth(), b.getEast(), b.getNorth());
        try {
            const t0 = performance.now();
            const res = await queryMap(bbox, m.getZoom(), timeFilterStart, timeFilterEnd);
            stats.queries++;
            stats.lastQueryMs = Math.round((performance.now() - t0) * 10) / 10;
            if (seq !== querySeq || map !== m) return;
            const becameNonEmpty = total === 0 && res.total > 0;
            total = res.total;
            summaryLoaded = true;
            applyFeatures(res.features);
            if (res.version !== lastVersion) {
                lastVersion = res.version;
                // Library changed. Frame everything only the first time photos
                // appear; later imports must not steal the camera.
                if (becameNonEmpty) fitAll(true);
                loadStamps(timelineBbox);
            }
        } catch (e) {
            console.error("[Map] query_map failed", e);
        }
    }

    function applyFeatures(features: MapFeature[]) {
        if (!map) return;
        const source = map.getSource(SOURCE_ID) as maplibregl.GeoJSONSource | undefined;
        if (!source) return;

        currentFeatures = features;
        const ids = new Set<string>();
        let fingerprint = '';
        const out: GeoJSON.Feature[] = new Array(features.length);
        for (let i = 0; i < features.length; i++) {
            const f = features[i];
            const imageId = imageIdFor(f);
            if (imageId) ids.add(imageId);
            fingerprint += f.key + ':' + f.count + ':' + f.id + '|';
            out[i] = {
                type: 'Feature',
                geometry: { type: 'Point', coordinates: [f.lon, f.lat] },
                properties: {
                    key: f.key,
                    count: f.count,
                    label: abbreviateCount(f.count),
                    id: f.id,
                    path: f.path,
                    thumb: f.thumb,
                    imageId,
                    w: f.bounds[0],
                    s: f.bounds[1],
                    e: f.bounds[2],
                    n: f.bounds[3],
                },
            };
        }
        currentImageIds = ids;
        if (fingerprint === lastFingerprint) return;
        lastFingerprint = fingerprint;
        stats.setData++;
        source.setData({ type: 'FeatureCollection', features: out });
    }

    // Re-query while the camera moves so markers fill in at the edges.
    const MOVE_QUERY_INTERVAL = 120;
    let moveTimer: ReturnType<typeof setTimeout> | null = null;
    let lastMoveQuery = 0;

    function handleMove() {
        const now = performance.now();
        if (now - lastMoveQuery >= MOVE_QUERY_INTERVAL) {
            if (moveTimer) {
                clearTimeout(moveTimer);
                moveTimer = null;
            }
            lastMoveQuery = now;
            refresh();
        } else if (!moveTimer) {
            moveTimer = setTimeout(() => {
                moveTimer = null;
                lastMoveQuery = performance.now();
                refresh();
            }, MOVE_QUERY_INTERVAL);
        }
    }

    function handleMoveEnd() {
        refresh();
        scheduleTimelineSync();
    }

    // ============================================================
    // Timeline data
    // ============================================================

    let timelineBbox: Bbox | null = null;
    let stampSeq = 0;
    let timelineSyncTimer: ReturnType<typeof setTimeout> | null = null;

    async function loadStamps(bbox: Bbox | null) {
        const seq = ++stampSeq;
        try {
            const stamps = await queryTimeline(bbox);
            if (seq !== stampSeq) return;
            // An empty viewport keeps the previous timeline instead of blanking it.
            if (stamps.length === 0 && bbox !== null && timelineStamps.length > 0) return;
            timelineBbox = bbox;
            timelineStamps = stamps;
            mapConstrained = bbox !== null && stamps.length < total;
        } catch (e) {
            console.error("[Map] query_timeline failed", e);
        }
    }

    /** After the map settles, show the timeline of what is in view (unless the user set a range). */
    function scheduleTimelineSync() {
        if (timelineSyncTimer) clearTimeout(timelineSyncTimer);
        timelineSyncTimer = setTimeout(() => {
            timelineSyncTimer = null;
            if (!map || timeFilterStart !== null) return;
            const b = map.getBounds();
            loadStamps(paddedBbox(b.getWest(), b.getSouth(), b.getEast(), b.getNorth(), 0));
        }, 300);
    }

    // ============================================================
    // Thumbnails
    // ============================================================

    function drawCircularThumbnail(img: HTMLImageElement, borderWidth: number = THUMB_BORDER_WIDTH): ImageData {
        const size = THUMB_CANVAS_SIZE;
        const radius = size / 2;
        const inset = borderWidth;

        const canvas = document.createElement('canvas');
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext('2d')!;

        ctx.save();
        ctx.beginPath();
        ctx.arc(radius, radius, radius - inset, 0, Math.PI * 2);
        ctx.clip();

        // Cover-crop the image into the circle
        const aspect = img.naturalWidth / img.naturalHeight;
        let sx = 0, sy = 0, sw = img.naturalWidth, sh = img.naturalHeight;
        if (aspect > 1) {
            sx = (img.naturalWidth - img.naturalHeight) / 2;
            sw = sh = img.naturalHeight;
        } else {
            sy = (img.naturalHeight - img.naturalWidth) / 2;
            sw = sh = img.naturalWidth;
        }
        ctx.drawImage(img, sx, sy, sw, sh, inset, inset, size - 2 * inset, size - 2 * inset);
        ctx.restore();

        if (borderWidth > 0) {
            ctx.beginPath();
            ctx.arc(radius, radius, radius - borderWidth / 2, 0, Math.PI * 2);
            ctx.strokeStyle = theme === 'dark' ? '#ffffff' : '#1e293b';
            ctx.lineWidth = borderWidth;
            ctx.stroke();
        }

        return ctx.getImageData(0, 0, size, size);
    }

    /** Put the image for `imageId` into the atlas, from cache or by decoding the file. */
    function loadImage(imageId: string) {
        if (!map || !imageId) return;
        if (loadingImages.has(imageId) || loadedImages.has(imageId)) return;
        if (map.hasImage(imageId)) { loadedImages.add(imageId); return; }

        const isCluster = imageId.startsWith('cthumb:');
        const thumbPath = imageId.slice(isCluster ? 7 : 6);
        const borderWidth = isCluster ? 0 : THUMB_BORDER_WIDTH;

        const cached = thumbCacheGet(imageId);
        if (cached) {
            try {
                map.addImage(imageId, cached, { pixelRatio: THUMB_PIXEL_RATIO });
                loadedImages.add(imageId);
            } catch (_) { /* ignore */ }
            return;
        }

        loadingImages.add(imageId);
        const img = new Image();
        img.crossOrigin = 'anonymous';
        img.onload = () => {
            loadingImages.delete(imageId);
            if (!map) return;
            if (map.hasImage(imageId)) { loadedImages.add(imageId); return; }
            try {
                const imageData = drawCircularThumbnail(img, borderWidth);
                thumbCacheSet(imageId, imageData);
                map.addImage(imageId, imageData, { pixelRatio: THUMB_PIXEL_RATIO });
                loadedImages.add(imageId);
            } catch (_) { /* ignore */ }
        };
        img.onerror = () => { loadingImages.delete(imageId); };
        img.src = getThumbnailUrl(thumbPath);
    }

    /** MapLibre met an icon that is not in the atlas yet: load it right away. */
    function handleMissingImage(e: { id: string }) {
        if (e.id.startsWith('thumb:') || e.id.startsWith('cthumb:')) {
            // An evicted image may still be marked as loaded.
            loadedImages.delete(e.id);
            loadImage(e.id);
        }
    }

    /** Safety net on idle: load anything still missing, then trim the atlas. */
    function handleIdle() {
        if (!map) return;
        let started = 0;
        for (const imageId of currentImageIds) {
            if (started >= 20) break;
            if (loadedImages.has(imageId) || loadingImages.has(imageId)) continue;
            loadImage(imageId);
            started++;
        }
        evictUnusedImages();
    }

    // ============================================================
    // Camera
    // ============================================================

    /** Frame every geotagged item. */
    async function fitAll(delay: boolean = false) {
        try {
            const summary = await geoSummary();
            if (!map || !summary.extent) return;
            const [w, s, e, n] = summary.extent;
            const doFit = () => {
                map?.fitBounds([[w, s], [e, n]], { padding: 100, maxZoom: 15 });
            };
            if (delay) setTimeout(doFit, 150);
            else doFit();
        } catch (e) {
            console.error("[Map] geo_summary failed", e);
        }
    }

    // ============================================================
    // Click & Hover Handlers
    // ============================================================

    /**
     * Click handler using queryRenderedFeatures for GPU layers.
     */
    function handleMapClick(e: maplibregl.MapMouseEvent) {
        if (!map || isBoxSelectMode) return;

        const tolerance = 10;
        const bbox: [maplibregl.PointLike, maplibregl.PointLike] = [
            [e.point.x - tolerance, e.point.y - tolerance],
            [e.point.x + tolerance, e.point.y + tolerance]
        ];

        const features = map.queryRenderedFeatures(bbox, {
            layers: [
                CLUSTER_THUMB_LAYER_ID, CLUSTER_LAYER_ID, CLUSTER_COUNT_LAYER_ID,
                UNCLUSTERED_THUMB_LAYER_ID, UNCLUSTERED_LAYER_ID, UNCLUSTERED_BORDER_LAYER_ID,
            ]
        });
        if (features.length === 0) return;

        let best = features[0];
        let bestDist = Infinity;
        for (const f of features) {
            const coords = (f.geometry as GeoJSON.Point).coordinates as [number, number];
            const projected = map.project(coords);
            const dx = projected.x - e.point.x;
            const dy = projected.y - e.point.y;
            const dist = dx * dx + dy * dy;
            if (dist < bestDist) {
                bestDist = dist;
                best = f;
            }
        }
        const p = best.properties as any;
        if (!p) return;

        // Above zoom 17 a cluster is photos taken at the same spot: open it instead of zooming.
        const degenerate = p.w === p.e && p.s === p.n;
        if (p.count > 1 && !degenerate && map.getZoom() < 17) {
            // Zoom to the extent of the cluster's members (computed in Rust).
            map.fitBounds([[p.w, p.s], [p.e, p.n]], {
                padding: 80,
                maxZoom: 18,
                duration: 300
            });
            return;
        }

        onOpenPreview?.({ id: p.id, path: p.path });
    }

    /**
     * Mouse move handler for hover popup + cursor change (desktop only).
     * Throttled via rAF and suppressed during drag/zoom to avoid GPU readback stalls.
     */
    function handleMouseMove(e: maplibregl.MapMouseEvent) {
        if (!map || isBoxSelectMode || isDragging) return;
        if (mouseMoveRafPending) return;

        const point = e.point;
        mouseMoveRafPending = true;
        requestAnimationFrame(() => {
            mouseMoveRafPending = false;
            if (!map || isDragging) return;

            const tolerance = 5;
            const bbox: [maplibregl.PointLike, maplibregl.PointLike] = [
                [point.x - tolerance, point.y - tolerance],
                [point.x + tolerance, point.y + tolerance]
            ];

            // Check clusters
            const clusters = map.queryRenderedFeatures(bbox, {
                layers: [CLUSTER_THUMB_LAYER_ID, CLUSTER_LAYER_ID, CLUSTER_COUNT_LAYER_ID]
            });
            if (clusters.length > 0) {
                map.getCanvas().style.cursor = 'pointer';
                hideHoverPopup();
                return;
            }

            // Check unclustered photos
            const photoFeatures = map.queryRenderedFeatures(bbox, {
                layers: [UNCLUSTERED_THUMB_LAYER_ID, UNCLUSTERED_LAYER_ID, UNCLUSTERED_BORDER_LAYER_ID]
            });
            if (photoFeatures.length > 0) {
                map.getCanvas().style.cursor = 'pointer';
                const feature = photoFeatures[0];
                const path = feature.properties?.path;
                if (path && path !== hoveredPhotoPath) {
                    showHoverPopup(feature);
                }
                return;
            }

            // No features under cursor
            map.getCanvas().style.cursor = '';
            hideHoverPopup();
        });
    }

    function escapeHtml(text: string): string {
        return text.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
    }

    function popupHtml(path: string, thumb: string, date: string, hasRaw: boolean): string {
        const filename = escapeHtml(path.split('/').pop() || '');
        const img = document.createElement('img');
        img.src = getThumbnailUrl(thumb || path);
        img.alt = filename;
        const dateInfo = date ? `<div class="popup-date">${escapeHtml(date)}</div>` : '';
        const rawBadge = hasRaw ? '<div class="popup-raw-badge">RAW</div>' : '';
        return `
            <div class="photo-popup">
                ${img.outerHTML}
                <div class="popup-info">
                    <div class="popup-filename">${filename}</div>
                    ${dateInfo}
                </div>
                ${rawBadge}
            </div>
        `;
    }

    function showHoverPopup(feature: maplibregl.GeoJSONFeature) {
        if (!map || !hoverPopup) return;
        const props = feature.properties as any;
        const path: string = props?.path;
        if (!path) return;
        hoveredPhotoPath = path;

        // Thumbnail and name come with the feature; date and RAW badge follow.
        const coords = (feature.geometry as GeoJSON.Point).coordinates as [number, number];
        hoverPopup.setLngLat(coords).setHTML(popupHtml(path, props.thumb, '', false)).addTo(map);

        photoDetail(props.id).then((detail) => {
            if (!detail || hoveredPhotoPath !== path || !hoverPopup) return;
            const date = detail.metadata?.date_taken?.replace(/"/g, '') || '';
            hoverPopup.setHTML(popupHtml(path, props.thumb, date, !!detail.hasRaw));
        }).catch(() => {});
    }

    function hideHoverPopup() {
        hoveredPhotoPath = null;
        if (hoverPopup) {
            hoverPopup.remove();
        }
    }

    function handleShowAllPhotos() {
        fitAll(false);
    }

    // ============================================================
    // Map Style
    // ============================================================

    function getMapStyle(isDark: boolean): maplibregl.StyleSpecification {
        return baseMapStyle(isDark, tileConfig);
    }

    // ============================================================
    // Map Setup
    // ============================================================

    onMount(() => {
        if (!mapContainer) return;
        map = new maplibregl.Map({
            container: mapContainer,
            style: getMapStyle(theme === 'dark'),
            center: [0, 20],
            zoom: 2,
            attributionControl: false,
            maxTileCacheSize: 200,
        });

        setupMapControls();

        (window as any).__footosMap = map;
        (window as any).__footosSetTimeFilter = handleTimeRangeChange;
        (window as any).__footosMapStats = () => ({
            ...stats,
            total,
            features: currentFeatures.length,
            stamps: timelineStamps.length,
            atlasImages: loadedImages.size,
        });
    });

    function setupMapControls() {
        if (!map) return;

        // Navigation control
        map.addControl(new maplibregl.NavigationControl(), 'top-right');

        // Box select control
        class BoxSelectControl {
            _container: HTMLDivElement | undefined;
            _button: HTMLButtonElement | undefined;

            onAdd() {
                this._container = document.createElement('div');
                this._container.className = 'maplibregl-ctrl maplibregl-ctrl-group';

                this._button = document.createElement('button');
                this._button.type = 'button';
                this._button.className = 'maplibregl-ctrl-box-select';
                this._button.title = t.map.boxSelect;
                this._button.innerHTML = '<i class="fa-solid fa-vector-square"></i>';
                this._button.onclick = (e) => {
                    e.stopPropagation();
                    toggleBoxSelectMode();
                    this._updateStyle();
                };

                this._container.appendChild(this._button);
                return this._container;
            }

            onRemove() {
                this._container?.parentNode?.removeChild(this._container);
            }

            _updateStyle() {
                if (this._button) {
                    if (isBoxSelectMode) {
                        this._button.classList.add('active');
                    } else {
                        this._button.classList.remove('active');
                    }
                }
            }

            update() {
                this._updateStyle();
            }
        }

        const boxSelectControl = new BoxSelectControl();
        map.addControl(boxSelectControl as any, 'top-right');
        (window as any).__boxSelectControl = boxSelectControl;

        // Show all photos control
        class ShowAllControl {
            _container: HTMLDivElement | undefined;

            onAdd() {
                this._container = document.createElement('div');
                this._container.className = 'maplibregl-ctrl maplibregl-ctrl-group';

                const button = document.createElement('button');
                button.type = 'button';
                button.className = 'maplibregl-ctrl-show-all';
                button.title = t.map.showAllPhotos;
                button.innerHTML = '<i class="fa-solid fa-expand"></i>';
                button.onclick = (e) => {
                    e.stopPropagation();
                    handleShowAllPhotos();
                };

                this._container.appendChild(button);
                return this._container;
            }

            onRemove() {
                this._container?.parentNode?.removeChild(this._container);
            }
        }

        map.addControl(new ShowAllControl() as any, 'top-right');

        map.addControl(new maplibregl.AttributionControl({ compact: true }), 'bottom-right');

        // Handle resize
        resizeObserver = new ResizeObserver(() => {
            map?.resize();
        });
        resizeObserver.observe(mapContainer);

        setTimeout(() => {
            map?.resize();
        }, 100);

        // Create hover popup
        hoverPopup = new maplibregl.Popup({
            closeButton: false,
            closeOnClick: false,
            maxWidth: '300px',
            offset: [0, -25]
        });

        function onMapReady() {
            if (!map || mapLoaded) return;
            mapLoaded = true;

            // Setup cluster layers
            setupClusterLayers();

            // Register interaction handlers
            map.on('click', handleMapClick);
            map.on('mousemove', handleMouseMove);
            map.on('mouseout', () => { hideHoverPopup(); });
            map.on('idle', handleIdle);
            map.on('styleimagemissing', handleMissingImage);
            map.on('move', handleMove);
            map.on('moveend', handleMoveEnd);
            refresh();

            // Track drag/zoom to suppress mousemove during active interaction
            map.on('movestart', handleDragStart);
            map.on('moveend', handleDragEnd);

            requestAnimationFrame(() => {
                isReady = true;
            });
        }

        if (map.loaded()) {
            onMapReady();
        } else {
            map.on('load', onMapReady);
            map.on('style.load', () => {
                if (!mapLoaded) onMapReady();
            });
        }
    }

    onDestroy(() => {
        delete (window as any).__footosMap;
        delete (window as any).__footosMapStats;
        delete (window as any).__footosSetTimeFilter;
        loadedImages.clear();
        loadingImages.clear();
        thumbDataCache.clear();
        if (timelineSyncTimer) {
            clearTimeout(timelineSyncTimer);
            timelineSyncTimer = null;
        }
        if (moveTimer) {
            clearTimeout(moveTimer);
            moveTimer = null;
        }
        if (resizeObserver) {
            resizeObserver.disconnect();
            resizeObserver = null;
        }
        if (hoverPopup) {
            hoverPopup.remove();
            hoverPopup = null;
        }
        if (map) {
            map.remove();
            map = null;
        }
    });

    // ============================================================
    // Reactive Effects
    // ============================================================

    // Update map style when theme changes
    let prevTheme: string = theme;
    $effect(() => {
        const currentTheme = theme;
        if (!map || !mapLoaded) return;
        if (currentTheme === prevTheme) return;
        prevTheme = currentTheme;

        const newStyle = getMapStyle(currentTheme === 'dark');

        try {
            // Theme changed — thumbnail border color depends on theme, clear all caches
            loadedImages.clear();
            loadingImages.clear();
            thumbDataCache.clear();
            lastFingerprint = '';

            // Remove handlers before style change to prevent duplicates
            map.off('click', handleMapClick);
            map.off('mousemove', handleMouseMove);
            map.off('idle', handleIdle);
            map.off('move', handleMove);
            map.off('moveend', handleMoveEnd);
            map.off('styleimagemissing', handleMissingImage);
            map.off('movestart', handleDragStart);
            map.off('moveend', handleDragEnd);

            map.setStyle(newStyle);

            // Re-setup layers and handlers after style is ready
            const mapRef = map;
            function onStyleReady() {
                if (!mapRef || !mapRef.isStyleLoaded()) {
                    requestAnimationFrame(onStyleReady);
                    return;
                }
                setupClusterLayers();
                applyFeatures(currentFeatures);
                refresh();

                mapRef.on('click', handleMapClick);
                mapRef.on('mousemove', handleMouseMove);
                mapRef.on('idle', handleIdle);
                mapRef.on('move', handleMove);
                mapRef.on('moveend', handleMoveEnd);
                mapRef.on('styleimagemissing', handleMissingImage);
                mapRef.on('movestart', handleDragStart);
                mapRef.on('moveend', handleDragEnd);
            }
            requestAnimationFrame(onStyleReady);
        } catch (_) {
            // Style change failed — non-critical
        }
    });

    // Library changed (import progress, delete, clear): query again.
    $effect(() => {
        refreshKey;
        if (!mapLoaded) return;
        untrack(() => refresh());
    });

    // ============================================================
    // Box Selection (Desktop only)
    // ============================================================

    let boxSelectedTimeRange = $state<{ startMs: number; endMs: number } | null>(null);

    function toggleBoxSelectMode() {
        isBoxSelectMode = !isBoxSelectMode;
        if (!isBoxSelectMode) {
            cleanupBoxSelection();
        }
        (window as any).__boxSelectControl?.update();
    }

    function cleanupBoxSelection() {
        if (selectionBox && selectionBox.parentNode) {
            selectionBox.parentNode.removeChild(selectionBox);
        }
        selectionBox = null;
        isDrawingBox = false;
        boxStart = null;
    }

    function handleGlobalMouseDown(e: MouseEvent) {
        if (!isBoxSelectMode || !map || !mapContainer) return;

        const rect = mapContainer.getBoundingClientRect();
        if (e.clientX < rect.left || e.clientX > rect.right ||
            e.clientY < rect.top || e.clientY > rect.bottom) return;

        e.preventDefault();
        isDrawingBox = true;
        boxStart = { x: e.clientX - rect.left, y: e.clientY - rect.top };

        selectionBox = document.createElement('div');
        selectionBox.className = 'selection-box';
        selectionBox.style.left = `${boxStart.x}px`;
        selectionBox.style.top = `${boxStart.y}px`;
        selectionBox.style.width = '0px';
        selectionBox.style.height = '0px';
        mapContainer.appendChild(selectionBox);

        map.dragPan.disable();
    }

    function handleGlobalMouseMove(e: MouseEvent) {
        if (!isDrawingBox || !boxStart || !selectionBox || !mapContainer) return;

        const rect = mapContainer.getBoundingClientRect();
        const currentX = e.clientX - rect.left;
        const currentY = e.clientY - rect.top;

        const minX = Math.min(boxStart.x, currentX);
        const minY = Math.min(boxStart.y, currentY);
        const width = Math.abs(currentX - boxStart.x);
        const height = Math.abs(currentY - boxStart.y);

        selectionBox.style.left = `${minX}px`;
        selectionBox.style.top = `${minY}px`;
        selectionBox.style.width = `${width}px`;
        selectionBox.style.height = `${height}px`;
    }

    function handleGlobalMouseUp(e: MouseEvent) {
        if (!isDrawingBox || !boxStart || !map || !mapContainer) return;

        const rect = mapContainer.getBoundingClientRect();
        const endX = e.clientX - rect.left;
        const endY = e.clientY - rect.top;

        // Get bounds from pixel coordinates
        const sw = map.unproject([Math.min(boxStart.x, endX), Math.max(boxStart.y, endY)]);
        const ne = map.unproject([Math.max(boxStart.x, endX), Math.min(boxStart.y, endY)]);
        const bounds = new maplibregl.LngLatBounds(sw, ne);

        // Time span of what is inside the box, straight from the index.
        const boxMap = map;
        queryTimeline({
            west: bounds.getWest(),
            south: bounds.getSouth(),
            east: bounds.getEast(),
            north: bounds.getNorth(),
        }).then((stamps) => {
            if (stamps.length === 0 || map !== boxMap) return;
            boxSelectedTimeRange = { startMs: stamps[0], endMs: stamps[stamps.length - 1] };
            boxMap.fitBounds(bounds, { padding: 100, animate: true });
        }).catch((e) => console.error("[Map] box select failed", e));

        // Cleanup
        cleanupBoxSelection();
        map.dragPan.enable();
        isBoxSelectMode = false;
        (window as any).__boxSelectControl?.update();
    }
</script>

<svelte:window
    onmousedown={handleGlobalMouseDown}
    onmousemove={handleGlobalMouseMove}
    onmouseup={handleGlobalMouseUp}
/>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="w-full h-full theme-bg-secondary flex flex-col overflow-hidden" style:cursor={isBoxSelectMode ? 'crosshair' : 'auto'}>
    <!-- Map area -->
    <div class="flex-1 min-h-0 relative">
        <div
            bind:this={mapContainer}
            class="absolute inset-0 map-container"
            class:box-select-mode={isBoxSelectMode}
        ></div>

        <!-- Time range indicator -->
        {#if timeRangeDisplay && hasGeotaggedPhotos}
            <div class="absolute top-4 left-1/2 -translate-x-1/2 z-[1000] pointer-events-none">
                <div class="px-5 py-2 text-base theme-bg-card rounded-full theme-text-primary font-medium shadow-lg flex items-center gap-1.5">
                    <i class="fa-regular fa-calendar text-[var(--accent)] flex-shrink-0"></i>
                    <span class="tabular-nums whitespace-nowrap">{timeRangeDisplay}</span>
                </div>
            </div>
        {/if}

        <!-- No geotagged photos message -->
        {#if summaryLoaded && libraryCount > 0 && !hasGeotaggedPhotos}
            <div class="absolute inset-0 flex items-center justify-center bg-black/30 backdrop-blur-sm z-[1000] pointer-events-none">
                <div class="text-center p-6 theme-bg-secondary rounded-2xl border theme-border shadow-xl">
                    <i class="fa-solid fa-location-slash text-4xl theme-text-muted mb-4"></i>
                    <h3 class="text-xl font-bold theme-text-primary mb-2">{t.map.noGeotaggedPhotos}</h3>
                    <p class="theme-text-secondary max-w-xs">{t.map.noGeotaggedPhotosDesc}</p>
                </div>
            </div>
        {/if}

    </div>

    <!-- Timeline slider -->
    {#if hasGeotaggedPhotos}
        <div class="flex-shrink-0 min-h-[120px]">
            {#if map && isReady && timelineStamps.length > 0}
                <TimelineSlider
                    stamps={timelineStamps}
                    {mapConstrained}
                    externalTimeRange={boxSelectedTimeRange}
                    onTimeRangeChange={handleTimeRangeChange}
                    onExternalRangeConsumed={() => { boxSelectedTimeRange = null; }}
                    onShowAll={handleShowAllPhotos}
                    {t}
                    {locale}
                />
            {/if}
        </div>
    {/if}
</div>

<style>
    /* Map container must have explicit dimensions for MapLibre */
    .map-container {
        width: 100%;
        height: 100%;
    }
    .map-container :global(.maplibregl-map) {
        width: 100% !important;
        height: 100% !important;
    }
    .map-container :global(.maplibregl-canvas-container) {
        width: 100% !important;
        height: 100% !important;
    }
    .map-container :global(.maplibregl-canvas) {
        width: 100% !important;
        height: 100% !important;
    }

    /* MapLibre overrides - Dark theme (default) */
    :global(.maplibregl-ctrl-attrib) {
        background-color: rgba(15, 23, 42, 0.8) !important;
        color: #94a3b8 !important;
        border-radius: 4px;
        font-size: 10px;
    }
    :global(.maplibregl-ctrl-attrib a) {
        color: #818cf8 !important;
    }
    :global(.maplibregl-ctrl-group) {
        background: rgba(15, 23, 42, 0.9) !important;
        border: 1px solid rgba(255, 255, 255, 0.1) !important;
    }
    :global(.maplibregl-ctrl-group button) {
        background-color: transparent !important;
        border-bottom: 1px solid rgba(255, 255, 255, 0.1) !important;
    }
    :global(.maplibregl-ctrl-group button:hover) {
        background-color: rgba(255, 255, 255, 0.1) !important;
    }
    :global(.maplibregl-ctrl-group button + button) {
        border-top: none !important;
    }
    :global(.maplibregl-ctrl button .maplibregl-ctrl-icon) {
        filter: invert(1);
    }

    /* MapLibre overrides - Light theme */
    :global(:root.light .maplibregl-ctrl-attrib) {
        background-color: rgba(255, 255, 255, 0.9) !important;
        color: #64748b !important;
    }
    :global(:root.light .maplibregl-ctrl-attrib a) {
        color: #6366f1 !important;
    }
    :global(:root.light .maplibregl-ctrl-group) {
        background: rgba(255, 255, 255, 0.95) !important;
        border: 1px solid rgba(0, 0, 0, 0.1) !important;
    }
    :global(:root.light .maplibregl-ctrl-group button) {
        border-bottom: 1px solid rgba(0, 0, 0, 0.1) !important;
    }
    :global(:root.light .maplibregl-ctrl-group button:hover) {
        background-color: rgba(0, 0, 0, 0.05) !important;
    }
    :global(:root.light .maplibregl-ctrl button .maplibregl-ctrl-icon) {
        filter: none;
    }

    /* Box select control */
    :global(.maplibregl-ctrl-box-select) {
        width: 29px;
        height: 29px;
        display: flex;
        align-items: center;
        justify-content: center;
        color: #e2e8f0;
        cursor: pointer;
        transition: all 0.15s ease;
    }
    :global(:root.light .maplibregl-ctrl-box-select) {
        color: #475569;
    }
    :global(.maplibregl-ctrl-box-select:hover) {
        color: var(--accent);
    }
    :global(.maplibregl-ctrl-box-select.active) {
        background-color: var(--accent) !important;
        color: black !important;
    }

    /* Show all photos control */
    :global(.maplibregl-ctrl-show-all) {
        width: 29px;
        height: 29px;
        display: flex;
        align-items: center;
        justify-content: center;
        color: #e2e8f0;
        cursor: pointer;
        transition: all 0.15s ease;
    }
    :global(:root.light .maplibregl-ctrl-show-all) {
        color: #475569;
    }
    :global(.maplibregl-ctrl-show-all:hover) {
        color: var(--accent);
    }

    /* Photo popup styles - Dark theme (default) */
    :global(.maplibregl-popup-content) {
        background: rgba(15, 23, 42, 0.95) !important;
        border-radius: 12px !important;
        padding: 0 !important;
        box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5) !important;
        border: 1px solid rgba(255, 255, 255, 0.1) !important;
        overflow: hidden;
    }
    :global(.maplibregl-popup-tip) {
        border-top-color: rgba(15, 23, 42, 0.95) !important;
    }
    :global(.photo-popup) {
        position: relative;
        width: 280px;
    }
    :global(.photo-popup img) {
        width: 100%;
        height: 200px;
        object-fit: cover;
        display: block;
    }
    :global(.popup-info) {
        padding: 10px 12px;
        color: white;
    }
    :global(.popup-filename) {
        font-size: 12px;
        font-weight: 600;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    :global(.popup-date) {
        font-size: 11px;
        color: #94a3b8;
        margin-top: 4px;
    }

    /* Photo popup styles - Light theme */
    :global(:root.light .maplibregl-popup-content) {
        background: rgba(255, 255, 255, 0.98) !important;
        box-shadow: 0 8px 32px rgba(0, 0, 0, 0.15) !important;
        border: 1px solid rgba(0, 0, 0, 0.1) !important;
    }
    :global(:root.light .maplibregl-popup-tip) {
        border-top-color: rgba(255, 255, 255, 0.98) !important;
    }
    :global(:root.light .popup-info) {
        color: #1e293b;
    }
    :global(:root.light .popup-date) {
        color: #64748b;
    }
    :global(.popup-raw-badge) {
        position: absolute;
        top: 8px;
        right: 8px;
        background: #d97706;
        color: white;
        font-size: 10px;
        font-weight: bold;
        padding: 2px 6px;
        border-radius: 4px;
    }

    /* Box selection styles */
    :global(.selection-box) {
        position: absolute;
        border: 2px dashed var(--accent);
        background-color: rgba(var(--accent-rgb, 99, 102, 241), 0.15);
        pointer-events: none;
        z-index: 1000;
    }

    /* Force crosshair cursor in box select mode */
    .box-select-mode,
    .box-select-mode :global(*),
    .box-select-mode :global(.maplibregl-canvas-container),
    .box-select-mode :global(.maplibregl-canvas) {
        cursor: crosshair !important;
    }
</style>
