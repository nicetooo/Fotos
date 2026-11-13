<script lang="ts">
    import { onMount, onDestroy, untrack } from "svelte";
    import maplibregl from "maplibre-gl";
    import "maplibre-gl/dist/maplibre-gl.css";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import TimelineSlider from "./TimelineSlider.svelte";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";
    import {
        queryMap,
        geoSummary,
        queryTimeline,
        paddedBbox,
        abbreviateCount,
        type Bbox,
        type MapFeature,
    } from "../../shared/query";

    // The map never receives the library. It asks Rust for what is inside the
    // viewport (already clustered) and draws exactly that.
    let {
        refreshKey = 0,
        libraryCount = 0,
        onOpenPreview,
        theme = "dark",
        tileConfig,
        t,
        locale,
        isSyncing = false,
    } = $props<{
        /** Bump to re-query after the library changed (import progress, delete, clear). */
        refreshKey?: number;
        /** Rows in the library, geotagged or not (for the "no GPS" hint). */
        libraryCount?: number;
        onOpenPreview?: (item: { id: number; path: string }) => void;
        theme?: "dark" | "light";
        tileConfig?: { tiles: string[]; tileSize: number };
        t: Translations;
        locale: Locale;
        isSyncing?: boolean;
    }>();

    let mapContainer: HTMLDivElement;
    let map = $state<maplibregl.Map | null>(null);
    let mapLoaded = $state(false);
    let isReady = $state(false);

    /** Geotagged items in the whole library. */
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

    let resizeObserver: ResizeObserver | null = null;

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
            month: 'short', day: 'numeric', year: '2-digit', timeZone: 'UTC'
        });
        const formatTime = (d: Date) => d.toLocaleTimeString(locale, {
            hour: 'numeric', minute: '2-digit', timeZone: 'UTC'
        });
        if (sameDay) {
            return `${formatDate(a)} ${formatTime(a)} - ${formatTime(b)}`;
        }
        return `${formatDate(a)} - ${formatDate(b)}`;
    }

    let timeRangeDisplay = $derived(formatTimeRange(timeFilterStart, timeFilterEnd));

    function getThumbnailUrl(path: string): string {
        return convertFileSrc(path);
    }

    // ============================================================
    // Layers
    // ============================================================

    const SOURCE_ID = 'photos-source';
    const CLUSTER_LAYER_ID = 'photo-clusters';
    const CLUSTER_THUMB_LAYER_ID = 'photo-cluster-thumb';
    const CLUSTER_COUNT_LAYER_ID = 'photo-cluster-count';
    const UNCLUSTERED_LAYER_ID = 'photo-unclustered';
    const UNCLUSTERED_BORDER_LAYER_ID = 'photo-unclustered-border';
    const UNCLUSTERED_THUMB_LAYER_ID = 'photo-unclustered-thumb';
    const GLYPHS_URL = 'https://demotiles.maplibre.org/font/{fontstack}/{range}.pbf';

    // Thumbnail rendering constants
    const THUMB_CANVAS_SIZE = 66;    // Canvas pixels (renders at 44px with pixelRatio 1.5)
    const THUMB_BORDER_WIDTH = 3;    // Border width in canvas pixels
    const THUMB_PIXEL_RATIO = 1.5;   // Balance between clarity and GPU memory

    const IS_CLUSTER: any = ['>', ['get', 'count'], 1];
    const IS_SINGLE: any = ['==', ['get', 'count'], 1];

    // Image ids are derived from the thumbnail path, so they survive re-queries:
    // `thumb:<path>` (bordered, single item) and `cthumb:<path>` (borderless, cluster).
    let loadedImages = new Set<string>();
    let loadingImages = new Set<string>();
    // LRU of rendered ImageData. Map insertion order = oldest first.
    let thumbDataCache = new Map<string, ImageData>();
    const THUMB_CACHE_MAX = 120;   // Max ImageData entries in JS heap (~2MB)
    const ATLAS_BUDGET = 180;      // Max images in MapLibre GPU atlas

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

    /** Ask Rust for the clusters in (a padded) viewport and draw them. */
    async function refresh() {
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

    function handleShowAllPhotos() {
        fitAll(false);
    }

    // ============================================================
    // Interaction
    // ============================================================

    // Above this zoom a cluster is photos taken at the same spot: open it instead of zooming.
    const CLUSTER_OPEN_ZOOM = 17;

    /**
     * Global click handler using queryRenderedFeatures.
     * Layer-specific click handlers don't reliably fire on Android WebView touch
     * events, so query features around the tap point manually.
     */
    function handleMapClick(e: maplibregl.MapMouseEvent) {
        if (!map) return;

        const tolerance = 15; // fat finger
        const bbox: [maplibregl.PointLike, maplibregl.PointLike] = [
            [e.point.x - tolerance, e.point.y - tolerance],
            [e.point.x + tolerance, e.point.y + tolerance]
        ];

        function closestFeature(features: maplibregl.GeoJSONFeature[], point: maplibregl.Point): maplibregl.GeoJSONFeature {
            if (features.length === 1) return features[0];
            let best = features[0];
            let bestDist = Infinity;
            for (const f of features) {
                const coords = (f.geometry as GeoJSON.Point).coordinates as [number, number];
                const projected = map!.project(coords);
                const dx = projected.x - point.x;
                const dy = projected.y - point.y;
                const dist = dx * dx + dy * dy;
                if (dist < bestDist) {
                    bestDist = dist;
                    best = f;
                }
            }
            return best;
        }

        const features = map.queryRenderedFeatures(bbox, {
            layers: [
                CLUSTER_THUMB_LAYER_ID, CLUSTER_LAYER_ID, CLUSTER_COUNT_LAYER_ID,
                UNCLUSTERED_THUMB_LAYER_ID, UNCLUSTERED_LAYER_ID, UNCLUSTERED_BORDER_LAYER_ID,
            ]
        });
        if (features.length === 0) return;

        const p = closestFeature(features, e.point).properties as any;
        if (!p) return;

        const degenerate = p.w === p.e && p.s === p.n;
        if (p.count > 1 && !degenerate && map.getZoom() < CLUSTER_OPEN_ZOOM) {
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

    // ============================================================
    // Map setup
    // ============================================================

    function getMapStyle(isDark: boolean): maplibregl.StyleSpecification {
        if (tileConfig) {
            return {
                version: 8,
                glyphs: GLYPHS_URL,
                sources: {
                    'tiles': {
                        type: 'raster',
                        tiles: tileConfig.tiles,
                        tileSize: tileConfig.tileSize,
                        attribution: '&copy; OpenStreetMap contributors'
                    }
                },
                layers: [
                    {
                        id: 'tiles-layer',
                        type: 'raster',
                        source: 'tiles',
                        minzoom: 0,
                        maxzoom: 19
                    }
                ]
            };
        }

        const tileType = isDark ? 'dark_all' : 'light_all';
        const sourceId = isDark ? 'carto-dark' : 'carto-light';
        return {
            version: 8,
            glyphs: GLYPHS_URL,
            sources: {
                [sourceId]: {
                    type: 'raster',
                    tiles: [
                        `https://a.basemaps.cartocdn.com/${tileType}/{z}/{x}/{y}.png`,
                        `https://b.basemaps.cartocdn.com/${tileType}/{z}/{x}/{y}.png`,
                        `https://c.basemaps.cartocdn.com/${tileType}/{z}/{x}/{y}.png`,
                        `https://d.basemaps.cartocdn.com/${tileType}/{z}/{x}/{y}.png`
                    ],
                    tileSize: 256,
                    attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> &copy; <a href="https://carto.com/attributions">CARTO</a>'
                }
            },
            layers: [
                {
                    id: `${sourceId}-layer`,
                    type: 'raster',
                    source: sourceId,
                    minzoom: 0,
                    maxzoom: 20
                }
            ]
        };
    }

    onMount(() => {
        if (!mapContainer) return;

        map = new maplibregl.Map({
            container: mapContainer,
            style: getMapStyle(theme === 'dark'),
            center: [0, 20],
            zoom: 2,
            attributionControl: false,
            maxTileCacheSize: 50,
            pixelRatio: 2,
        });

        setupMapControls();

        (window as any).__footosMap = map;
        (window as any).__footosMapStats = () => ({
            ...stats,
            total,
            features: currentFeatures.length,
            stamps: timelineStamps.length,
            atlasImages: loadedImages.size,
        });
    });

    function registerHandlers(m: maplibregl.Map) {
        m.on('click', handleMapClick);
        m.on('idle', handleIdle);
        m.on('styleimagemissing', handleMissingImage);
        m.on('move', handleMove);
        m.on('moveend', handleMoveEnd);
    }

    function unregisterHandlers(m: maplibregl.Map) {
        m.off('click', handleMapClick);
        m.off('idle', handleIdle);
        m.off('styleimagemissing', handleMissingImage);
        m.off('move', handleMove);
        m.off('moveend', handleMoveEnd);
    }

    function setupMapControls() {
        if (!map) return;

        class ShowAllControl {
            _container: HTMLDivElement | undefined;
            _button: HTMLButtonElement | undefined;

            onAdd() {
                this._container = document.createElement('div');
                this._container.className = 'maplibregl-ctrl maplibregl-ctrl-group';

                this._button = document.createElement('button');
                this._button.type = 'button';
                this._button.className = 'maplibregl-ctrl-show-all';
                this._button.title = t.map.showAllPhotos;
                this._button.innerHTML = '<i class="fa-solid fa-expand"></i>';
                this._button.onclick = (e) => {
                    e.stopPropagation();
                    handleShowAllPhotos();
                };

                this._container.appendChild(this._button);
                return this._container;
            }

            onRemove() {
                this._container?.parentNode?.removeChild(this._container);
            }
        }

        const showAllControl = new ShowAllControl();
        map.addControl(showAllControl as any, 'bottom-right');
        map.addControl(new maplibregl.NavigationControl(), 'bottom-right');
        map.addControl(new maplibregl.AttributionControl({ compact: true }), 'bottom-left');

        resizeObserver = new ResizeObserver(() => {
            map?.resize();
        });
        resizeObserver.observe(mapContainer);

        setTimeout(() => {
            map?.resize();
        }, 100);

        function onMapReady() {
            if (!map || mapLoaded) return;
            mapLoaded = true;

            setupClusterLayers();
            registerHandlers(map);
            refresh();

            requestAnimationFrame(() => {
                isReady = true;
            });
        }

        if (map.loaded()) {
            onMapReady();
        } else {
            map.on('load', onMapReady);
            // style.load fires before load on some WebView versions
            map.on('style.load', () => {
                if (!mapLoaded) onMapReady();
            });
        }
    }

    onDestroy(() => {
        loadedImages.clear();
        loadingImages.clear();
        thumbDataCache.clear();
        querySeq++;
        stampSeq++;
        delete (window as any).__footosMap;
        delete (window as any).__footosMapStats;

        if (moveTimer) clearTimeout(moveTimer);
        if (timelineSyncTimer) clearTimeout(timelineSyncTimer);
        if (resizeObserver) {
            resizeObserver.disconnect();
            resizeObserver = null;
        }
        if (map) {
            map.remove();
            map = null;
        }
    });

    // ============================================================
    // Reactive Effects
    // ============================================================

    // Update map style when theme or tile source changes
    let prevTheme: string = theme;
    let prevTiles = tileConfig;
    $effect(() => {
        const currentTheme = theme;
        const tiles = tileConfig;
        if (!map || !mapLoaded) return;
        if (currentTheme === prevTheme && tiles === prevTiles) return;
        prevTheme = currentTheme;
        prevTiles = tiles;

        const newStyle = getMapStyle(currentTheme === 'dark');
        const camera = {
            center: map.getCenter(),
            zoom: map.getZoom(),
            bearing: map.getBearing(),
            pitch: map.getPitch(),
        };

        try {
            // Thumbnail border colour depends on the theme: drop every cached image.
            loadedImages.clear();
            loadingImages.clear();
            thumbDataCache.clear();
            lastFingerprint = '';

            // Handlers survive setStyle(); remove them to prevent duplicates.
            unregisterHandlers(map);

            map.setStyle(newStyle);

            // NOTE: map.once('style.load') is unreliable in Android WebView for inline
            // styles. Poll isStyleLoaded() instead.
            const mapRef = map;
            function onStyleReady() {
                if (!mapRef || !mapRef.isStyleLoaded()) {
                    requestAnimationFrame(onStyleReady);
                    return;
                }
                setupClusterLayers();
                applyFeatures(currentFeatures);
                mapRef.jumpTo(camera);
                registerHandlers(mapRef);
                refresh();
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
</script>

<div class="w-full h-full theme-bg-secondary flex flex-col overflow-hidden">
    <!-- Map area -->
    <div class="flex-1 min-h-0 relative">
        <div
            bind:this={mapContainer}
            class="absolute inset-0 map-container mobile-map"
        ></div>

        <!-- Time range indicator -->
        {#if timeRangeDisplay && hasGeotaggedPhotos}
            <div class="absolute left-1/2 -translate-x-1/2 z-[1000] pointer-events-none max-w-[95%]" style="top: calc(max(47px, env(safe-area-inset-top, 0px)) + 12px)">
                <div class="px-3 py-1.5 text-[11px] theme-bg-card rounded-full theme-text-primary font-medium shadow-lg flex items-center gap-1.5">
                    <i class="fa-regular fa-calendar text-[var(--accent)] text-[10px] flex-shrink-0"></i>
                    <span class="tabular-nums whitespace-nowrap">{timeRangeDisplay}</span>
                </div>
            </div>
        {/if}

        <!-- No geotagged photos message -->
        {#if !isSyncing && summaryLoaded && libraryCount > 0 && !hasGeotaggedPhotos}
            <div class="absolute inset-0 flex items-center justify-center bg-black/30 z-[1000] pointer-events-none">
                <div class="text-center p-6 theme-bg-secondary rounded-2xl border theme-border shadow-xl">
                    <i class="fa-solid fa-location-slash text-4xl theme-text-muted mb-4"></i>
                    <h3 class="text-xl font-bold theme-text-primary mb-2">{t.map.noGeotaggedPhotos}</h3>
                    <p class="theme-text-secondary max-w-xs">{t.map.noGeotaggedPhotosDesc}</p>
                </div>
            </div>
        {/if}

    </div>

    <!-- Timeline slider -->
    {#if hasGeotaggedPhotos && map && isReady && timelineStamps.length > 0}
        <div class="flex-shrink-0">
            <TimelineSlider
                stamps={timelineStamps}
                {mapConstrained}
                onTimeRangeChange={handleTimeRangeChange}
                onShowAll={handleShowAllPhotos}
                {t}
                {locale}
            />
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

    /* Show all control */
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
    :global(.maplibregl-ctrl-show-all:active) {
        transform: scale(0.9);
        opacity: 0.7;
    }
    :global(.maplibregl-ctrl-show-all:focus) {
        outline: none;
        box-shadow: none;
    }

    /* Mobile: move bottom-right controls up above attribution */
    .mobile-map :global(.maplibregl-ctrl-bottom-right) {
        bottom: 30px !important;
        right: 10px !important;
    }

    /* Ensure map controls are tappable on mobile (Android WebView touch fix) */
    .mobile-map :global(.maplibregl-ctrl-group button) {
        touch-action: manipulation !important;
        -webkit-tap-highlight-color: transparent;
        min-width: 40px !important;
        min-height: 40px !important;
    }
    .mobile-map :global(.maplibregl-ctrl-group) {
        touch-action: manipulation !important;
        z-index: 10 !important;
    }
</style>
