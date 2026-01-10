<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import * as L from "leaflet";
    import "leaflet/dist/leaflet.css";
    import { invoke } from "@tauri-apps/api/core";

    let { photos } = $props<{ photos: any[] }>();

    let mapContainer: HTMLDivElement;
    let map: L.Map | null = null;
    let objectUrls: string[] = [];
    let resizeObserver: ResizeObserver | null = null;

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
        objectUrls.forEach((url) => URL.revokeObjectURL(url));
        objectUrls = [];
    });

    async function loadThumbnailUrl(path: string): Promise<string> {
        try {
            const bytes = await invoke<number[]>("read_file_bytes", { path });
            const blob = new Blob([new Uint8Array(bytes)]);
            const url = URL.createObjectURL(blob);
            objectUrls.push(url);
            return url;
        } catch (e) {
            console.error("Failed to load map thumb:", path, e);
            return "";
        }
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

    async function addMarker(photo: any, lat: number, lon: number) {
        if (!map) return;

        const iconSize = 48;

        const customIcon = L.divIcon({
            className: "custom-map-marker",
            html: `<div class="w-12 h-12 rounded-full border-2 border-white bg-slate-800 shadow-lg overflow-hidden flex items-center justify-center">
                    <i class="fa-solid fa-spinner fa-spin text-white"></i>
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

        // Load image
        const thumbPath = photo.thumb_path || photo.path;
        const url = await loadThumbnailUrl(thumbPath);

        if (url) {
            const newIcon = L.divIcon({
                className: "custom-map-marker group",
                html: `<div class="w-12 h-12 rounded-full border-2 border-white bg-slate-800 shadow-lg overflow-hidden relative group-hover:scale-110 transition-transform">
                        <img src="${url}" class="w-full h-full object-cover" />
                       </div>`,
                iconSize: [iconSize, iconSize],
                iconAnchor: [iconSize / 2, iconSize / 2],
            });
            marker.setIcon(newIcon);
        }
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
