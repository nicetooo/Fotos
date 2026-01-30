<script lang="ts">
    import { onMount } from "svelte";
    import Map from "./Map.svelte";
    import { getPlatformService, type MapTileConfig } from "../../lib/platform";
    import type { Translations, Locale } from "../../shared/i18n/types";

    let { photos, onOpenPreview, theme = "dark", t, locale = 'en' as Locale } = $props<{
        photos: any[];
        onOpenPreview?: (photo: any, visiblePhotos: any[]) => void;
        theme?: "dark" | "light";
        t: Translations;
        locale?: Locale;
    }>();

    let tileConfig = $state<MapTileConfig | null>(null);

    onMount(() => {
        // Get tile config from platform service
        const platformService = getPlatformService();
        tileConfig = platformService.getMapTileConfig();
    });
</script>

{#if tileConfig}
    <Map {photos} {onOpenPreview} {theme} {tileConfig} {t} {locale} />
{:else}
    <div class="w-full h-full flex items-center justify-center theme-bg-secondary">
        <i class="fa-solid fa-spinner fa-spin text-2xl theme-text-muted"></i>
    </div>
{/if}
