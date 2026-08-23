<script lang="ts">
    import Map from "./Map.svelte";
    import { getPlatformService } from "../../lib/platform";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";

    let { photos, onOpenPreview, theme = "dark", t, locale, isSyncing = false } = $props<{
        photos: any[];
        onOpenPreview?: (photo: any, visiblePhotos: any[]) => void;
        theme?: "dark" | "light";
        t: Translations;
        locale: Locale;
        isSyncing?: boolean;
    }>();

    const tileConfig = getPlatformService().getMapTileConfig();
</script>

<Map {photos} {onOpenPreview} {theme} {tileConfig} {t} {locale} {isSyncing} />
