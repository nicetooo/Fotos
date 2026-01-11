<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import * as L from "leaflet";
    import "leaflet/dist/leaflet.css";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import TimelineSlider from "./TimelineSlider.svelte";

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

    // Parse date from photo metadata
    function parsePhotoDate(dateStr: string | null): Date | null {
        if (!dateStr) return null;
        const cleaned = dateStr.replace(/"/g, '').trim();
        const date = new Date(cleaned.replace(' ', 'T'));
        return isNaN(date.getTime()) ? null : date;
    }

    // Cached geotagged photos (computed lazily)
    let cachedGeotagged: any[] | null = null;
    function getGeotaggedPhotos() {
        if (cachedGeotagged === null) {
            cachedGeotagged = photos.filter((p: any) => p.metadata?.lat && p.metadata?.lon);
        }
        return cachedGeotagged;
    }

    // Reset cache when photos change
    $effect(() => {
        photos;
        cachedGeotagged = null;
    });

    let hasGeotaggedPhotos = $derived(getGeotaggedPhotos().length > 0);

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

    onMount(() => {
        if (!mapContainer) return;

        // Initialize map with explicit size
        map = L.map(mapContainer, {
            zoomControl: false,
            attributionControl: false,
        }).setView([20, 0], 2);

        L.control.zoom({ position: "topright" }).addTo(map);
        L.control.attribution({ position: "bottomright" }).addTo(map);

        L.tileLayer(
            "https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png",
            {
                attribution:
                    '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors &copy; <a href="https://carto.com/attributions">CARTO</a>',
                subdomains: "abcd",
                maxZoom: 20,
            },
        ).addTo(map);

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
        if (map) {
            map.remove();
            map = null;
        }
    });

    function getThumbnailUrl(path: string): string {
        return convertFileSrc(path);
    }

    let initialBoundsFit = false;

    // Store all markers with their photo data for visibility toggling
    let allMarkers: Map<string, { marker: L.Marker; photo: any; date: Date | null }> = new Map();

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
        const geo = getGeotaggedPhotos();

        // Clear old markers first
        for (const { marker } of allMarkers.values()) {
            marker.remove();
        }
        allMarkers.clear();
        initialBoundsFit = false;

        // Create new markers if we have map and photos
        if (currentMap && geo.length > 0) {
            createAllMarkers(geo);
        }
    });

    // Update visibility based on time filter (fast - just toggle opacity)
    $effect(() => {
        if (!map || allMarkers.size === 0) return;

        const start = timeFilterStart;
        const end = timeFilterEnd;

        for (const { marker, date } of allMarkers.values()) {
            let visible = true;
            if (start && end && date) {
                visible = date >= start && date <= end;
            }

            const el = marker.getElement();
            if (el) {
                el.style.opacity = visible ? '1' : '0';
                el.style.pointerEvents = visible ? 'auto' : 'none';
            }
        }
    });

    // Pan map to center of visible photos when time filter changes (keep zoom level)
    $effect(() => {
        if (!map || allMarkers.size === 0) return;

        const start = timeFilterStart;
        const end = timeFilterEnd;
        if (!start || !end) return;

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
            map.panTo(bounds.getCenter(), { animate: true, duration: 0.3 });
        }
    });

    function createAllMarkers(geotagged: any[]) {
        if (!map) return;

        const bounds = new L.LatLngBounds([]);
        const maxMarkers = 500;
        const photosToShow = geotagged.length > maxMarkers
            ? geotagged.slice(0, maxMarkers)
            : geotagged;

        for (const photo of photosToShow) {
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
        photosWithMarkers = photosToShow;

        // Fit bounds on initial load
        if (photosToShow.length > 0 && !initialBoundsFit) {
            map.fitBounds(bounds, { padding: [50, 50] });
            initialBoundsFit = true;
        }
    }

    function createMarker(photo: any, lat: number, lon: number): L.Marker | null {
        if (!map) return null;

        const iconSize = 48;
        const thumbPath = photo.thumb_path || photo.path;
        const url = getThumbnailUrl(thumbPath);
        const fileName = photo.path.split("/").pop();
        const dateTaken = photo.metadata.date_taken || "";

        const rawBadge = photo.hasRaw ? '<div class="marker-raw-badge">R</div>' : '';

        const customIcon = L.divIcon({
            className: "custom-map-marker",
            html: `<div class="marker-wrapper">
                    <div class="marker-dot">
                        <img src="${url}" onerror="this.style.display='none'" />
                        ${rawBadge}
                    </div>
                    <div class="marker-preview">
                        <img src="${url}" onerror="this.parentElement.style.display='none'" />
                        <div class="marker-info">
                            <div class="marker-name">${fileName}</div>
                            ${dateTaken ? `<div class="marker-date">${dateTaken}</div>` : ''}
                            ${photo.hasRaw ? '<div class="marker-raw-info">RAW available</div>' : ''}
                        </div>
                    </div>
                   </div>`,
            iconSize: [iconSize, iconSize],
            iconAnchor: [iconSize / 2, iconSize / 2],
        });

        const marker = L.marker([lat, lon], { icon: customIcon }).addTo(map);

        // Add click handler for preview
        marker.on('click', () => {
            handleMarkerClick(photo);
        });

        return marker;
    }
</script>

<div class="w-full h-full bg-[#1e293b] relative flex flex-col">
    <div bind:this={mapContainer} class="flex-1 z-0 outline-none"></div>

    <!-- Time range indicator -->
    {#if timeRangeDisplay && hasGeotaggedPhotos}
        <div class="absolute top-4 left-1/2 -translate-x-1/2 z-[1000] pointer-events-none">
            <div class="px-5 py-2 bg-black/80 backdrop-blur-sm rounded-full text-white text-base font-medium shadow-lg flex items-center gap-2">
                <i class="fa-regular fa-calendar text-yellow-400"></i>
                <span class="tabular-nums min-w-[200px] text-center">{timeRangeDisplay}</span>
            </div>
        </div>
    {/if}

    {#if !hasGeotaggedPhotos}
        <div
            class="absolute inset-0 flex items-center justify-center bg-black/50 backdrop-blur-sm z-[1000] pointer-events-none"
        >
            <div
                class="text-center p-6 bg-slate-800 rounded-2xl border border-slate-700 shadow-xl"
            >
                <i
                    class="fa-solid fa-location-slash text-4xl text-slate-500 mb-4"
                ></i>
                <h3 class="text-xl font-bold text-white mb-2">
                    No Geotagged Photos
                </h3>
                <p class="text-slate-400 max-w-xs">
                    None of your imported photos have GPS data.
                </p>
            </div>
        </div>
    {/if}

    <!-- Timeline slider at bottom (delayed load) -->
    {#if hasGeotaggedPhotos && map && isReady && photosWithMarkers.length > 0}
        <div class="z-[1000]">
            <TimelineSlider photos={photosWithMarkers} onTimeRangeChange={handleTimeRangeChange} />
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
        transition: transform 0.15s ease;
    }
    :global(.marker-dot img) {
        width: 44px;
        height: 44px;
        border-radius: 50%;
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
        bottom: 56px;
        left: 50%;
        transform: translateX(-50%) scale(0.8);
        opacity: 0;
        pointer-events: none;
        transition: all 0.15s ease;
        z-index: 1000;
        width: fit-content;
        background: rgba(0,0,0,0.9);
        border-radius: 8px;
        overflow: hidden;
        box-shadow: 0 8px 24px rgba(0,0,0,0.5);
    }
    :global(.marker-wrapper:hover .marker-preview) {
        opacity: 1;
        transform: translateX(-50%) scale(1);
    }
    :global(.marker-preview img) {
        display: block;
        min-height: 280px;
        max-height: 360px;
        width: auto;
        height: auto;
    }
    :global(.marker-info) {
        padding: 8px;
        max-width: 450px;
    }
    :global(.marker-name) {
        font-size: 11px;
        font-weight: 600;
        color: white;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    :global(.marker-date) {
        font-size: 10px;
        color: #94a3b8;
        margin-top: 2px;
    }
    :global(.marker-raw-badge) {
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
        box-shadow: 0 1px 3px rgba(0,0,0,0.4);
    }
    :global(.marker-raw-info) {
        font-size: 9px;
        color: #d97706;
        margin-top: 2px;
    }
</style>
