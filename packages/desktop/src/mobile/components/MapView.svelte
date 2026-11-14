<script lang="ts">
    import Map from "./Map.svelte";
    import { getPlatformService } from "../../lib/platform";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";
    import type { Bbox } from "../../shared/query";

    let { refreshKey = 0, libraryCount = 0, onOpenPreview, onOpenAlbum, theme = "dark", t, locale, isSyncing = false } = $props<{
        refreshKey?: number;
        libraryCount?: number;
        onOpenPreview?: (item: { id: number; path: string }) => void;
        onOpenAlbum?: (bbox: Bbox, startMs: number | null, endMs: number | null) => void;
        theme?: "dark" | "light";
        t: Translations;
        locale: Locale;
        isSyncing?: boolean;
    }>();

    const tileConfig = getPlatformService().getMapTileConfig();
</script>

<Map {refreshKey} {libraryCount} {onOpenPreview} {onOpenAlbum} {theme} {tileConfig} {t} {locale} {isSyncing} />
