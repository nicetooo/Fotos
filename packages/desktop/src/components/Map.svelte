<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import * as L from "leaflet";
    import "leaflet/dist/leaflet.css";
    import { invoke, convertFileSrc } from "@tauri-apps/api/core";
    import { appDataDir, join } from "@tauri-apps/api/path";

    let { photos } = $props<{ photos: any[] }>();

    let mapContainer: HTMLDivElement;
    let map: L.Map | null = null;
    let objectUrls: string[] = [];
    let resizeObserver: ResizeObserver | null = null;
    let tileCacheDir = "";

    // Fix for default marker icons
    delete (L.Icon.Default.prototype as any)._getIconUrl;
    L.Icon.Default.mergeOptions({
        iconRetinaUrl: null,
        iconUrl: null,
        shadowUrl: null,
    });

    // Custom tile layer with local caching
    const CachedTileLayer = L.TileLayer.extend({
        createTile: function (coords: L.Coords, done: L.DoneCallback) {
            const tile = document.createElement("img");
            tile.alt = "";
            tile.setAttribute("role", "presentation");

            const { x, y, z } = coords;
            const subdomains = this.options.subdomains;
            const s = subdomains[Math.abs(x + y) % subdomains.length];
            const url = `https://${s}.basemaps.cartocdn.com/dark_all/${z}/${x}/${y}.png`;

            // Try to load from cache first, then download if needed
            (async () => {
                try {
                    if (tileCacheDir) {
                        // Check cache
                        const cachedPath = await invoke<string | null>("get_cached_tile", {
                            cacheDir: tileCacheDir,
                            z, x, y
                        });

                        if (cachedPath) {
                            tile.src = convertFileSrc(cachedPath);
                            done(undefined, tile);
                            return;
                        }

                        // Download and cache
                        const savedPath = await invoke<string>("download_tile", {
                            cacheDir: tileCacheDir,
                            z, x, y,
                            url
                        });
                        tile.src = convertFileSrc(savedPath);
                        done(undefined, tile);
                    } else {
                        // Fallback to direct URL
                        tile.src = url;
                        done(undefined, tile);
                    }
                } catch (e) {
                    // Fallback to direct URL on error
                    tile.src = url;
                    done(undefined, tile);
                }
            })();

            return tile;
        }
    });

    onMount(async () => {
        if (!mapContainer) return;

        // Setup tile cache directory
        const appData = await appDataDir();
        tileCacheDir = await join(appData, "map_tiles");

        // Initialize map with explicit size
        map = L.map(mapContainer, {
            zoomControl: false,
            attributionControl: false,
        }).setView([20, 0], 2);

        L.control.zoom({ position: "topright" }).addTo(map);
        L.control.attribution({ position: "bottomright" }).addTo(map);

        // Use cached tile layer
        new CachedTileLayer("", {
            attribution:
                '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors &copy; <a href="https://carto.com/attributions">CARTO</a>',
            subdomains: "abcd",
            maxZoom: 20,
        }).addTo(map);

        // Initial marker update
        updateMarkers();

        // Setup resize observer with debouncing - only trigger on actual size changes
        let lastWidth = 0;
        let lastHeight = 0;
        let resizeTimeout: number | null = null;

        resizeObserver = new ResizeObserver((entries) => {
            if (!map || !mapContainer) return;

            const entry = entries[0];
            const { width, height } = entry.contentRect;

            // Skip if size hasn't actually changed
            if (width === lastWidth && height === lastHeight) return;
            if (width === 0 || height === 0) return;

            lastWidth = width;
            lastHeight = height;

            // Clear previous timeout
            if (resizeTimeout) {
                clearTimeout(resizeTimeout);
            }

            // Debounced invalidation only
            resizeTimeout = window.setTimeout(() => {
                if (map) {
                    map.invalidateSize({ pan: false, animate: false });
                }
            }, 150);
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
        objectUrls.forEach((url) => URL.revokeObjectURL(url));
        objectUrls = [];
    });

    function getThumbnailUrl(path: string): string {
        return convertFileSrc(path);
    }

    $effect(() => {
        photos;
        if (map) {
            updateMarkers();
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

        // Filter photos with coords
        const geotagged = photos.filter(
            (p: any) => p.metadata.lat && p.metadata.lon,
        );

        if (geotagged.length === 0) return;

        const bounds = new L.LatLngBounds([]);

        for (const photo of geotagged) {
            const lat = photo.metadata.lat!;
            const lon = photo.metadata.lon!;

            addMarker(photo, lat, lon);
            bounds.extend([lat, lon]);
        }

        if (geotagged.length > 0) {
            map.fitBounds(bounds, { padding: [50, 50] });
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

<div class="w-full h-full bg-[#1e293b] relative">
    <div bind:this={mapContainer} class="w-full h-full z-0 outline-none"></div>
    {#if photos.filter((p: any) => p.metadata.lat).length === 0}
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
