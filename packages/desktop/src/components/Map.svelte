<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import * as L from "leaflet";
    import "leaflet/dist/leaflet.css";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import TimelineSlider from "./TimelineSlider.svelte";

    let { photos } = $props<{ photos: any[] }>();

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

        const customIcon = L.divIcon({
            className: "custom-map-marker group",
            html: `<div class="w-12 h-12 rounded-full border-2 border-white bg-slate-800 shadow-lg overflow-hidden relative group-hover:scale-110 transition-transform">
                    <img src="${url}" class="w-full h-full object-cover" onerror="this.style.display='none'" />
                   </div>`,
            iconSize: [iconSize, iconSize],
            iconAnchor: [iconSize / 2, iconSize / 2],
        });

        const marker = L.marker([lat, lon], { icon: customIcon }).addTo(map);
        marker.bindPopup(`
            <div class="text-center">
                <p class="font-bold text-xs">${photo.path.split("/").pop()}</p>
                <p class="text-[10px] text-gray-500">${photo.metadata.date_taken || "No date"}</p>
            </div>
        `);
        return marker;
    }
</script>

<div class="w-full h-full bg-[#1e293b] relative flex flex-col">
    <div bind:this={mapContainer} class="flex-1 z-0 outline-none"></div>

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
</style>
