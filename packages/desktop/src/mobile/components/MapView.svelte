<script lang="ts">
    import { onMount } from "svelte";
    import Map from "./Map.svelte";
    import { getPlatformService, type MapTileConfig } from "../../lib/platform";

    let { photos, onOpenPreview, theme = "dark" } = $props<{
        photos: any[];
        onOpenPreview?: (photo: any, visiblePhotos: any[]) => void;
        theme?: "dark" | "light";
    }>();

    let tileConfig = $state<MapTileConfig | null>(null);

    onMount(() => {
        // Get tile config from platform service
        const platformService = getPlatformService();
        tileConfig = platformService.getMapTileConfig();
    });
</script>

<Map {photos} {onOpenPreview} {theme} {tileConfig} />
