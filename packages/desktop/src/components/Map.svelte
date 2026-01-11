<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import * as L from "leaflet";
    import "leaflet/dist/leaflet.css";
    import "leaflet.markercluster";
    import "leaflet.markercluster/dist/MarkerCluster.css";
    import "leaflet.markercluster/dist/MarkerCluster.Default.css";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import TimelineSlider from "./TimelineSlider.svelte";

    // === IndexedDB Tile Cache ===
    const DB_NAME = 'map-tile-cache';
    const DB_VERSION = 1;
    const STORE_NAME = 'tiles';
    let tileDb: IDBDatabase | null = null;

    async function initTileCache(): Promise<IDBDatabase> {
        return new Promise((resolve, reject) => {
            const request = indexedDB.open(DB_NAME, DB_VERSION);
            request.onerror = () => reject(request.error);
            request.onsuccess = () => resolve(request.result);
            request.onupgradeneeded = (event) => {
                const db = (event.target as IDBOpenDBRequest).result;
                if (!db.objectStoreNames.contains(STORE_NAME)) {
                    db.createObjectStore(STORE_NAME);
                }
            };
        });
    }

    async function getCachedTile(key: string): Promise<Blob | null> {
        if (!tileDb) return null;
        return new Promise((resolve) => {
            const tx = tileDb!.transaction(STORE_NAME, 'readonly');
            const store = tx.objectStore(STORE_NAME);
            const request = store.get(key);
            request.onsuccess = () => resolve(request.result || null);
            request.onerror = () => resolve(null);
        });
    }

    async function cacheTile(key: string, blob: Blob): Promise<void> {
        if (!tileDb) return;
        return new Promise((resolve) => {
            const tx = tileDb!.transaction(STORE_NAME, 'readwrite');
            const store = tx.objectStore(STORE_NAME);
            store.put(blob, key);
            tx.oncomplete = () => resolve();
            tx.onerror = () => resolve();
        });
    }

    // Custom cached tile layer
    function createCachedTileLayer(urlTemplate: string, options: L.TileLayerOptions) {
        const CachedTileLayer = L.TileLayer.extend({
            createTile: function(coords: L.Coords, done: L.DoneCallback) {
                const tile = document.createElement('img');
                const url = this.getTileUrl(coords);
                const cacheKey = `${coords.z}/${coords.x}/${coords.y}`;

                tile.alt = '';
                tile.setAttribute('role', 'presentation');

                // Try cache first
                getCachedTile(cacheKey).then(async (cached) => {
                    if (cached) {
                        tile.src = URL.createObjectURL(cached);
                        done(undefined, tile);
                    } else {
                        // Fetch and cache
                        try {
                            const response = await fetch(url);
                            const blob = await response.blob();
                            cacheTile(cacheKey, blob);
                            tile.src = URL.createObjectURL(blob);
                            done(undefined, tile);
                        } catch (err) {
                            tile.src = url;
                            done(err as Error, tile);
                        }
                    }
                });

                return tile;
            }
        });
        return new CachedTileLayer(urlTemplate, options);
    }

    let { photos, onOpenPreview } = $props<{
        photos: any[];
        onOpenPreview?: (photo: any, visiblePhotos: any[]) => void;
    }>();

    let mapContainer: HTMLDivElement;
    let map = $state<L.Map | null>(null);
    let resizeObserver: ResizeObserver | null = null;
    let isReady = $state(false); // Delayed initialization flag

    // Time filter state
    let timeFilterStart = $state<Date | null>(null);
    let timeFilterEnd = $state<Date | null>(null);

    // Box selection state
    let isBoxSelectMode = $state(false);
    let isDrawingBox = $state(false);
    let boxStartPoint: L.Point | null = null;
    let selectionBox: HTMLDivElement | null = null;

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

    // Fix for default marker icons
    delete (L.Icon.Default.prototype as any)._getIconUrl;
    L.Icon.Default.mergeOptions({
        iconRetinaUrl: null,
        iconUrl: null,
        shadowUrl: null,
    });

    onMount(async () => {
        if (!mapContainer) return;

        // Initialize tile cache
        try {
            tileDb = await initTileCache();
        } catch (e) {
            console.warn('Failed to init tile cache:', e);
        }

        // Initialize map with explicit size
        map = L.map(mapContainer, {
            zoomControl: false,
            attributionControl: false,
            zoomDelta: 1,
            zoomSnap: 0,
            wheelPxPerZoomLevel: 8,
            wheelDebounceTime: 0,
            zoomAnimation: true,
            fadeAnimation: false,
            markerZoomAnimation: true,
            inertia: true,
            inertiaDeceleration: 3000,
            preferCanvas: true,
            renderer: L.canvas(),
        }).setView([20, 0], 2);

        L.control.zoom({ position: "topright" }).addTo(map);
        L.control.attribution({ position: "bottomright" }).addTo(map);

        // Use cached tile layer
        createCachedTileLayer(
            "https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png",
            {
                attribution:
                    '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors &copy; <a href="https://carto.com/attributions">CARTO</a>',
                subdomains: "abcd",
                maxZoom: 20,
                keepBuffer: 100,
            },
        ).addTo(map);

        // Create marker cluster group with custom options
        markerClusterGroup = L.markerClusterGroup({
            maxClusterRadius: 60,
            spiderfyOnMaxZoom: true,
            showCoverageOnHover: false,
            zoomToBoundsOnClick: true,
            disableClusteringAtZoom: 18,
            chunkedLoading: true,
            animate: true,
            // Don't cluster groups smaller than 20
            iconCreateFunction: (cluster) => {
                const count = cluster.getChildCount();

                // For small clusters (< 20), show a mini indicator that auto-spiderfies on click
                if (count < 20) {
                    return L.divIcon({
                        html: `<div class="cluster-mini"><span>${count}</span></div>`,
                        className: 'marker-cluster-custom',
                        iconSize: L.point(28, 28),
                    });
                }

                let size = 'small';
                let diameter = 40;
                if (count >= 100) {
                    size = 'large';
                    diameter = 60;
                } else if (count >= 20) {
                    size = 'medium';
                    diameter = 50;
                }
                return L.divIcon({
                    html: `<div class="cluster-marker cluster-${size}"><span>${count}</span></div>`,
                    className: 'marker-cluster-custom',
                    iconSize: L.point(diameter, diameter),
                });
            },
        });
        map.addLayer(markerClusterGroup);

        // Delay timeline to next frame for smoother tab switch
        requestAnimationFrame(() => {
            isReady = true;
        });

        // Setup resize observer with debouncing
        let resizeTimeout: number | null = null;
        resizeObserver = new ResizeObserver(() => {
            if (!map || !mapContainer) return;

            // Clear previous timeout
            if (resizeTimeout) {
                clearTimeout(resizeTimeout);
            }

            // Immediate size check
            const rect = mapContainer.getBoundingClientRect();
            if (rect.width > 0 && rect.height > 0) {
                map.invalidateSize({ pan: false });
            }

            // Debounced final invalidation
            resizeTimeout = window.setTimeout(() => {
                if (map && mapContainer) {
                    const rect = mapContainer.getBoundingClientRect();
                    if (rect.width > 0 && rect.height > 0) {
                        map.invalidateSize({ pan: false });
                    }
                }
            }, 100);
        });

        resizeObserver.observe(mapContainer);
    });

    onDestroy(() => {
        if (resizeObserver) {
            resizeObserver.disconnect();
            resizeObserver = null;
        }
        if (markerClusterGroup) {
            markerClusterGroup.clearLayers();
            markerClusterGroup = null;
        }
        if (map) {
            map.remove();
            map = null;
        }
        if (tileDb) {
            tileDb.close();
            tileDb = null;
        }
    });

    function getThumbnailUrl(path: string): string {
        return convertFileSrc(path);
    }

    let initialBoundsFit = false;

    // Store all markers with their photo data for visibility toggling
    let allMarkers: Map<string, { marker: L.Marker; photo: any; date: Date | null }> = new Map();

    // Marker cluster group
    let markerClusterGroup: L.MarkerClusterGroup | null = null;

    // Photos that actually have markers (for TimelineSlider)
    let photosWithMarkers = $state<any[]>([]);

    // Get visible photos sorted by time for navigation
    let visiblePhotosSorted = $derived.by(() => {
        const start = timeFilterStart;
        const end = timeFilterEnd;

        const visible: { photo: any; date: Date }[] = [];
        for (const { photo, date } of allMarkers.values()) {
            if (!date) continue;
            if (start && end) {
                if (date >= start && date <= end) {
                    visible.push({ photo, date });
                }
            } else {
                visible.push({ photo, date });
            }
        }

        // Sort by date ascending
        visible.sort((a, b) => a.date.getTime() - b.date.getTime());
        return visible.map(v => v.photo);
    });

    function handleMarkerClick(photo: any) {
        if (onOpenPreview) {
            onOpenPreview(photo, visiblePhotosSorted);
        }
    }

    // Create/recreate all markers when map or photos change
    $effect(() => {
        const currentMap = map;  // Read map to establish dependency
        const geo = geotaggedPhotos;
        const clusterGroup = markerClusterGroup;

        // Clear old markers first
        if (clusterGroup) {
            clusterGroup.clearLayers();
        }
        allMarkers.clear();
        initialBoundsFit = false;

        // Create new markers if we have map, cluster group, and photos
        if (currentMap && clusterGroup && geo.length > 0) {
            createAllMarkers(geo);
        }
    });

    // Update visibility based on time filter (add/remove from cluster group)
    $effect(() => {
        // Read state values first to ensure proper dependency tracking
        const start = timeFilterStart;
        const end = timeFilterEnd;
        const currentMap = map;
        const clusterGroup = markerClusterGroup;

        if (!currentMap || !clusterGroup || allMarkers.size === 0) return;

        const toAdd: L.Marker[] = [];
        const toRemove: L.Marker[] = [];

        for (const { marker, date } of allMarkers.values()) {
            let visible = true;
            if (start && end && date) {
                visible = date >= start && date <= end;
            }

            const isInCluster = clusterGroup.hasLayer(marker);
            if (visible && !isInCluster) {
                toAdd.push(marker);
            } else if (!visible && isInCluster) {
                toRemove.push(marker);
            }
        }

        // Batch updates for performance
        if (toRemove.length > 0) {
            clusterGroup.removeLayers(toRemove);
        }
        if (toAdd.length > 0) {
            clusterGroup.addLayers(toAdd);
        }
    });

    // Pan map to center of visible photos when time filter changes (keep zoom level)
    $effect(() => {
        // Read state values first to ensure proper dependency tracking
        const start = timeFilterStart;
        const end = timeFilterEnd;
        const currentMap = map;

        if (!currentMap || allMarkers.size === 0 || !start || !end) return;

        // Collect visible photo positions
        const bounds = new L.LatLngBounds([]);
        for (const { photo, date } of allMarkers.values()) {
            if (!date) continue;
            if (date >= start && date <= end) {
                bounds.extend([photo.metadata.lat, photo.metadata.lon]);
            }
        }

        // Pan to center of visible photos without changing zoom
        if (bounds.isValid()) {
            currentMap.panTo(bounds.getCenter(), { animate: true, duration: 0.3 });
        }
    });

    function createAllMarkers(geotagged: any[]) {
        if (!map) return;

        const bounds = new L.LatLngBounds([]);

        for (const photo of geotagged) {
            const lat = photo.metadata.lat!;
            const lon = photo.metadata.lon!;
            const marker = createMarker(photo, lat, lon);
            if (marker) {
                const date = parsePhotoDate(photo.metadata?.date_taken);
                allMarkers.set(photo.path, { marker, photo, date });
                bounds.extend([lat, lon]);
            }
        }

        // Update photosWithMarkers for TimelineSlider
        photosWithMarkers = geotagged;

        // Fit bounds on initial load (extra bottom padding for timeline)
        if (geotagged.length > 0 && !initialBoundsFit) {
            map.fitBounds(bounds, { padding: [100, 100] });
            initialBoundsFit = true;
        }
    }

    function createMarker(photo: any, lat: number, lon: number): L.Marker | null {
        if (!map || !markerClusterGroup) return null;

        const iconSize = 48;
        const thumbPath = photo.thumb_path || photo.path;
        const url = getThumbnailUrl(thumbPath);
        const fileName = photo.path.split("/").pop();
        const dateTaken = photo.metadata.date_taken || "";

        const rawBadge = photo.hasRaw ? '<div class="marker-raw-badge">R</div>' : '';

        // Marker with hover preview
        const customIcon = L.divIcon({
            className: "custom-map-marker",
            html: `<div class="marker-wrapper">
                    <div class="marker-dot">
                        <img src="${url}" loading="lazy" decoding="async" onerror="this.style.display='none'" />
                        ${rawBadge}
                    </div>
                    <div class="marker-preview">
                        <img src="${url}" loading="lazy" decoding="async" />
                        <div class="marker-preview-info">
                            <div class="marker-preview-filename">${fileName}</div>
                            ${dateTaken ? `<div class="marker-preview-date">${dateTaken.replace(/"/g, '')}</div>` : ''}
                        </div>
                    </div>
                   </div>`,
            iconSize: [iconSize, iconSize],
            iconAnchor: [iconSize / 2, iconSize / 2],
        });

        const marker = L.marker([lat, lon], { icon: customIcon });

        // Add click handler for preview
        marker.on('click', () => {
            handleMarkerClick(photo);
        });

        // Add to cluster group instead of directly to map
        markerClusterGroup.addLayer(marker);

        return marker;
    }

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
        boxStartPoint = null;
    }

    // Box selected time range (to pass to TimelineSlider)
    let boxSelectedTimeRange = $state<{ start: Date; end: Date } | null>(null);

    // Global mouse handlers for box selection
    function handleGlobalMouseDown(e: MouseEvent) {
        if (!isBoxSelectMode || !map || !mapContainer) return;

        // Check if click is within map container
        const rect = mapContainer.getBoundingClientRect();
        if (e.clientX < rect.left || e.clientX > rect.right ||
            e.clientY < rect.top || e.clientY > rect.bottom) return;

        e.preventDefault();
        isDrawingBox = true;
        boxStartPoint = L.point(e.clientX - rect.left, e.clientY - rect.top);

        // Create selection box element
        selectionBox = document.createElement('div');
        selectionBox.className = 'selection-box';
        selectionBox.style.left = `${boxStartPoint.x}px`;
        selectionBox.style.top = `${boxStartPoint.y}px`;
        selectionBox.style.width = '0px';
        selectionBox.style.height = '0px';
        mapContainer.appendChild(selectionBox);

        map.dragging.disable();
    }

    function handleGlobalMouseMove(e: MouseEvent) {
        if (!isDrawingBox || !boxStartPoint || !selectionBox || !mapContainer) return;

        const rect = mapContainer.getBoundingClientRect();
        const currentPoint = L.point(e.clientX - rect.left, e.clientY - rect.top);

        const minX = Math.min(boxStartPoint.x, currentPoint.x);
        const minY = Math.min(boxStartPoint.y, currentPoint.y);
        const width = Math.abs(currentPoint.x - boxStartPoint.x);
        const height = Math.abs(currentPoint.y - boxStartPoint.y);

        selectionBox.style.left = `${minX}px`;
        selectionBox.style.top = `${minY}px`;
        selectionBox.style.width = `${width}px`;
        selectionBox.style.height = `${height}px`;
    }

    function handleGlobalMouseUp(e: MouseEvent) {
        if (!isDrawingBox || !boxStartPoint || !map || !mapContainer) return;

        const rect = mapContainer.getBoundingClientRect();
        const endPoint = L.point(e.clientX - rect.left, e.clientY - rect.top);

        // Calculate bounds from pixel coordinates
        const sw = map.containerPointToLatLng(L.point(
            Math.min(boxStartPoint.x, endPoint.x),
            Math.max(boxStartPoint.y, endPoint.y)
        ));
        const ne = map.containerPointToLatLng(L.point(
            Math.max(boxStartPoint.x, endPoint.x),
            Math.min(boxStartPoint.y, endPoint.y)
        ));
        const bounds = L.latLngBounds(sw, ne);

        // Find photos within bounds
        const photosInBounds: Date[] = [];
        for (const { photo, date } of allMarkers.values()) {
            if (!date) continue;
            const lat = photo.metadata?.lat;
            const lon = photo.metadata?.lon;
            if (lat && lon && bounds.contains([lat, lon])) {
                photosInBounds.push(date);
            }
        }

        if (photosInBounds.length > 0) {
            photosInBounds.sort((a, b) => a.getTime() - b.getTime());
            boxSelectedTimeRange = {
                start: photosInBounds[0],
                end: photosInBounds[photosInBounds.length - 1]
            };

            // Fit map to selected bounds (extra bottom padding for timeline)
            map.fitBounds(bounds, { padding: [100, 100], animate: true });
        }

        // Cleanup
        cleanupBoxSelection();
        map.dragging.enable();
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
    <!-- Map area (takes remaining space) -->
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

        <!-- Box select button (positioned in map area) -->
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

    <!-- Timeline slider at bottom (always reserves space) -->
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
    :global(.leaflet-control-container .leaflet-control-attribution) {
        background-color: rgba(15, 23, 42, 0.8) !important;
        color: #94a3b8 !important;
        border-radius: 4px;
        padding: 0 4px;
    }
    :global(.leaflet-control-container .leaflet-control-attribution a) {
        color: #818cf8 !important;
    }

    /* Map marker styles */
    :global(.custom-map-marker) {
        background: transparent !important;
        border: none !important;
    }
    :global(.marker-wrapper) {
        position: relative;
        width: 48px;
        height: 48px;
    }
    :global(.marker-dot) {
        position: relative;
        width: 48px;
        height: 48px;
        display: flex;
        align-items: center;
        justify-content: center;
    }
    :global(.marker-dot img) {
        width: 44px !important;
        height: 44px !important;
        min-width: 44px;
        min-height: 44px;
        max-width: 44px;
        max-height: 44px;
        aspect-ratio: 1/1;
        border-radius: 999px;
        border: 2px solid white;
        object-fit: cover;
        object-position: center;
        box-shadow: 0 4px 12px rgba(0,0,0,0.4);
        background: #1e293b;
    }
    :global(.marker-wrapper:hover .marker-dot) {
        transform: scale(1.1);
    }
    :global(.marker-preview) {
        position: absolute;
        bottom: 100%;
        left: 50%;
        transform: translateX(-50%);
        margin-bottom: 8px;
        opacity: 0;
        visibility: hidden;
        transition: opacity 0.2s ease, visibility 0.2s ease;
        pointer-events: none;
        z-index: 1000;
    }
    :global(.marker-wrapper:hover .marker-preview) {
        opacity: 1;
        visibility: visible;
    }
    :global(.marker-preview img) {
        width: 280px;
        height: 200px;
        object-fit: cover;
        border-radius: 8px;
        box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
        border: 2px solid rgba(255, 255, 255, 0.9);
        background: #1e293b;
    }
    :global(.marker-preview-info) {
        position: absolute;
        bottom: 0;
        left: 0;
        right: 0;
        padding: 6px 8px;
        background: linear-gradient(transparent, rgba(0, 0, 0, 0.8));
        border-radius: 0 0 6px 6px;
        color: white;
    }
    :global(.marker-preview-filename) {
        font-size: 11px;
        font-weight: 500;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    :global(.marker-preview-date) {
        font-size: 10px;
        opacity: 0.8;
        margin-top: 2px;
    }
    :global(.marker-raw-badge) {
        position: absolute;
        top: 10px;
        right: 8px;
        z-index: 10;
        width: 12px;
        height: 12px;
        background: #d97706;
        color: white;
        font-size: 7px;
        font-weight: bold;
        border-radius: 2px;
        display: flex;
        align-items: center;
        justify-content: center;
        box-shadow: 0 1px 2px rgba(0,0,0,0.5);
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
    .box-select-mode :global(.leaflet-container),
    .box-select-mode :global(.leaflet-grab),
    .box-select-mode :global(.leaflet-dragging) {
        cursor: crosshair !important;
    }

    /* Marker cluster styles */
    :global(.marker-cluster-custom) {
        background: transparent !important;
    }
    :global(.cluster-marker) {
        display: flex;
        align-items: center;
        justify-content: center;
        border-radius: 50%;
        background: linear-gradient(135deg, rgba(99, 102, 241, 0.9), rgba(139, 92, 246, 0.9));
        border: 3px solid rgba(255, 255, 255, 0.9);
        box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4), 0 0 20px rgba(99, 102, 241, 0.3);
        color: white;
        font-weight: 700;
        font-size: 14px;
        transition: transform 0.2s ease;
    }
    :global(.cluster-marker:hover) {
        transform: scale(1.1);
    }
    :global(.cluster-small) {
        width: 40px;
        height: 40px;
        font-size: 12px;
    }
    :global(.cluster-medium) {
        width: 50px;
        height: 50px;
        font-size: 14px;
    }
    :global(.cluster-large) {
        width: 60px;
        height: 60px;
        font-size: 16px;
    }
    :global(.cluster-mini) {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 28px;
        height: 28px;
        border-radius: 50%;
        background: rgba(30, 41, 59, 0.9);
        border: 2px solid rgba(255, 255, 255, 0.7);
        box-shadow: 0 2px 6px rgba(0, 0, 0, 0.3);
        color: white;
        font-weight: 600;
        font-size: 11px;
        transition: transform 0.2s ease;
    }
    :global(.cluster-mini:hover) {
        transform: scale(1.15);
        background: rgba(99, 102, 241, 0.9);
    }
    /* Hide default MarkerCluster styles */
    :global(.leaflet-cluster-anim .leaflet-marker-icon, .leaflet-cluster-anim .leaflet-marker-shadow) {
        transition: transform 0.3s ease-out, opacity 0.3s ease-in;
    }
</style>
