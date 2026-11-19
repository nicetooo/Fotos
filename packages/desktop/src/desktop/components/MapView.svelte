<script lang="ts">
    import { onMount } from "svelte";
    import Map from "./Map.svelte";
    import NativeMap from "./NativeMap.svelte";
    import { getPlatformService, type MapTileConfig } from "../../lib/platform";
    import type { Translations, Locale } from "../../shared/i18n/types";

    let { refreshKey = 0, libraryCount = 0, onOpenPreview, onOpenLibrary, theme = "dark", t, locale = 'en' as Locale } = $props<{
        refreshKey?: number;
        libraryCount?: number;
        onOpenPreview?: (item: { id: number; path: string }) => void;
        /** Opens the library drawer (the system map shows the button for it). */
        onOpenLibrary?: () => void;
        theme?: "dark" | "light";
        t: Translations;
        locale?: Locale;
    }>();

    // null means "use the default base map", so readiness is tracked separately.
    let tileConfig = $state<MapTileConfig | null>(null);
    let ready = $state(false);
    // The system map where the platform has one (macOS); the web map elsewhere.
    let nativeCommand = $state<((name: string, payload?: Record<string, unknown>) => void) | null>(null);

    onMount(() => {
        // Get tile config from platform service
        const platformService = getPlatformService();
        tileConfig = platformService.getMapTileConfig();
        if (platformService.capabilities.nativeMap && platformService.nativeMapCommand) {
            nativeCommand = (name, payload) => platformService.nativeMapCommand!(name, payload);
        }
        ready = true;
    });
</script>

{#if ready && nativeCommand}
    <NativeMap {refreshKey} {libraryCount} {onOpenPreview} {onOpenLibrary} {theme} command={nativeCommand} {t} {locale} />
{:else if ready}
    <Map {refreshKey} {libraryCount} {onOpenPreview} {theme} tileConfig={tileConfig ?? undefined} {t} {locale} />
{:else}
    <div class="w-full h-full flex items-center justify-center theme-bg-secondary">
        <i class="fa-solid fa-spinner fa-spin text-2xl theme-text-muted"></i>
    </div>
{/if}
