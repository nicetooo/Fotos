<script lang="ts">
    import { onMount } from "svelte";
    import Map from "./Map.svelte";
    import { getPlatformService, type MapTileConfig } from "../../lib/platform";
    import type { Translations, Locale } from "../../shared/i18n/types";

    let { refreshKey = 0, libraryCount = 0, onOpenPreview, theme = "dark", t, locale = 'en' as Locale } = $props<{
        refreshKey?: number;
        libraryCount?: number;
        onOpenPreview?: (item: { id: number; path: string }) => void;
        theme?: "dark" | "light";
        t: Translations;
        locale?: Locale;
    }>();

    // null means "use the default base map", so readiness is tracked separately.
    let tileConfig = $state<MapTileConfig | null>(null);
    let ready = $state(false);

    onMount(() => {
        // Get tile config from platform service
        const platformService = getPlatformService();
        tileConfig = platformService.getMapTileConfig();
        ready = true;
    });
</script>

{#if ready}
    <Map {refreshKey} {libraryCount} {onOpenPreview} {theme} tileConfig={tileConfig ?? undefined} {t} {locale} />
{:else}
    <div class="w-full h-full flex items-center justify-center theme-bg-secondary">
        <i class="fa-solid fa-spinner fa-spin text-2xl theme-text-muted"></i>
    </div>
{/if}
