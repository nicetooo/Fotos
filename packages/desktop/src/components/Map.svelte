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

    // Pre-compute geotagged photos (only once when photos change)
    let geotaggedPhotos = $derived(() => {
        return photos.filter((p: any) => p.metadata?.lat && p.metadata?.lon);
    });

    let hasGeotaggedPhotos = $derived(() => geotaggedPhotos().length > 0);

    // Filter photos by time range
    let filteredPhotos = $derived(() => {
        const geo = geotaggedPhotos();
        if (!timeFilterStart || !timeFilterEnd) return geo;
        return geo.filter(p => {
            const date = parsePhotoDate(p.metadata?.date_taken);
            if (!date) return true; // Show photos without dates
            return date >= timeFilterStart! && date <= timeFilterEnd!;
        });
    });

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

        // Initial marker update
        updateMarkers();

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

    // Throttle marker updates (execute at most once per 100ms)
    let lastUpdateTime = 0;
    let pendingUpdate = false;

    $effect(() => {
        filteredPhotos();
        if (map) {
            const now = Date.now();
            if (now - lastUpdateTime >= 100) {
                lastUpdateTime = now;
                updateMarkers();
            } else if (!pendingUpdate) {
                pendingUpdate = true;
                setTimeout(() => {
                    pendingUpdate = false;
                    lastUpdateTime = Date.now();
                    updateMarkers();
                }, 100 - (now - lastUpdateTime));
            }
        }
    });

    async function updateMarkers() {
        if (!map) return;

        // Clear existing markers
        map.eachLayer((layer) => {
            if (layer instanceof L.Marker) {
                map!.removeLayer(layer);
            }
        });

        // Use pre-filtered photos (already geotagged + time filtered)
        const geotagged = filteredPhotos();

        if (geotagged.length === 0) return;

        const bounds = new L.LatLngBounds([]);

        // Limit markers to prevent performance issues
        const maxMarkers = 500;
        const photosToShow = geotagged.length > maxMarkers
            ? geotagged.slice(0, maxMarkers)
            : geotagged;

        for (const photo of photosToShow) {
            const lat = photo.metadata.lat!;
            const lon = photo.metadata.lon!;

            addMarker(photo, lat, lon);
            bounds.extend([lat, lon]);
        }

        // Only fit bounds on initial load, not during timeline scrubbing
        if (!initialBoundsFit && photosToShow.length > 0) {
            map.fitBounds(bounds, { padding: [50, 50] });
            initialBoundsFit = true;
        }
    }

    function addMarker(photo: any, lat: number, lon: number) {
        if (!map) return;

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
    }
</script>

<div class="w-full h-full bg-[#1e293b] relative flex flex-col">
    <div bind:this={mapContainer} class="flex-1 z-0 outline-none"></div>

    {#if !hasGeotaggedPhotos()}
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

    <!-- Timeline slider at bottom -->
    {#if hasGeotaggedPhotos() && map}
        <div class="z-[1000]">
            <TimelineSlider photos={geotaggedPhotos()} onTimeRangeChange={handleTimeRangeChange} />
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
