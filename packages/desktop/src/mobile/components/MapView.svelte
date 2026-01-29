<script lang="ts">
    import { onMount } from "svelte";
    import Map from "./Map.svelte";
    import { getPlatformService, type MapTileConfig } from "../../lib/platform";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";

    let { photos, onOpenPreview, theme = "dark", t, locale } = $props<{
        photos: any[];
        onOpenPreview?: (photo: any, visiblePhotos: any[]) => void;
        theme?: "dark" | "light";
        t: Translations;
        locale: Locale;
    }>();

    let tileConfig = $state<MapTileConfig | null>(null);

    onMount(() => {
        // Get tile config from platform service
        const platformService = getPlatformService();
        tileConfig = platformService.getMapTileConfig();
    });
</script>

<Map {photos} {onOpenPreview} {theme} {tileConfig} {t} {locale} />
