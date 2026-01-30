<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import maplibregl from "maplibre-gl";
    import "maplibre-gl/dist/maplibre-gl.css";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import TimelineSlider from "./TimelineSlider.svelte";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";

    let { photos, onOpenPreview, theme = "dark", tileConfig, t, locale } = $props<{
        photos: any[];
        onOpenPreview?: (photo: any, visiblePhotos: any[]) => void;
        theme?: "dark" | "light";
        tileConfig?: { tiles: string[]; tileSize: number };
        t: Translations;
        locale: Locale;
    }>();

    let mapContainer: HTMLDivElement;
    let map = $state<maplibregl.Map | null>(null);
    let mapLoaded = $state(false);
    let isReady = $state(false);

    // Time filter state
    let timeFilterStart = $state<Date | null>(null);
    let timeFilterEnd = $state<Date | null>(null);

    // Sync mode: prevent circular updates
    let syncSource = $state<'map' | 'timeline' | null>(null);



    // Resize observer
    let resizeObserver: ResizeObserver | null = null;

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
        // If this change was triggered by map pan sync, skip GeoJSON data update.
        // Map panning should only update the timeline display (via mapViewTimeRange prop),
        // NOT filter GeoJSON data. Filtering on pan causes setData() → cluster ID regeneration
        // → thumbnails disappear → white circle flash.
        if (syncSource === 'map') {
            syncSource = null;
            return;
        }
        syncSource = 'timeline';
        timeFilterStart = start;
        timeFilterEnd = end;
        // Reset sync source after a short delay
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

    let timeRangeDisplay = $derived(formatTimeRange(timeFilterStart, timeFilterEnd, true));

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

    // Thumbnail rendering constants
    const THUMB_CANVAS_SIZE = 88;    // Canvas pixels (renders at 44px with pixelRatio 2)
    const THUMB_BORDER_WIDTH = 4;    // Border width in canvas pixels
    const THUMB_PIXEL_RATIO = 2;     // For retina displays

    // Track previous photo count to detect add vs remove
    let prevPhotoCount = 0;

    // Track loaded thumbnail map images (imageId -> loaded status)
    let loadedImages = new Set<string>();
    // Track images currently being loaded (prevent duplicate loads)
    let loadingImages = new Set<string>();
    // Cache rendered ImageData by thumbPath — avoids re-fetch + re-canvas-render
    // LRU eviction: Map insertion order = oldest first. On access, delete+re-insert.
    let thumbDataCache = new Map<string, ImageData>();
    const THUMB_CACHE_MAX = 120;   // Max ImageData entries in JS heap (~3.6MB)
    const ATLAS_BUDGET = 180;      // Max images in MapLibre GPU atlas (~5.5MB)
    // GeoJSON fingerprint to skip redundant source.setData() calls
    let lastGeoJSONFingerprint = '';

    /** LRU get from thumbDataCache — moves entry to end (most recent). */
    function thumbCacheGet(key: string): ImageData | undefined {
        const val = thumbDataCache.get(key);
        if (val !== undefined) {
            // Move to end (most recently used)
            thumbDataCache.delete(key);
            thumbDataCache.set(key, val);
        }
        return val;
    }

    /** LRU set into thumbDataCache — evicts oldest if over limit. */
    function thumbCacheSet(key: string, value: ImageData) {
        if (thumbDataCache.has(key)) {
            thumbDataCache.delete(key);
        }
        thumbDataCache.set(key, value);
        // Evict oldest entries if over limit
        while (thumbDataCache.size > THUMB_CACHE_MAX) {
            const oldest = thumbDataCache.keys().next().value;
            if (oldest !== undefined) thumbDataCache.delete(oldest);
            else break;
        }
    }

    /**
     * Evict non-visible images from MapLibre's GPU atlas to stay under budget.
     * Called on 'idle' after loading visible thumbnails.
     * Safe: evicted images are re-loaded on demand via styleimagemissing handler.
     */
    function evictNonVisibleImages() {
        if (!map) return;
        if (loadedImages.size <= ATLAS_BUDGET) return;

        // Collect visible image IDs from rendered features
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

        // Remove non-visible images until under budget
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
     * Called once after map load.
     */
    function setupClusterLayers() {
        if (!map) return;
        // Guard against duplicate calls
        if (map.getSource(SOURCE_ID)) return;

        // Add clustered GeoJSON source
        map.addSource(SOURCE_ID, {
            type: 'geojson',
            data: { type: 'FeatureCollection', features: [] },
            cluster: true,
            clusterMaxZoom: 14,  // Cluster up to zoom 14
            clusterRadius: 50,   // Cluster radius in pixels
        });

        const isDark = theme === 'dark';

        // Cluster circles layer — white border behind thumbnail.
        // Radius = thumbnail apparent radius + 3px border.
        // Thumbnail: 44px CSS diameter (radius 22) * icon-size → apparent radius.
        // icon-size: 1.0→22, 1.15→25.3, 1.3→28.6, 1.45→31.9, 1.6→35.2
        // border radii: +3 each → 25, 28, 32, 35, 38
        map.addLayer({
            id: CLUSTER_LAYER_ID,
            type: 'circle',
            source: SOURCE_ID,
            filter: ['has', 'point_count'],
            paint: {
                'circle-radius': [
                    'step', ['get', 'point_count'],
                    25,       // < 10
                    10, 28,   // 10+
                    50, 32,   // 50+
                    100, 35,  // 100+
                    500, 38   // 500+
                ],
                'circle-color': isDark ? '#ffffff' : '#1e293b',
                'circle-stroke-width': 0,
            }
        });

        // Cluster thumbnail symbol layer — shows the most recent photo in the cluster.
        // Images are loaded asynchronously via getClusterLeaves on idle.
        // icon-image uses "cluster:{cluster_id}" as the key.
        map.addLayer({
            id: CLUSTER_THUMB_LAYER_ID,
            type: 'symbol',
            source: SOURCE_ID,
            filter: ['has', 'point_count'],
            layout: {
                'icon-image': ['concat', 'cluster:', ['to-string', ['get', 'cluster_id']]],
                'icon-size': [
                    'step', ['get', 'point_count'],
                    1.0,       // default (count < 10)
                    10, 1.15,  // 10+
                    50, 1.3,   // 50+
                    100, 1.45, // 100+
                    500, 1.6   // 500+
                ],
                'icon-allow-overlap': true,
                'icon-ignore-placement': true,
            }
        });

        // Cluster count text (symbol layer — with halo for readability over thumbnail)
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
                    13,      // default
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

        // Unclustered photo dots (circle layer — GPU accelerated)
        map.addLayer({
            id: UNCLUSTERED_BORDER_LAYER_ID,
            type: 'circle',
            source: SOURCE_ID,
            filter: ['!', ['has', 'point_count']],
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
            filter: ['!', ['has', 'point_count']],
            paint: {
                'circle-radius': 20,
                'circle-color': isDark
                    ? 'rgba(99, 102, 241, 0.9)'
                    : 'rgba(79, 70, 229, 0.9)',
                'circle-stroke-width': 2,
                'circle-stroke-color': isDark ? '#ffffff' : '#1e293b',
            }
        });

        // Thumbnail symbol layer — renders circular photo thumbnails on top of circle layers.
        // Images are loaded asynchronously; circle layers serve as placeholder until loaded.
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
     * Update the GeoJSON source data (e.g., when photos change or time filter changes).
     */
    function updateSourceData(photos: any[], start: Date | null = null, end: Date | null = null) {
        if (!map) return;
        const source = map.getSource(SOURCE_ID) as maplibregl.GeoJSONSource | undefined;
        if (!source) return;

        const geojson = buildGeoJSON(photos, start, end);

        // Content-based fingerprint: skip setData if the actual photo set hasn't changed.
        // Uses count + sampled paths instead of time ranges, so that identical photo sets
        // with slightly different time ranges don't trigger unnecessary cluster regeneration.
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

        // Data is changing — clear cluster tracking sets (but DON'T removeImage from atlas).
        // Old cluster images stay in the atlas and render instantly if the same cluster ID reappears.
        for (const imageId of loadedImages) {
            if (imageId.startsWith('cluster:')) loadedImages.delete(imageId);
        }
        for (const imageId of loadingImages) {
            if (imageId.startsWith('cluster:')) loadingImages.delete(imageId);
        }

        source.setData(geojson);
    }

    // ============================================================
    // Thumbnail Loading for Unclustered Points
    // ============================================================

    /**
     * Draw a circular thumbnail image onto a canvas.
     * @param img - Source image element
     * @param borderWidth - Border width in canvas pixels (0 = no border, used for clusters)
     * Returns ImageData ready for map.addImage().
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

        // Draw border (only if borderWidth > 0)
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
     * Load a single thumbnail image and add it to the map.
     */
    /**
     * Add an image to the map from cache or by fetching.
     * Uses thumbDataCache to avoid re-fetch + re-canvas-render.
     */
    function addImageToMap(imageId: string, thumbPath: string, borderWidth: number = THUMB_BORDER_WIDTH) {
        if (!map) return;

        // Fast-path: image already in map atlas
        if (map.hasImage(imageId)) {
            loadedImages.add(imageId);
            loadingImages.delete(imageId);
            return;
        }

        // Check ImageData cache (same thumbPath, possibly different imageId for cluster vs unclustered)
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

        // Fetch and render
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

    function loadThumbnailForMap(imageId: string, thumbPath: string, _originalPath: string) {
        if (loadingImages.has(imageId) || loadedImages.has(imageId)) return;
        // Fast-path: already in atlas from a previous load
        if (map?.hasImage(imageId)) { loadedImages.add(imageId); return; }
        loadingImages.add(imageId);
        addImageToMap(imageId, thumbPath, THUMB_BORDER_WIDTH);
    }

    /**
     * Handle missing image event from MapLibre.
     * Fires immediately when the renderer encounters an icon-image id not in the atlas.
     * This provides near-instant thumbnail loading after setData() regenerates cluster IDs.
     */
    function handleMissingImage(e: { id: string }) {
        const imageId = e.id;
        if (!map) return;

        // Skip if already loading
        if (loadingImages.has(imageId)) return;

        if (imageId.startsWith('cluster:')) {
            // Extract cluster ID and load its thumbnail
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

                // Find leaf with latest date_ts
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
            // Individual photo thumbnail
            const thumbPath = imageId.replace('thumb:', '');
            loadingImages.add(imageId);
            addImageToMap(imageId, thumbPath, THUMB_BORDER_WIDTH);
        }
    }

    /**
     * Scan visible features (clusters + unclustered) and load thumbnails.
     * Called on map 'idle' event (after all tiles/rendering finishes).
     */
    function loadVisibleThumbnails() {
        if (!map) return;

        // 1. Load thumbnails for visible clusters
        loadClusterThumbnails();

        // 2. Load thumbnails for visible unclustered photo features
        let features: maplibregl.MapGeoJSONFeature[];
        try {
            features = map.queryRenderedFeatures(undefined, {
                layers: [UNCLUSTERED_LAYER_ID]
            });
        } catch (_) {
            return; // Layer might not exist yet
        }

        if (features.length === 0) {
            // No visible features, but still run eviction
            evictNonVisibleImages();
            return;
        }

        // Limit batch size to avoid blocking
        let loaded = 0;
        const BATCH_SIZE = 15;

        for (const feature of features) {
            if (loaded >= BATCH_SIZE) break;

            const imageId = feature.properties?.imageId;
            if (!imageId) continue;
            // Skip already loaded or loading
            if (loadedImages.has(imageId) || loadingImages.has(imageId)) continue;

            const thumbPath = feature.properties?.thumb_path;
            const originalPath = feature.properties?.path;
            if (!thumbPath) continue;

            loadThumbnailForMap(imageId, thumbPath, originalPath);
            loaded++;
        }

        // 3. Evict non-visible images to keep GPU memory under budget
        evictNonVisibleImages();
    }

    /**
     * Scan visible clusters and load a thumbnail for each (most recent photo).
     * Called on map 'idle' event.
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

            // Fast-path: image already in atlas from a previous load (e.g., same cluster ID after pan)
            if (map.hasImage(imageId)) {
                loadedImages.add(imageId);
                continue;
            }

            loadingImages.add(imageId);

            // Get a sample of leaves to find the most recent photo
            source.getClusterLeaves(clusterId, 20, 0).then((leaves) => {
                if (!map || leaves.length === 0) {
                    loadingImages.delete(imageId);
                    return;
                }

                // Find leaf with latest date_ts
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

                // Use shared addImageToMap (with cache + fast-path)
                addImageToMap(imageId, thumbPath, 0);
            }).catch(() => {
                loadingImages.delete(imageId);
            });
        }
    }

    /**
     * Initialize photos on the map.
     * Called when photos prop changes.
     */
    function initializePhotos(photos: any[]) {
        if (!map || !mapLoaded) return;

        // Debounce: skip if same photo count as last call (rapid re-renders)
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

        // Update GeoJSON source
        updateSourceData(photos, timeFilterStart, timeFilterEnd);

        // Fit bounds only on initial load or when adding new photos
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
    // Interaction Handlers
    // ============================================================

    /**
     * Global click handler using queryRenderedFeatures.
     * Layer-specific click handlers don't reliably fire on Android WebView touch events,
     * so we use a single global handler and manually query features at the click point.
     */
    function handleMapClick(e: maplibregl.MapMouseEvent) {
        if (!map) return;

        // Use a generous tolerance for touch events (fat finger)
        const tolerance = 15;
        const bbox: [maplibregl.PointLike, maplibregl.PointLike] = [
            [e.point.x - tolerance, e.point.y - tolerance],
            [e.point.x + tolerance, e.point.y + tolerance]
        ];

        // Find the feature closest to the tap point among candidates
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

        // Check clusters first (higher priority — check all cluster layers)
        const clusterFeatures = map.queryRenderedFeatures(bbox, {
            layers: [CLUSTER_THUMB_LAYER_ID, CLUSTER_LAYER_ID, CLUSTER_COUNT_LAYER_ID]
        });
        if (clusterFeatures.length > 0) {
            const feature = closestFeature(clusterFeatures, e.point);
            const clusterId = feature.properties?.cluster_id;
            if (clusterId !== undefined) {
                const source = map.getSource(SOURCE_ID) as maplibregl.GeoJSONSource;
                // Use getClusterChildren to compute bounding box for accurate zoom
                source.getClusterChildren(clusterId).then((children) => {
                    if (!map || children.length === 0) return;
                    // Compute bounds that contain all children
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

        // Check unclustered photo points (including thumbnail symbol layer)
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
            return;
        }
    }



    // ============================================================
    // Map sync (for timeline)
    // ============================================================

    /**
     * Get time range of photos visible in current map bounds.
     */
    function getVisiblePhotosTimeRange(): { start: Date; end: Date } | null {
        if (!map || allPhotosData.length === 0) return null;

        const bounds = map.getBounds();
        const visibleDates: Date[] = [];

        for (const photo of allPhotosData) {
            const lat = photo.metadata?.lat;
            const lon = photo.metadata?.lon;
            if (!lat || !lon) continue;

            if (bounds.contains(new maplibregl.LngLat(lon, lat))) {
                const date = dateByPath.get(photo.path);
                if (date) visibleDates.push(date);
            }
        }

        if (visibleDates.length === 0) return null;

        visibleDates.sort((a, b) => a.getTime() - b.getTime());
        return {
            start: visibleDates[0],
            end: visibleDates[visibleDates.length - 1]
        };
    }

    // Debounce timer for map move → timeline sync
    let mapMoveDebounceTimer: ReturnType<typeof setTimeout> | null = null;

    function handleMapMove() {
        if (syncSource === 'timeline') return;

        // Debounce: during continuous drag/inertia, only fire after 300ms idle.
        // This avoids ~5 × O(n) recalculations per moveend event in TimelineSlider.
        if (mapMoveDebounceTimer) clearTimeout(mapMoveDebounceTimer);
        mapMoveDebounceTimer = setTimeout(() => {
            mapMoveDebounceTimer = null;
            if (syncSource === 'timeline') return;

            const timeRange = getVisiblePhotosTimeRange();
            if (timeRange) {
                syncSource = 'map';
                mapVisibleTimeRange = timeRange;
                setTimeout(() => {
                    syncSource = null;
                }, 100);
            }
        }, 300);
    }

    let mapVisibleTimeRange = $state<{ start: Date; end: Date } | null>(null);

    function handleShowAllPhotos() {
        if (!map || allPhotosData.length === 0) return;
        fitBoundsToPhotos(allPhotosData, false);
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

    onMount(() => {
        if (!mapContainer) return;

        map = new maplibregl.Map({
            container: mapContainer,
            style: getMapStyle(theme === 'dark'),
            center: [0, 20],
            zoom: 2,
            attributionControl: false
        });

        setupMapControls();
    });

    function setupMapControls() {
        if (!map) return;

        // Show All Photos control
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

        // Handle resize
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

            // Setup cluster layers
            setupClusterLayers();

            // Register interaction handlers
            // Use global click handler with queryRenderedFeatures for reliable
            // touch event handling on Android WebView
            map.on('click', handleMapClick);

            // Load thumbnail images for visible unclustered points
            map.on('idle', loadVisibleThumbnails);

            // Eagerly load missing images when MapLibre tries to render them.
            // This fires before idle, so cluster/photo thumbnails appear faster
            // after setData() regenerates cluster IDs.
            map.on('styleimagemissing', handleMissingImage);

            // Ready for timeline
            requestAnimationFrame(() => {
                isReady = true;
            });
        }

        // Handle both cases: map already loaded or not yet
        if (map.loaded()) {
            onMapReady();
        } else {
            map.on('load', onMapReady);
            // style.load fires before load on some WebView versions
            map.on('style.load', () => {
                if (!mapLoaded) onMapReady();
            });
        }

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
        if (map) {
            map.remove();
            map = null;
        }
    });

    // ============================================================
    // Reactive Effects
    // ============================================================

    // Update map style when theme changes
    let prevTheme: string = theme; // Initialize to current theme to avoid false change on first render
    $effect(() => {
        const currentTheme = theme;
        if (!map || !mapLoaded) return;
        if (currentTheme === prevTheme) return;
        prevTheme = currentTheme;

        const newStyle = getMapStyle(currentTheme === 'dark');

        try {
            // Theme changed — thumbnail border color depends on theme, so clear all caches
            loadedImages.clear();
            loadingImages.clear();
            thumbDataCache.clear();
            lastGeoJSONFingerprint = '';

            map.setStyle(newStyle);
            // After style change, re-add layers and data
            map.once('style.load', () => {
                setupClusterLayers();
                updateSourceData(allPhotosData, timeFilterStart, timeFilterEnd);

                // Re-register interaction handlers after style change
                if (map) {
                    map.on('click', handleMapClick);
                    map.on('idle', loadVisibleThumbnails);
                    map.on('styleimagemissing', handleMissingImage);
                }
            });
        } catch (e) {
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

    // Update GeoJSON source when time filter changes (instead of toggling DOM visibility).
    // Uses rAF throttle: if rapid filter changes arrive (e.g. during timeline pinch),
    // only the latest one executes per animation frame, avoiding redundant buildGeoJSON + setData.
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
            <div class="absolute top-4 left-1/2 -translate-x-1/2 z-[1000] pointer-events-none max-w-[95%]">
                <div class="px-3 py-1.5 text-[11px] theme-bg-card backdrop-blur-sm rounded-full theme-text-primary font-medium shadow-lg flex items-center gap-1.5">
                    <i class="fa-regular fa-calendar text-[var(--accent)] text-[10px] flex-shrink-0"></i>
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
                    photos={photosWithMarkers}
                    mapViewTimeRange={mapVisibleTimeRange}
                    onTimeRangeChange={handleTimeRangeChange}
                    onMapRangeConsumed={() => { mapVisibleTimeRange = null; }}
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
