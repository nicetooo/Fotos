<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import maplibregl from "maplibre-gl";
    import "maplibre-gl/dist/maplibre-gl.css";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import TimelineSlider from "./TimelineSlider.svelte";

    let { photos, onOpenPreview, theme = "dark", tileConfig } = $props<{
        photos: any[];
        onOpenPreview?: (photo: any, visiblePhotos: any[]) => void;
        theme?: "dark" | "light";
        tileConfig?: { tiles: string[]; tileSize: number };
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
        const formatDate = (d: Date) => d.toLocaleDateString('en-US', {
            month: 'short',
            day: 'numeric',
            year: short ? '2-digit' : 'numeric'
        });
        const formatTime = (d: Date) => d.toLocaleTimeString('en-US', {
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

    // Track previous photo count to detect add vs remove
    let prevPhotoCount = 0;

    // Create all markers for photos
    function createPhotoMarkers(photos: any[]) {
        if (!map) return;

        const isInitialLoad = photoMarkers.size === 0;
        const isAddingPhotos = photos.length > prevPhotoCount;

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

            // Create marker with image element for proper error handling
            const markerThumb = document.createElement('div');
            markerThumb.className = 'marker-thumb';

            const img = document.createElement('img');
            img.src = url;
            img.alt = '';
            img.loading = 'lazy';

            // Handle image load errors - try original path if thumbnail fails
            img.onerror = () => {
                // If thumbnail failed, try original image path
                if (photo.thumb_path && img.src !== getThumbnailUrl(photo.path)) {
                    img.src = getThumbnailUrl(photo.path);
                } else {
                    // Show placeholder if all fails
                    img.style.display = 'none';
                    markerThumb.classList.add('error');
                }
            };

            markerThumb.appendChild(img);

            if (photo.hasRaw) {
                const badge = document.createElement('span');
                badge.className = 'raw-badge';
                badge.textContent = 'R';
                markerThumb.appendChild(badge);
            }

            el.appendChild(markerThumb);

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
                const originalUrl = getThumbnailUrl(photo.path);
                const popupId = `popup-img-${photo.id || Date.now()}`;

                const html = `
                    <div class="photo-popup">
                        <img id="${popupId}" src="${url}" alt="${filename}" onerror="this.onerror=null; this.src='${originalUrl}';" />
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

        // Fit bounds only on initial load or when adding new photos
        // Don't reset view when deleting photos
        if (photos.length > 0 && (isInitialLoad || isAddingPhotos)) {
            // Delay fitBounds on initial load to ensure map is properly sized
            if (isInitialLoad) {
                setTimeout(() => {
                    map?.fitBounds(bounds, { padding: 100, maxZoom: 15 });
                }, 150);
            } else {
                map.fitBounds(bounds, { padding: 100, maxZoom: 15 });
            }
        }

        prevPhotoCount = photos.length;
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

    // Get time range of photos visible in current map bounds
    function getVisiblePhotosTimeRange(): { start: Date; end: Date } | null {
        if (!map || photoMarkers.size === 0) return null;

        const bounds = map.getBounds();
        const visibleDates: Date[] = [];

        for (const { marker, date } of photoMarkers.values()) {
            if (!date) continue;
            const lngLat = marker.getLngLat();
            if (bounds.contains(lngLat)) {
                visibleDates.push(date);
            }
        }

        if (visibleDates.length === 0) return null;

        visibleDates.sort((a, b) => a.getTime() - b.getTime());
        return {
            start: visibleDates[0],
            end: visibleDates[visibleDates.length - 1]
        };
    }

    // Handle map move - sync to timeline
    function handleMapMove() {
        if (syncSource === 'timeline') return; // Prevent circular sync

        const timeRange = getVisiblePhotosTimeRange();
        if (timeRange) {
            syncSource = 'map';
            mapVisibleTimeRange = timeRange;
            // Reset sync source after a short delay
            setTimeout(() => {
                syncSource = null;
            }, 100);
        }
    }

    // Time range from map view (for timeline sync)
    let mapVisibleTimeRange = $state<{ start: Date; end: Date } | null>(null);

    // Fit map to show all photos
    function handleShowAllPhotos() {
        if (!map || photoMarkers.size === 0) return;

        const bounds = new maplibregl.LngLatBounds();
        for (const { marker } of photoMarkers.values()) {
            bounds.extend(marker.getLngLat());
        }
        map.fitBounds(bounds, { padding: 100, maxZoom: 15, duration: 300 });
    }

    // Map style based on theme and tile config
    function getMapStyle(isDark: boolean): maplibregl.StyleSpecification {
        // Use provided tileConfig or default to CARTO
        if (tileConfig) {
            return {
                version: 8,
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

        // Default: CARTO tiles with theme support
        const tileType = isDark ? 'dark_all' : 'light_all';
        const sourceId = isDark ? 'carto-dark' : 'carto-light';
        return {
            version: 8,
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

        // Initialize MapLibre map
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

        // Add controls at bottom-right for mobile
        map.addControl(new maplibregl.NavigationControl(), 'bottom-right');

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
                this._button.title = 'Show all photos';
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

        // Attribution at bottom-left to not overlap with controls
        map.addControl(new maplibregl.AttributionControl({ compact: true }), 'bottom-left');

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

        // Map move event - sync to timeline
        map.on('moveend', handleMapMove);
    }

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

    // Update map style when theme changes
    let prevTheme = theme;
    $effect(() => {
        if (!map || !mapLoaded) return;
        if (theme === prevTheme) return;
        prevTheme = theme;

        map.setStyle(getMapStyle(theme === 'dark'));
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
                    <h3 class="text-xl font-bold theme-text-primary mb-2">No Geotagged Photos</h3>
                    <p class="theme-text-secondary max-w-xs">None of your imported photos have GPS data.</p>
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

    /* Photo marker styles - optimized for smooth map movement */
    :global(.maplibregl-marker) {
        will-change: transform;
        contain: layout style;
    }
    :global(.photo-marker) {
        cursor: pointer;
        will-change: transform;
        backface-visibility: hidden;
        -webkit-backface-visibility: hidden;
        transform: translateZ(0);
    }
    :global(.photo-marker:hover) {
        transform: translateZ(0) scale(1.15);
        z-index: 1000 !important;
    }
    :global(.marker-thumb) {
        width: 48px;
        height: 48px;
        position: relative;
        contain: strict;
    }
    :global(.marker-thumb img) {
        width: 44px;
        height: 44px;
        border-radius: 50%;
        border: 2px solid white;
        object-fit: cover;
        background: #1e293b;
        will-change: auto;
    }
    :global(.marker-thumb.error) {
        background: #374151;
        border-radius: 50%;
        border: 2px solid white;
        display: flex;
        align-items: center;
        justify-content: center;
    }
    :global(.marker-thumb.error::after) {
        content: '\f03e';
        font-family: 'Font Awesome 6 Free';
        font-weight: 400;
        font-size: 20px;
        color: #6b7280;
    }
    :global(:root.light .marker-thumb img) {
        border-color: #1e293b;
        background: #f1f5f9;
    }
    :global(:root.light .marker-thumb.error) {
        background: #e5e7eb;
        border-color: #1e293b;
    }
    :global(:root.light .marker-thumb.error::after) {
        color: #9ca3af;
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

    /* Mobile: move bottom-right controls up above attribution */
    .mobile-map :global(.maplibregl-ctrl-bottom-right) {
        bottom: 30px !important;
        right: 10px !important;
    }
</style>
