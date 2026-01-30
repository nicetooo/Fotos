<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import maplibregl from "maplibre-gl";
    import "maplibre-gl/dist/maplibre-gl.css";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import TimelineSlider from "./TimelineSlider.svelte";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";

    let { photos, onOpenPreview, theme = "dark", tileConfig, t, locale = 'en' as Locale } = $props<{
        photos: any[];
        onOpenPreview?: (photo: any, visiblePhotos: any[]) => void;
        theme?: "dark" | "light";
        tileConfig?: { tiles: string[]; tileSize: number };
        t: Translations;
        locale?: Locale;
    }>();

    let mapContainer: HTMLDivElement;
    let map = $state<maplibregl.Map | null>(null);
    let mapLoaded = $state(false);
    let isReady = $state(false);

    // Time filter state
    let timeFilterStart = $state<Date | null>(null);
    let timeFilterEnd = $state<Date | null>(null);

    // Box selection state
    let isBoxSelectMode = $state(false);
    let isDrawingBox = $state(false);
    let boxStart: { x: number; y: number } | null = null;
    let selectionBox: HTMLDivElement | null = null;

    // Sync mode: prevent circular updates
    let syncSource = $state<'map' | 'timeline' | null>(null);

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

    // All photos data (for lookups and timeline)
    let allPhotosData: any[] = [];
    // Photo lookup by path
    let photoByPath: Map<string, any> = new Map();
    // Pre-parsed dates by path
    let dateByPath: Map<string, Date> = new Map();

    // Parse date from photo metadata
    function parsePhotoDate(dateStr: string | null): Date | null {
        if (!dateStr) return null;
        const cleaned = dateStr.replace(/"/g, '').trim();
        const date = new Date(cleaned.replace(' ', 'T'));
        return isNaN(date.getTime()) ? null : date;
    }

    // Geotagged photos (reactive)
    let geotaggedPhotos = $derived(photos.filter((p: any) => p.metadata?.lat && p.metadata?.lon));
    let hasGeotaggedPhotos = $derived(geotaggedPhotos.length > 0);

    function handleTimeRangeChange(start: Date, end: Date) {
        if (syncSource === 'map') {
            syncSource = null;
            return;
        }
        syncSource = 'timeline';
        timeFilterStart = start;
        timeFilterEnd = end;
        setTimeout(() => {
            syncSource = null;
        }, 100);
    }

    // Format time range for display
    function formatTimeRange(start: Date | null, end: Date | null, short: boolean = false): string {
        if (!start || !end) return '';
        const sameDay = start.toDateString() === end.toDateString();
        const formatDate = (d: Date) => d.toLocaleDateString(locale, {
            month: 'short',
            day: 'numeric',
            year: short ? '2-digit' : 'numeric'
        });
        const formatTime = (d: Date) => d.toLocaleTimeString(locale, {
            hour: short ? 'numeric' : '2-digit',
            minute: '2-digit'
        });
        if (sameDay) {
            return `${formatDate(start)} ${formatTime(start)} - ${formatTime(end)}`;
        }
        return `${formatDate(start)} - ${formatDate(end)}`;
    }

    let timeRangeDisplay = $derived(formatTimeRange(timeFilterStart, timeFilterEnd, false));

    // Photos with markers for TimelineSlider
    let photosWithMarkers = $state<any[]>([]);

    // Get visible photos sorted by time for navigation
    let visiblePhotosSorted = $derived.by(() => {
        const start = timeFilterStart;
        const end = timeFilterEnd;

        const visible: { photo: any; date: Date }[] = [];
        for (const photo of geotaggedPhotos) {
            const date = dateByPath.get(photo.path) || parsePhotoDate(photo.metadata?.date_taken);
            if (!date) continue;
            if (start && end) {
                if (date >= start && date <= end) {
                    visible.push({ photo, date });
                }
            } else {
                visible.push({ photo, date });
            }
        }

        visible.sort((a, b) => a.date.getTime() - b.date.getTime());
        return visible.map(v => v.photo);
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
    const GLYPHS_URL = 'https://demotiles.maplibre.org/font/{fontstack}/{range}.pbf';

    // Desktop thumbnail constants (larger than mobile for Retina clarity)
    const THUMB_CANVAS_SIZE = 88;     // 88px canvas → 44px display at pixelRatio 2
    const THUMB_BORDER_WIDTH = 4;
    const THUMB_PIXEL_RATIO = 2;

    // Desktop has more GPU/RAM headroom than mobile
    const THUMB_CACHE_MAX = 200;      // Max ImageData entries in JS heap
    const ATLAS_BUDGET = 300;         // Max images in MapLibre GPU atlas

    // Track previous photo count to detect add vs remove
    let prevPhotoCount = 0;

    // Track loaded thumbnail map images
    let loadedImages = new Set<string>();
    let loadingImages = new Set<string>();
    // LRU ImageData cache
    let thumbDataCache = new Map<string, ImageData>();
    // GeoJSON fingerprint to skip redundant setData() calls
    let lastGeoJSONFingerprint = '';

    /** LRU get — moves entry to end (most recent). */
    function thumbCacheGet(key: string): ImageData | undefined {
        const val = thumbDataCache.get(key);
        if (val !== undefined) {
            thumbDataCache.delete(key);
            thumbDataCache.set(key, val);
        }
        return val;
    }

    /** LRU set — evicts oldest if over limit. */
    function thumbCacheSet(key: string, value: ImageData) {
        if (thumbDataCache.has(key)) {
            thumbDataCache.delete(key);
        }
        thumbDataCache.set(key, value);
        while (thumbDataCache.size > THUMB_CACHE_MAX) {
            const oldest = thumbDataCache.keys().next().value;
            if (oldest !== undefined) thumbDataCache.delete(oldest);
            else break;
        }
    }

    /**
     * Evict non-visible images from MapLibre's GPU atlas to stay under budget.
     * Called on 'idle' after loading visible thumbnails.
     */
    function evictNonVisibleImages() {
        if (!map) return;
        if (loadedImages.size <= ATLAS_BUDGET) return;

        const visibleIds = new Set<string>();
        try {
            const clusters = map.queryRenderedFeatures(undefined, { layers: [CLUSTER_THUMB_LAYER_ID] });
            for (const f of clusters) {
                const cid = f.properties?.cluster_id;
                if (cid !== undefined) visibleIds.add(`cluster:${cid}`);
            }
        } catch (_) { /* layer may not exist */ }
        try {
            const singles = map.queryRenderedFeatures(undefined, { layers: [UNCLUSTERED_THUMB_LAYER_ID] });
            for (const f of singles) {
                const id = f.properties?.imageId;
                if (id) visibleIds.add(id);
            }
        } catch (_) { /* layer may not exist */ }

        let removed = 0;
        const target = loadedImages.size - ATLAS_BUDGET;
        for (const imageId of loadedImages) {
            if (removed >= target) break;
            if (visibleIds.has(imageId)) continue;
            try {
                if (map.hasImage(imageId)) map.removeImage(imageId);
            } catch (_) { /* ignore */ }
            loadedImages.delete(imageId);
            removed++;
        }
    }

    /**
     * Build GeoJSON FeatureCollection from photos, optionally filtered by time.
     */
    function buildGeoJSON(
        photos: any[],
        start: Date | null = null,
        end: Date | null = null
    ): GeoJSON.FeatureCollection {
        const features: GeoJSON.Feature[] = [];

        for (const photo of photos) {
            const lat = photo.metadata?.lat;
            const lon = photo.metadata?.lon;
            if (!lat || !lon) continue;

            // Time filter
            if (start && end) {
                const date = dateByPath.get(photo.path);
                if (date && (date < start || date > end)) continue;
            }

            const thumbPath = photo.thumb_path || photo.path;
            const imageId = `thumb:${thumbPath}`;
            const date = dateByPath.get(photo.path);
            const dateTs = date ? date.getTime() : 0;

            features.push({
                type: 'Feature',
                geometry: {
                    type: 'Point',
                    coordinates: [lon, lat]
                },
                properties: {
                    path: photo.path,
                    thumb_path: thumbPath,
                    imageId,
                    date_ts: dateTs,
                    date_taken: photo.metadata?.date_taken || '',
                    hasRaw: photo.hasRaw || false,
                }
            });
        }

        return { type: 'FeatureCollection', features };
    }

    /**
     * Setup GeoJSON source and cluster layers on the map.
     */
    function setupClusterLayers() {
        if (!map) return;
        if (map.getSource(SOURCE_ID)) return;

        map.addSource(SOURCE_ID, {
            type: 'geojson',
            data: { type: 'FeatureCollection', features: [] },
            cluster: true,
            clusterMaxZoom: 14,
            clusterRadius: 50,
        });

        const isDark = theme === 'dark';

        // Cluster border circle
        map.addLayer({
            id: CLUSTER_LAYER_ID,
            type: 'circle',
            source: SOURCE_ID,
            filter: ['has', 'point_count'],
            paint: {
                'circle-radius': [
                    'step', ['get', 'point_count'],
                    25,
                    10, 28,
                    50, 32,
                    100, 35,
                    500, 38
                ],
                'circle-color': isDark ? '#ffffff' : '#1e293b',
                'circle-stroke-width': 0,
            }
        });

        // Cluster thumbnail symbol
        map.addLayer({
            id: CLUSTER_THUMB_LAYER_ID,
            type: 'symbol',
            source: SOURCE_ID,
            filter: ['has', 'point_count'],
            layout: {
                'icon-image': ['concat', 'cluster:', ['to-string', ['get', 'cluster_id']]],
                'icon-size': [
                    'step', ['get', 'point_count'],
                    1.0,
                    10, 1.15,
                    50, 1.3,
                    100, 1.45,
                    500, 1.6
                ],
                'icon-allow-overlap': true,
                'icon-ignore-placement': true,
            }
        });

        // Cluster count text
        map.addLayer({
            id: CLUSTER_COUNT_LAYER_ID,
            type: 'symbol',
            source: SOURCE_ID,
            filter: ['has', 'point_count'],
            layout: {
                'text-field': ['get', 'point_count_abbreviated'],
                'text-font': ['Open Sans Semibold'],
                'text-size': [
                    'step', ['get', 'point_count'],
                    13,
                    10, 14,
                    100, 15,
                    500, 16
                ],
                'text-allow-overlap': true,
            },
            paint: {
                'text-color': '#ffffff',
                'text-halo-color': 'rgba(0, 0, 0, 0.65)',
                'text-halo-width': 1.5,
            }
        });

        // Unclustered border circle
        map.addLayer({
            id: UNCLUSTERED_BORDER_LAYER_ID,
            type: 'circle',
            source: SOURCE_ID,
            filter: ['!', ['has', 'point_count']],
            paint: {
                'circle-radius': 25,
                'circle-color': isDark ? '#ffffff' : '#1e293b',
                'circle-stroke-width': 0,
            }
        });

        // Unclustered placeholder circle (visible before thumbnail loads)
        map.addLayer({
            id: UNCLUSTERED_LAYER_ID,
            type: 'circle',
            source: SOURCE_ID,
            filter: ['!', ['has', 'point_count']],
            paint: {
                'circle-radius': 22,
                'circle-color': isDark
                    ? 'rgba(99, 102, 241, 0.9)'
                    : 'rgba(79, 70, 229, 0.9)',
                'circle-stroke-width': 2,
                'circle-stroke-color': isDark ? '#ffffff' : '#1e293b',
            }
        });

        // Unclustered thumbnail symbol
        map.addLayer({
            id: UNCLUSTERED_THUMB_LAYER_ID,
            type: 'symbol',
            source: SOURCE_ID,
            filter: ['!', ['has', 'point_count']],
            layout: {
                'icon-image': ['get', 'imageId'],
                'icon-size': 1,
                'icon-allow-overlap': true,
                'icon-ignore-placement': true,
            }
        });
    }

    /**
     * Update the GeoJSON source data with fingerprint-based deduplication.
     */
    function updateSourceData(photos: any[], start: Date | null = null, end: Date | null = null) {
        if (!map) return;
        const source = map.getSource(SOURCE_ID) as maplibregl.GeoJSONSource | undefined;
        if (!source) return;

        const geojson = buildGeoJSON(photos, start, end);

        // Content-based fingerprint to skip redundant setData()
        const n = geojson.features.length;
        let fingerprint: string;
        if (n === 0) {
            fingerprint = '0';
        } else {
            const firstPath = (geojson.features[0].properties as any)?.path || '';
            const lastPath = (geojson.features[n - 1].properties as any)?.path || '';
            const midPath = (geojson.features[Math.floor(n / 2)].properties as any)?.path || '';
            fingerprint = `${n}:${firstPath}:${midPath}:${lastPath}`;
        }
        if (fingerprint === lastGeoJSONFingerprint) return;
        lastGeoJSONFingerprint = fingerprint;

        // Clear cluster tracking (keep atlas images for instant re-render)
        for (const imageId of loadedImages) {
            if (imageId.startsWith('cluster:')) loadedImages.delete(imageId);
        }
        for (const imageId of loadingImages) {
            if (imageId.startsWith('cluster:')) loadingImages.delete(imageId);
        }

        source.setData(geojson);
    }

    // ============================================================
    // Thumbnail Loading
    // ============================================================

    /**
     * Draw a circular thumbnail image onto a canvas.
     */
    function drawCircularThumbnail(img: HTMLImageElement, borderWidth: number = THUMB_BORDER_WIDTH): ImageData {
        const size = THUMB_CANVAS_SIZE;
        const radius = size / 2;
        const inset = borderWidth;

        const canvas = document.createElement('canvas');
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext('2d')!;

        // Circular clip for the photo
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

        // Draw border
        if (borderWidth > 0) {
            ctx.beginPath();
            ctx.arc(radius, radius, radius - borderWidth / 2, 0, Math.PI * 2);
            ctx.strokeStyle = theme === 'dark' ? '#ffffff' : '#1e293b';
            ctx.lineWidth = borderWidth;
            ctx.stroke();
        }

        return ctx.getImageData(0, 0, size, size);
    }

    /**
     * Add an image to the map from cache or by fetching.
     */
    function addImageToMap(imageId: string, thumbPath: string, borderWidth: number = THUMB_BORDER_WIDTH) {
        if (!map) return;

        if (map.hasImage(imageId)) {
            loadedImages.add(imageId);
            loadingImages.delete(imageId);
            return;
        }

        const cacheKey = `${borderWidth}:${thumbPath}`;
        const cached = thumbCacheGet(cacheKey);
        if (cached) {
            try {
                map.addImage(imageId, cached, { pixelRatio: THUMB_PIXEL_RATIO });
                loadedImages.add(imageId);
            } catch (_) { /* ignore */ }
            loadingImages.delete(imageId);
            return;
        }

        const img = new Image();
        img.crossOrigin = 'anonymous';
        img.onload = () => {
            if (!map) return;
            loadingImages.delete(imageId);
            if (map.hasImage(imageId)) { loadedImages.add(imageId); return; }
            try {
                const imageData = drawCircularThumbnail(img, borderWidth);
                thumbCacheSet(cacheKey, imageData);
                map.addImage(imageId, imageData, { pixelRatio: THUMB_PIXEL_RATIO });
                loadedImages.add(imageId);
            } catch (_) { /* ignore */ }
        };
        img.onerror = () => { loadingImages.delete(imageId); };
        img.src = getThumbnailUrl(thumbPath);
    }

    function loadThumbnailForMap(imageId: string, thumbPath: string) {
        if (loadingImages.has(imageId) || loadedImages.has(imageId)) return;
        if (map?.hasImage(imageId)) { loadedImages.add(imageId); return; }
        loadingImages.add(imageId);
        addImageToMap(imageId, thumbPath, THUMB_BORDER_WIDTH);
    }

    /**
     * Handle missing image event from MapLibre.
     */
    function handleMissingImage(e: { id: string }) {
        const imageId = e.id;
        if (!map) return;
        if (loadingImages.has(imageId)) return;

        if (imageId.startsWith('cluster:')) {
            const clusterId = parseInt(imageId.replace('cluster:', ''), 10);
            if (isNaN(clusterId)) return;

            const source = map.getSource(SOURCE_ID) as maplibregl.GeoJSONSource;
            if (!source) return;

            loadingImages.add(imageId);

            source.getClusterLeaves(clusterId, 20, 0).then((leaves) => {
                if (!map || leaves.length === 0) {
                    loadingImages.delete(imageId);
                    return;
                }

                let bestLeaf = leaves[0];
                let bestTs = (leaves[0].properties?.date_ts as number) || 0;
                for (let i = 1; i < leaves.length; i++) {
                    const ts = (leaves[i].properties?.date_ts as number) || 0;
                    if (ts > bestTs) {
                        bestTs = ts;
                        bestLeaf = leaves[i];
                    }
                }

                const thumbPath = bestLeaf.properties?.thumb_path as string;
                if (!thumbPath) {
                    loadingImages.delete(imageId);
                    return;
                }

                addImageToMap(imageId, thumbPath, 0);
            }).catch(() => {
                loadingImages.delete(imageId);
            });
        } else if (imageId.startsWith('thumb:')) {
            const thumbPath = imageId.replace('thumb:', '');
            loadingImages.add(imageId);
            addImageToMap(imageId, thumbPath, THUMB_BORDER_WIDTH);
        }
    }

    /**
     * Load visible thumbnails on map 'idle'.
     */
    function loadVisibleThumbnails() {
        if (!map) return;

        loadClusterThumbnails();

        let features: maplibregl.MapGeoJSONFeature[];
        try {
            features = map.queryRenderedFeatures(undefined, {
                layers: [UNCLUSTERED_LAYER_ID]
            });
        } catch (_) {
            return;
        }

        if (features.length === 0) {
            evictNonVisibleImages();
            return;
        }

        let loaded = 0;
        const BATCH_SIZE = 25; // Larger batch for desktop

        for (const feature of features) {
            if (loaded >= BATCH_SIZE) break;

            const imageId = feature.properties?.imageId;
            if (!imageId) continue;
            if (loadedImages.has(imageId) || loadingImages.has(imageId)) continue;

            const thumbPath = feature.properties?.thumb_path;
            if (!thumbPath) continue;

            loadThumbnailForMap(imageId, thumbPath);
            loaded++;
        }

        evictNonVisibleImages();
    }

    /**
     * Load cluster thumbnails from visible clusters.
     */
    function loadClusterThumbnails() {
        if (!map) return;

        let clusterFeatures: maplibregl.MapGeoJSONFeature[];
        try {
            clusterFeatures = map.queryRenderedFeatures(undefined, {
                layers: [CLUSTER_LAYER_ID]
            });
        } catch (_) {
            return;
        }

        if (clusterFeatures.length === 0) return;

        const source = map.getSource(SOURCE_ID) as maplibregl.GeoJSONSource;
        if (!source) return;

        for (const cluster of clusterFeatures) {
            const clusterId = cluster.properties?.cluster_id;
            if (clusterId === undefined) continue;

            const imageId = `cluster:${clusterId}`;
            if (loadedImages.has(imageId) || loadingImages.has(imageId)) continue;

            if (map.hasImage(imageId)) {
                loadedImages.add(imageId);
                continue;
            }

            loadingImages.add(imageId);

            source.getClusterLeaves(clusterId, 20, 0).then((leaves) => {
                if (!map || leaves.length === 0) {
                    loadingImages.delete(imageId);
                    return;
                }

                let bestLeaf = leaves[0];
                let bestTs = (leaves[0].properties?.date_ts as number) || 0;
                for (let i = 1; i < leaves.length; i++) {
                    const ts = (leaves[i].properties?.date_ts as number) || 0;
                    if (ts > bestTs) {
                        bestTs = ts;
                        bestLeaf = leaves[i];
                    }
                }

                const thumbPath = bestLeaf.properties?.thumb_path as string;
                if (!thumbPath) {
                    loadingImages.delete(imageId);
                    return;
                }

                addImageToMap(imageId, thumbPath, 0);
            }).catch(() => {
                loadingImages.delete(imageId);
            });
        }
    }

    // ============================================================
    // Photo Initialization
    // ============================================================

    /**
     * Initialize photo data and update the GeoJSON source.
     */
    function initializePhotos(photos: any[]) {
        if (!map || !mapLoaded) return;

        if (photos.length === prevPhotoCount && photos.length > 0) return;

        const isInitialLoad = prevPhotoCount === 0;
        const isAddingPhotos = photos.length > prevPhotoCount;

        // Build lookup caches
        allPhotosData = photos;
        photoByPath.clear();
        dateByPath.clear();
        for (const photo of photos) {
            photoByPath.set(photo.path, photo);
            const date = parsePhotoDate(photo.metadata?.date_taken);
            if (date) dateByPath.set(photo.path, date);
        }

        updateSourceData(photos, timeFilterStart, timeFilterEnd);

        if (photos.length > 0 && (isInitialLoad || isAddingPhotos)) {
            fitBoundsToPhotos(photos, isInitialLoad);
        }

        prevPhotoCount = photos.length;
        photosWithMarkers = photos;
    }

    /**
     * Fit map bounds to show all geotagged photos.
     */
    function fitBoundsToPhotos(photos: any[], delay: boolean = false) {
        if (!map) return;

        const bounds = new maplibregl.LngLatBounds();
        let hasPoints = false;
        for (const photo of photos) {
            const lat = photo.metadata?.lat;
            const lon = photo.metadata?.lon;
            if (lat && lon) {
                bounds.extend([lon, lat]);
                hasPoints = true;
            }
        }

        if (!hasPoints) return;

        const doFit = () => {
            map?.fitBounds(bounds, { padding: 100, maxZoom: 15 });
        };

        if (delay) {
            setTimeout(doFit, 150);
        } else {
            doFit();
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

        // Check clusters first
        const clusterFeatures = map.queryRenderedFeatures(bbox, {
            layers: [CLUSTER_THUMB_LAYER_ID, CLUSTER_LAYER_ID, CLUSTER_COUNT_LAYER_ID]
        });
        if (clusterFeatures.length > 0) {
            const feature = closestFeature(clusterFeatures, e.point);
            const clusterId = feature.properties?.cluster_id;
            if (clusterId !== undefined) {
                const source = map.getSource(SOURCE_ID) as maplibregl.GeoJSONSource;
                source.getClusterChildren(clusterId).then((children) => {
                    if (!map || children.length === 0) return;
                    const bounds = new maplibregl.LngLatBounds();
                    for (const child of children) {
                        const coords = (child.geometry as GeoJSON.Point).coordinates as [number, number];
                        bounds.extend(coords);
                    }
                    map.fitBounds(bounds, {
                        padding: 80,
                        maxZoom: 15,
                        duration: 300
                    });
                });
            }
            return;
        }

        // Check unclustered photos
        const photoFeatures = map.queryRenderedFeatures(bbox, {
            layers: [UNCLUSTERED_THUMB_LAYER_ID, UNCLUSTERED_LAYER_ID, UNCLUSTERED_BORDER_LAYER_ID]
        });
        if (photoFeatures.length > 0) {
            const feature = closestFeature(photoFeatures, e.point);
            const path = feature.properties?.path;
            if (!path) return;

            const photo = photoByPath.get(path);
            if (!photo) return;

            if (onOpenPreview) {
                onOpenPreview(photo, visiblePhotosSorted);
            }
        }
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

    function showHoverPopup(feature: maplibregl.GeoJSONFeature) {
        if (!map || !hoverPopup) return;

        const path = feature.properties?.path;
        hoveredPhotoPath = path;

        const photo = photoByPath.get(path);
        if (!photo) return;

        const coords = (feature.geometry as GeoJSON.Point).coordinates as [number, number];
        const thumbPath = photo.thumb_path || photo.path;
        const url = getThumbnailUrl(thumbPath);
        const originalUrl = getThumbnailUrl(photo.path);
        const filename = photo.path.split('/').pop() || '';
        const dateTaken = photo.metadata?.date_taken?.replace(/"/g, '') || '';
        const rawBadge = photo.hasRaw ? '<div class="popup-raw-badge">RAW</div>' : '';
        const dateInfo = dateTaken ? `<div class="popup-date">${dateTaken}</div>` : '';

        const html = `
            <div class="photo-popup">
                <img src="${url}" alt="${filename}" onerror="this.onerror=null; this.src='${originalUrl}';" />
                <div class="popup-info">
                    <div class="popup-filename">${filename}</div>
                    ${dateInfo}
                </div>
                ${rawBadge}
            </div>
        `;

        hoverPopup.setLngLat(coords).setHTML(html).addTo(map);
    }

    function hideHoverPopup() {
        hoveredPhotoPath = null;
        if (hoverPopup) {
            hoverPopup.remove();
        }
    }

    // ============================================================
    // Map ↔ Timeline Sync
    // ============================================================

    /**
     * Get time range of photos visible in current map bounds.
     */
    function getVisiblePhotosTimeRange(): { start: Date; end: Date; photos: any[] } | null {
        if (!map || allPhotosData.length === 0) return null;

        const bounds = map.getBounds();
        const visibleDates: Date[] = [];
        const visiblePhotos: any[] = [];

        for (const photo of allPhotosData) {
            const lat = photo.metadata?.lat;
            const lon = photo.metadata?.lon;
            if (!lat || !lon) continue;

            if (bounds.contains(new maplibregl.LngLat(lon, lat))) {
                visiblePhotos.push(photo);
                const date = dateByPath.get(photo.path);
                if (date) visibleDates.push(date);
            }
        }

        if (visibleDates.length === 0) return null;

        visibleDates.sort((a, b) => a.getTime() - b.getTime());
        return {
            start: visibleDates[0],
            end: visibleDates[visibleDates.length - 1],
            photos: visiblePhotos
        };
    }

    // Debounce timer for map move → timeline sync
    let mapMoveDebounceTimer: ReturnType<typeof setTimeout> | null = null;

    function handleMapMove() {
        if (syncSource === 'timeline') return;

        if (mapMoveDebounceTimer) clearTimeout(mapMoveDebounceTimer);
        mapMoveDebounceTimer = setTimeout(() => {
            mapMoveDebounceTimer = null;
            if (syncSource === 'timeline') return;

            const result = getVisiblePhotosTimeRange();
            if (result) {
                syncSource = 'map';
                mapVisibleTimeRange = { start: result.start, end: result.end };
                mapVisiblePhotos = result.photos;
                setTimeout(() => {
                    syncSource = null;
                }, 100);
            }
        }, 300);
    }

    // Time range from map view (for timeline sync)
    let mapVisibleTimeRange = $state<{ start: Date; end: Date } | null>(null);
    let mapVisiblePhotos = $state<any[] | null>(null);
    let timelinePhotos = $derived(mapVisiblePhotos ?? photosWithMarkers);

    // Fit map to show all photos
    function handleShowAllPhotos() {
        if (!map || allPhotosData.length === 0) return;
        fitBoundsToPhotos(allPhotosData, false);
    }

    // ============================================================
    // Map Style
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

        // Default: CARTO tiles with theme support (@2x for desktop Retina)
        const tileType = isDark ? 'dark_all' : 'light_all';
        const sourceId = isDark ? 'carto-dark' : 'carto-light';
        return {
            version: 8,
            glyphs: GLYPHS_URL,
            sources: {
                [sourceId]: {
                    type: 'raster',
                    tiles: [
                        `https://a.basemaps.cartocdn.com/${tileType}/{z}/{x}/{y}@2x.png`,
                        `https://b.basemaps.cartocdn.com/${tileType}/{z}/{x}/{y}@2x.png`,
                        `https://c.basemaps.cartocdn.com/${tileType}/{z}/{x}/{y}@2x.png`,
                        `https://d.basemaps.cartocdn.com/${tileType}/{z}/{x}/{y}@2x.png`
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
            map.on('idle', loadVisibleThumbnails);
            map.on('styleimagemissing', handleMissingImage);

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

        // Map move → timeline sync (outside of onMapReady since it uses moveend)
        map.on('moveend', handleMapMove);
    }

    onDestroy(() => {
        loadedImages.clear();
        loadingImages.clear();
        thumbDataCache.clear();
        photoByPath.clear();
        dateByPath.clear();

        if (mapMoveDebounceTimer) {
            clearTimeout(mapMoveDebounceTimer);
            mapMoveDebounceTimer = null;
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
            lastGeoJSONFingerprint = '';

            // Remove handlers before style change to prevent duplicates
            map.off('click', handleMapClick);
            map.off('mousemove', handleMouseMove);
            map.off('idle', loadVisibleThumbnails);
            map.off('styleimagemissing', handleMissingImage);
            map.off('moveend', handleMapMove);
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
                updateSourceData(allPhotosData, timeFilterStart, timeFilterEnd);

                mapRef.on('click', handleMapClick);
                mapRef.on('mousemove', handleMouseMove);
                mapRef.on('idle', loadVisibleThumbnails);
                mapRef.on('styleimagemissing', handleMissingImage);
                mapRef.on('moveend', handleMapMove);
                mapRef.on('movestart', handleDragStart);
                mapRef.on('moveend', handleDragEnd);
            }
            requestAnimationFrame(onStyleReady);
        } catch (_) {
            // Style change failed — non-critical
        }
    });

    // Update map data when photos change
    $effect(() => {
        const currentMap = map;
        const geo = geotaggedPhotos;
        const loaded = mapLoaded;

        if (!currentMap || !loaded) return;

        initializePhotos(geo);
    });

    // Update GeoJSON source when time filter changes (rAF throttled)
    let pendingSourceUpdateRaf: number | null = null;
    $effect(() => {
        const start = timeFilterStart;
        const end = timeFilterEnd;
        const loaded = mapLoaded;

        if (!loaded || allPhotosData.length === 0) return;

        if (pendingSourceUpdateRaf !== null) cancelAnimationFrame(pendingSourceUpdateRaf);
        pendingSourceUpdateRaf = requestAnimationFrame(() => {
            pendingSourceUpdateRaf = null;
            updateSourceData(allPhotosData, start, end);
        });
        return () => {
            if (pendingSourceUpdateRaf !== null) {
                cancelAnimationFrame(pendingSourceUpdateRaf);
                pendingSourceUpdateRaf = null;
            }
        };
    });

    // ============================================================
    // Box Selection (Desktop only)
    // ============================================================

    let boxSelectedTimeRange = $state<{ start: Date; end: Date } | null>(null);

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

        // Find photos within bounds using allPhotosData (no DOM markers)
        const photosInBounds: Date[] = [];
        for (const photo of allPhotosData) {
            const lat = photo.metadata?.lat;
            const lon = photo.metadata?.lon;
            if (!lat || !lon) continue;
            if (bounds.contains(new maplibregl.LngLat(lon, lat))) {
                const date = dateByPath.get(photo.path);
                if (date) photosInBounds.push(date);
            }
        }

        if (photosInBounds.length > 0) {
            photosInBounds.sort((a, b) => a.getTime() - b.getTime());
            boxSelectedTimeRange = {
                start: photosInBounds[0],
                end: photosInBounds[photosInBounds.length - 1]
            };

            map.fitBounds(bounds, { padding: 100, animate: true });
        }

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
                <div class="px-5 py-2 text-base theme-bg-card backdrop-blur-sm rounded-full theme-text-primary font-medium shadow-lg flex items-center gap-1.5">
                    <i class="fa-regular fa-calendar text-[var(--accent)] flex-shrink-0"></i>
                    <span class="tabular-nums whitespace-nowrap">{timeRangeDisplay}</span>
                </div>
            </div>
        {/if}

        <!-- No geotagged photos message -->
        {#if photos.length > 0 && !hasGeotaggedPhotos}
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
            {#if map && isReady && photosWithMarkers.length > 0}
                <TimelineSlider
                    photos={timelinePhotos}
                    externalTimeRange={boxSelectedTimeRange}
                    mapViewTimeRange={mapVisibleTimeRange}
                    onTimeRangeChange={handleTimeRangeChange}
                    onExternalRangeConsumed={() => { boxSelectedTimeRange = null; }}
                    onMapRangeConsumed={() => { mapVisibleTimeRange = null; }}
                    onShowAll={handleShowAllPhotos}
                    {t}
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
