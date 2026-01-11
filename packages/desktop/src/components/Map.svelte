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

    // Hover popup
    let hoverPopup: maplibregl.Popup | null = null;

    // Resize observer
    let resizeObserver: ResizeObserver | null = null;

    // Photo data indexed by id
    let photoIndex: Map<string, any> = new Map();

    // HTML markers for photos
    let photoMarkers: Map<string, { marker: maplibregl.Marker; photo: any; date: Date | null }> = new Map();

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

    // Create all markers for photos
    function createPhotoMarkers(photos: any[]) {
        if (!map) return;

        // Clear existing markers
        for (const { marker } of photoMarkers.values()) {
            marker.remove();
        }
        photoMarkers.clear();

        const bounds = new maplibregl.LngLatBounds();

        for (const photo of photos) {
            const lat = photo.metadata?.lat;
            const lon = photo.metadata?.lon;
            if (!lat || !lon) continue;

            const thumbPath = photo.thumb_path || photo.path;
            const url = getThumbnailUrl(thumbPath);
            const filename = photo.path.split('/').pop();
            const dateTaken = photo.metadata?.date_taken?.replace(/"/g, '') || '';

            // Create marker element
            const el = document.createElement('div');
            el.className = 'photo-marker';
            el.innerHTML = `
                <div class="marker-thumb">
                    <img src="${url}" alt="" loading="lazy" />
                    ${photo.hasRaw ? '<span class="raw-badge">R</span>' : ''}
                </div>
            `;

            // Create marker
            const marker = new maplibregl.Marker({ element: el })
                .setLngLat([lon, lat])
                .addTo(map);

            // Click handler
            el.addEventListener('click', (e) => {
                e.stopPropagation();
                if (onOpenPreview) {
                    onOpenPreview(photo, visiblePhotosSorted);
                }
            });

            // Hover handlers for popup
            el.addEventListener('mouseenter', () => {
                if (!map || !hoverPopup) return;

                const rawBadge = photo.hasRaw ? '<div class="popup-raw-badge">RAW</div>' : '';
                const dateInfo = dateTaken ? `<div class="popup-date">${dateTaken}</div>` : '';

                const html = `
                    <div class="photo-popup">
                        <img src="${url}" alt="${filename}" />
                        <div class="popup-info">
                            <div class="popup-filename">${filename}</div>
                            ${dateInfo}
                        </div>
                        ${rawBadge}
                    </div>
                `;

                hoverPopup.setLngLat([lon, lat]).setHTML(html).addTo(map);
            });

            el.addEventListener('mouseleave', () => {
                if (hoverPopup) {
                    hoverPopup.remove();
                }
            });

            const date = parsePhotoDate(photo.metadata?.date_taken);
            photoMarkers.set(photo.path, { marker, photo, date });
            photoIndex.set(photo.path, photo);
            bounds.extend([lon, lat]);
        }

        // Fit bounds
        if (photos.length > 0) {
            map.fitBounds(bounds, { padding: 100, maxZoom: 15 });
        }

        photosWithMarkers = photos;
    }

    // Update marker visibility based on time filter
    function updateMarkerVisibility(start: Date | null, end: Date | null) {
        for (const { marker, date } of photoMarkers.values()) {
            let visible = true;
            if (start && end && date) {
                visible = date >= start && date <= end;
            }

            const el = marker.getElement();
            if (el) {
                el.style.display = visible ? 'block' : 'none';
            }
        }
    }

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

        // Handle resize
        resizeObserver = new ResizeObserver(() => {
            map?.resize();
        });
        resizeObserver.observe(mapContainer);

        // Force resize after mount to ensure proper rendering
        setTimeout(() => {
            map?.resize();
        }, 100);

        // Create hover popup
        hoverPopup = new maplibregl.Popup({
            closeButton: false,
            closeOnClick: false,
            maxWidth: '300px',
            offset: [0, -20]
        });

        map.on('load', () => {
            if (!map) return;

            // Mark map as loaded
            mapLoaded = true;

            // Ready for timeline
            requestAnimationFrame(() => {
                isReady = true;
            });
        });
    });

    onDestroy(() => {
        // Clear all markers
        for (const { marker } of photoMarkers.values()) {
            marker.remove();
        }
        photoMarkers.clear();

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

    // Update map data when photos change
    $effect(() => {
        const currentMap = map;
        const geo = geotaggedPhotos;
        const loaded = mapLoaded;

        if (!currentMap || !loaded) return;

        // Create HTML markers for all photos
        createPhotoMarkers(geo);
    });

    // Update marker visibility when time filter changes
    $effect(() => {
        const start = timeFilterStart;
        const end = timeFilterEnd;
        const loaded = mapLoaded;

        if (!loaded) return;

        updateMarkerVisibility(start, end);
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
        for (const { marker, date } of photoMarkers.values()) {
            const lngLat = marker.getLngLat();
            if (bounds.contains(lngLat) && date) {
                photosInBounds.push(date);
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
            class="absolute inset-0 map-container"
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

    /* Photo marker styles */
    :global(.photo-marker) {
        cursor: pointer;
        transition: transform 0.15s ease;
    }
    :global(.photo-marker:hover) {
        transform: scale(1.15);
        z-index: 1000 !important;
    }
    :global(.marker-thumb) {
        width: 48px;
        height: 48px;
        position: relative;
    }
    :global(.marker-thumb img) {
        width: 44px;
        height: 44px;
        border-radius: 50%;
        border: 2px solid white;
        object-fit: cover;
        box-shadow: 0 4px 12px rgba(0,0,0,0.4);
        background: #1e293b;
    }
    :global(.marker-thumb .raw-badge) {
        position: absolute;
        top: 2px;
        right: 2px;
        width: 14px;
        height: 14px;
        background: #d97706;
        color: white;
        font-size: 8px;
        font-weight: bold;
        border-radius: 3px;
        display: flex;
        align-items: center;
        justify-content: center;
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
