<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import maplibregl from "maplibre-gl";
    import "maplibre-gl/dist/maplibre-gl.css";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import TimelineSlider from "./TimelineSlider.svelte";

    let { photos, onOpenPreview } = $props<{
        photos: any[];
        onOpenPreview?: (photo: any, visiblePhotos: any[]) => void;
    }>();

    let mapContainer: HTMLDivElement;
    let map = $state<maplibregl.Map | null>(null);
    let isReady = $state(false);

    // Time filter state
    let timeFilterStart = $state<Date | null>(null);
    let timeFilterEnd = $state<Date | null>(null);

    // Box selection state
    let isBoxSelectMode = $state(false);
    let isDrawingBox = $state(false);
    let boxStart: { x: number; y: number } | null = null;
    let selectionBox: HTMLDivElement | null = null;

    // Hover popup
    let hoverPopup: maplibregl.Popup | null = null;

    // Photo data indexed by id
    let photoIndex: Map<string, any> = new Map();

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
        timeFilterStart = start;
        timeFilterEnd = end;
    }

    // Format time range for display
    function formatTimeRange(start: Date | null, end: Date | null): string {
        if (!start || !end) return '';
        const sameDay = start.toDateString() === end.toDateString();
        const formatDate = (d: Date) => d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
        const formatTime = (d: Date) => d.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' });
        if (sameDay) {
            return `${formatDate(start)} ${formatTime(start)} - ${formatTime(end)}`;
        }
        return `${formatDate(start)} - ${formatDate(end)}`;
    }

    let timeRangeDisplay = $derived(formatTimeRange(timeFilterStart, timeFilterEnd));

    // Photos with markers for TimelineSlider
    let photosWithMarkers = $state<any[]>([]);

    // Get visible photos sorted by time for navigation
    let visiblePhotosSorted = $derived.by(() => {
        const start = timeFilterStart;
        const end = timeFilterEnd;

        const visible: { photo: any; date: Date }[] = [];
        for (const photo of geotaggedPhotos) {
            const date = parsePhotoDate(photo.metadata?.date_taken);
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

    // Convert photos to GeoJSON
    function photosToGeoJSON(photos: any[]): GeoJSON.FeatureCollection {
        const features: GeoJSON.Feature[] = [];

        for (const photo of photos) {
            const lat = photo.metadata?.lat;
            const lon = photo.metadata?.lon;
            if (!lat || !lon) continue;

            const date = parsePhotoDate(photo.metadata?.date_taken);
            const timestamp = date?.getTime() || 0;

            features.push({
                type: 'Feature',
                geometry: {
                    type: 'Point',
                    coordinates: [lon, lat]
                },
                properties: {
                    id: photo.path,
                    thumbnail: getThumbnailUrl(photo.thumb_path || photo.path),
                    filename: photo.path.split('/').pop(),
                    dateTaken: photo.metadata?.date_taken?.replace(/"/g, '') || '',
                    timestamp,
                    hasRaw: photo.hasRaw || false
                }
            });

            photoIndex.set(photo.path, photo);
        }

        return {
            type: 'FeatureCollection',
            features
        };
    }

    // Filter GeoJSON by time range
    function filterGeoJSONByTime(geojson: GeoJSON.FeatureCollection, start: Date | null, end: Date | null): GeoJSON.FeatureCollection {
        if (!start || !end) return geojson;

        const startTime = start.getTime();
        const endTime = end.getTime();

        return {
            type: 'FeatureCollection',
            features: geojson.features.filter(f => {
                const ts = f.properties?.timestamp;
                return ts && ts >= startTime && ts <= endTime;
            })
        };
    }

    let fullGeoJSON: GeoJSON.FeatureCollection = { type: 'FeatureCollection', features: [] };

    onMount(() => {
        if (!mapContainer) return;

        // Initialize MapLibre map
        map = new maplibregl.Map({
            container: mapContainer,
            style: {
                version: 8,
                sources: {
                    'carto-dark': {
                        type: 'raster',
                        tiles: [
                            'https://a.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png',
                            'https://b.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png',
                            'https://c.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png',
                            'https://d.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png'
                        ],
                        tileSize: 256,
                        attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> &copy; <a href="https://carto.com/attributions">CARTO</a>'
                    }
                },
                layers: [
                    {
                        id: 'carto-dark-layer',
                        type: 'raster',
                        source: 'carto-dark',
                        minzoom: 0,
                        maxzoom: 20
                    }
                ]
            },
            center: [0, 20],
            zoom: 2,
            attributionControl: false
        });

        // Add controls
        map.addControl(new maplibregl.NavigationControl(), 'top-right');
        map.addControl(new maplibregl.AttributionControl({ compact: true }), 'bottom-right');

        // Create hover popup
        hoverPopup = new maplibregl.Popup({
            closeButton: false,
            closeOnClick: false,
            maxWidth: '300px',
            offset: [0, -20]
        });

        map.on('load', () => {
            if (!map) return;

            // Add photos source with clustering
            map.addSource('photos', {
                type: 'geojson',
                data: { type: 'FeatureCollection', features: [] },
                cluster: true,
                clusterMaxZoom: 17,
                clusterRadius: 60
            });

            // Cluster circles layer
            map.addLayer({
                id: 'clusters',
                type: 'circle',
                source: 'photos',
                filter: ['has', 'point_count'],
                paint: {
                    'circle-color': [
                        'step',
                        ['get', 'point_count'],
                        'rgba(30, 41, 59, 0.9)',  // < 20: dark gray
                        20, 'rgba(99, 102, 241, 0.9)',  // 20-99: purple
                        100, 'rgba(139, 92, 246, 0.9)'  // 100+: violet
                    ],
                    'circle-radius': [
                        'step',
                        ['get', 'point_count'],
                        14,   // < 20: small
                        20, 22,  // 20-99: medium
                        100, 28  // 100+: large
                    ],
                    'circle-stroke-width': 3,
                    'circle-stroke-color': 'rgba(255, 255, 255, 0.9)'
                }
            });

            // Cluster count labels
            map.addLayer({
                id: 'cluster-count',
                type: 'symbol',
                source: 'photos',
                filter: ['has', 'point_count'],
                layout: {
                    'text-field': '{point_count_abbreviated}',
                    'text-font': ['Open Sans Bold', 'Arial Unicode MS Bold'],
                    'text-size': [
                        'step',
                        ['get', 'point_count'],
                        11,
                        20, 13,
                        100, 15
                    ]
                },
                paint: {
                    'text-color': '#ffffff'
                }
            });

            // Unclustered photo markers
            map.addLayer({
                id: 'unclustered-point',
                type: 'circle',
                source: 'photos',
                filter: ['!', ['has', 'point_count']],
                paint: {
                    'circle-color': '#1e293b',
                    'circle-radius': 22,
                    'circle-stroke-width': 2,
                    'circle-stroke-color': '#ffffff'
                }
            });

            // Click on cluster to zoom
            map.on('click', 'clusters', async (e) => {
                if (!map) return;
                const features = map.queryRenderedFeatures(e.point, { layers: ['clusters'] });
                if (!features.length) return;

                const clusterId = features[0].properties?.cluster_id;
                const source = map.getSource('photos') as maplibregl.GeoJSONSource;

                try {
                    const zoom = await source.getClusterExpansionZoom(clusterId);
                    const coords = (features[0].geometry as GeoJSON.Point).coordinates;
                    map.easeTo({
                        center: coords as [number, number],
                        zoom: zoom
                    });
                } catch (err) {
                    console.error('Error expanding cluster:', err);
                }
            });

            // Click on unclustered point to open preview
            map.on('click', 'unclustered-point', (e) => {
                if (!e.features?.length) return;
                const props = e.features[0].properties;
                const photo = photoIndex.get(props?.id);
                if (photo && onOpenPreview) {
                    onOpenPreview(photo, visiblePhotosSorted);
                }
            });

            // Hover on unclustered point to show preview
            map.on('mouseenter', 'unclustered-point', (e) => {
                if (!map || !hoverPopup || !e.features?.length) return;
                map.getCanvas().style.cursor = 'pointer';

                const feature = e.features[0];
                const coords = (feature.geometry as GeoJSON.Point).coordinates.slice() as [number, number];
                const props = feature.properties;

                const rawBadge = props?.hasRaw ? '<div class="popup-raw-badge">RAW</div>' : '';
                const dateInfo = props?.dateTaken ? `<div class="popup-date">${props.dateTaken}</div>` : '';

                const html = `
                    <div class="photo-popup">
                        <img src="${props?.thumbnail}" alt="${props?.filename}" />
                        <div class="popup-info">
                            <div class="popup-filename">${props?.filename}</div>
                            ${dateInfo}
                        </div>
                        ${rawBadge}
                    </div>
                `;

                hoverPopup.setLngLat(coords).setHTML(html).addTo(map);
            });

            map.on('mouseleave', 'unclustered-point', () => {
                if (!map || !hoverPopup) return;
                map.getCanvas().style.cursor = '';
                hoverPopup.remove();
            });

            // Hover cursor for clusters
            map.on('mouseenter', 'clusters', () => {
                if (map) map.getCanvas().style.cursor = 'pointer';
            });
            map.on('mouseleave', 'clusters', () => {
                if (map) map.getCanvas().style.cursor = '';
            });

            // Ready for timeline
            requestAnimationFrame(() => {
                isReady = true;
            });
        });
    });

    onDestroy(() => {
        if (hoverPopup) {
            hoverPopup.remove();
            hoverPopup = null;
        }
        if (map) {
            map.remove();
            map = null;
        }
    });

    // Update map data when photos change
    $effect(() => {
        const currentMap = map;
        const geo = geotaggedPhotos;

        if (!currentMap || !currentMap.isStyleLoaded()) return;

        photoIndex.clear();
        fullGeoJSON = photosToGeoJSON(geo);
        photosWithMarkers = geo;

        const source = currentMap.getSource('photos') as maplibregl.GeoJSONSource;
        if (source) {
            const filtered = filterGeoJSONByTime(fullGeoJSON, timeFilterStart, timeFilterEnd);
            source.setData(filtered);

            // Fit bounds if we have data
            if (geo.length > 0 && filtered.features.length > 0) {
                const bounds = new maplibregl.LngLatBounds();
                for (const feature of filtered.features) {
                    const coords = (feature.geometry as GeoJSON.Point).coordinates;
                    bounds.extend(coords as [number, number]);
                }
                currentMap.fitBounds(bounds, { padding: 100, maxZoom: 15 });
            }
        }
    });

    // Update data when time filter changes
    $effect(() => {
        const currentMap = map;
        const start = timeFilterStart;
        const end = timeFilterEnd;

        if (!currentMap || !currentMap.isStyleLoaded()) return;

        const source = currentMap.getSource('photos') as maplibregl.GeoJSONSource;
        if (source && fullGeoJSON.features.length > 0) {
            const filtered = filterGeoJSONByTime(fullGeoJSON, start, end);
            source.setData(filtered);

            // Pan to center of filtered data
            if (start && end && filtered.features.length > 0) {
                const bounds = new maplibregl.LngLatBounds();
                for (const feature of filtered.features) {
                    const coords = (feature.geometry as GeoJSON.Point).coordinates;
                    bounds.extend(coords as [number, number]);
                }
                currentMap.easeTo({ center: bounds.getCenter(), duration: 300 });
            }
        }
    });

    // === Box Selection ===
    function toggleBoxSelectMode() {
        isBoxSelectMode = !isBoxSelectMode;
        if (!isBoxSelectMode) {
            cleanupBoxSelection();
        }
    }

    function cleanupBoxSelection() {
        if (selectionBox && selectionBox.parentNode) {
            selectionBox.parentNode.removeChild(selectionBox);
        }
        selectionBox = null;
        isDrawingBox = false;
        boxStart = null;
    }

    let boxSelectedTimeRange = $state<{ start: Date; end: Date } | null>(null);

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

        // Disable map drag
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

        // Find photos within bounds
        const photosInBounds: Date[] = [];
        for (const feature of fullGeoJSON.features) {
            const coords = (feature.geometry as GeoJSON.Point).coordinates;
            const lngLat = new maplibregl.LngLat(coords[0], coords[1]);
            if (bounds.contains(lngLat)) {
                const ts = feature.properties?.timestamp;
                if (ts) {
                    photosInBounds.push(new Date(ts));
                }
            }
        }

        if (photosInBounds.length > 0) {
            photosInBounds.sort((a, b) => a.getTime() - b.getTime());
            boxSelectedTimeRange = {
                start: photosInBounds[0],
                end: photosInBounds[photosInBounds.length - 1]
            };

            // Fit map to selected bounds
            map.fitBounds(bounds, { padding: 100, animate: true });
        }

        // Cleanup
        cleanupBoxSelection();
        map.dragPan.enable();
        isBoxSelectMode = false;
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
            class="absolute inset-0"
            class:box-select-mode={isBoxSelectMode}
        ></div>

        <!-- Time range indicator -->
        {#if timeRangeDisplay && hasGeotaggedPhotos}
            <div class="absolute top-4 left-1/2 -translate-x-1/2 z-[1000] pointer-events-none">
                <div class="px-5 py-2 theme-bg-card backdrop-blur-sm rounded-full theme-text-primary text-base font-medium shadow-lg flex items-center gap-2">
                    <i class="fa-regular fa-calendar text-[var(--accent)]"></i>
                    <span class="tabular-nums min-w-[200px] text-center">{timeRangeDisplay}</span>
                </div>
            </div>
        {/if}

        <!-- No geotagged photos message -->
        {#if photos.length > 0 && !hasGeotaggedPhotos}
            <div class="absolute inset-0 flex items-center justify-center bg-black/30 backdrop-blur-sm z-[1000] pointer-events-none">
                <div class="text-center p-6 theme-bg-secondary rounded-2xl border theme-border shadow-xl">
                    <i class="fa-solid fa-location-slash text-4xl theme-text-muted mb-4"></i>
                    <h3 class="text-xl font-bold theme-text-primary mb-2">No Geotagged Photos</h3>
                    <p class="theme-text-secondary max-w-xs">None of your imported photos have GPS data.</p>
                </div>
            </div>
        {/if}

        <!-- Box select button -->
        {#if hasGeotaggedPhotos}
            <button
                onclick={(e) => { e.stopPropagation(); toggleBoxSelectMode(); }}
                onmousedown={(e) => e.stopPropagation()}
                class="absolute left-4 bottom-4 z-[1001] w-10 h-10 flex items-center justify-center rounded-xl shadow-lg transition-colors
                    {isBoxSelectMode
                        ? 'bg-[var(--accent)] text-black'
                        : 'theme-bg-card backdrop-blur-sm theme-text-primary hover:bg-[var(--accent)] hover:text-black'}"
                title="Box select photos"
            >
                <i class="fa-solid fa-vector-square"></i>
            </button>
        {/if}
    </div>

    <!-- Timeline slider -->
    {#if hasGeotaggedPhotos}
        <div class="flex-shrink-0 min-h-[120px]">
            {#if map && isReady && photosWithMarkers.length > 0}
                <TimelineSlider
                    photos={photosWithMarkers}
                    externalTimeRange={boxSelectedTimeRange}
                    onTimeRangeChange={handleTimeRangeChange}
                    onExternalRangeConsumed={() => boxSelectedTimeRange = null}
                />
            {/if}
        </div>
    {/if}
</div>

<style>
    /* MapLibre overrides */
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

    /* Photo popup styles */
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
