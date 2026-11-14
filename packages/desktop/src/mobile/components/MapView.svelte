<script lang="ts">
    import Map from "./Map.svelte";
    import NativeMap from "./NativeMap.svelte";
    import { getPlatformService, type IOSPlatformService } from "../../lib/platform";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";
    import type { Bbox } from "../../shared/query";

    let { refreshKey = 0, libraryCount = 0, onOpenPreview, onOpenAlbum, focusRequest = null, theme = "dark", t, locale, isSyncing = false } = $props<{
        refreshKey?: number;
        libraryCount?: number;
        onOpenPreview?: (item: { id: number; path: string }) => void;
        focusRequest?: { lon: number; lat: number; seq: number } | null;
        onOpenAlbum?: (bbox: Bbox, startMs: number | null, endMs: number | null) => void;
        theme?: "dark" | "light";
        t: Translations;
        locale: Locale;
        isSyncing?: boolean;
    }>();

    const platform = getPlatformService();
    const tileConfig = platform.getMapTileConfig();
    // The platform's own map where there is one; the web map everywhere else.
    const nativeCommand = platform.capabilities.nativeMap
        ? (name: string, payload?: Record<string, unknown>) =>
              (platform as IOSPlatformService).nativeMapCommand(name, payload)
        : null;
</script>

{#if nativeCommand}
    <NativeMap {refreshKey} {libraryCount} {onOpenPreview} {onOpenAlbum} {focusRequest} {theme} command={nativeCommand} {t} {locale} {isSyncing} />
{:else}
    <Map {refreshKey} {libraryCount} {onOpenPreview} {onOpenAlbum} {focusRequest} {theme} {tileConfig} {t} {locale} {isSyncing} />
{/if}
