<script lang="ts">
    import { platform } from "@tauri-apps/plugin-os";
    import { onMount } from "svelte";
    import Map from "./Map.svelte";

    let { photos, onOpenPreview, theme = "dark" } = $props<{
        photos: any[];
        onOpenPreview?: (photo: any, visiblePhotos: any[]) => void;
        theme?: "dark" | "light";
    }>();

    let tileConfig = $state<{ tiles: string[]; tileSize: number } | null>(null);

    // Platform-specific tile configurations
    const tileConfigs: Record<string, { tiles: string[]; tileSize: number }> = {
        // iOS/Android: OSM tiles (works in mobile WebView)
        ios: {
            tiles: ['https://tile.openstreetmap.org/{z}/{x}/{y}.png'],
            tileSize: 256
        },
        android: {
            tiles: ['https://tile.openstreetmap.org/{z}/{x}/{y}.png'],
            tileSize: 256
        },
        // Desktop: CARTO tiles (better looking)
        macos: {
            tiles: [
                'https://a.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png',
                'https://b.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png',
                'https://c.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png',
            ],
            tileSize: 256
        },
        windows: {
            tiles: [
                'https://a.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png',
                'https://b.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png',
            ],
            tileSize: 256
        },
        linux: {
            tiles: [
                'https://a.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png',
                'https://b.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}@2x.png',
            ],
            tileSize: 256
        },
    };

    let isMobile = $state(false);

    onMount(async () => {
        const currentPlatform = await platform();
        tileConfig = tileConfigs[currentPlatform] ?? tileConfigs.macos;
        isMobile = currentPlatform === "ios" || currentPlatform === "android";
    });
</script>

{#if tileConfig}
    <Map {photos} {onOpenPreview} {theme} {tileConfig} {isMobile} />
{:else}
    <div class="w-full h-full flex items-center justify-center theme-bg-secondary">
        <i class="fa-solid fa-spinner fa-spin text-2xl theme-text-muted"></i>
    </div>
{/if}
