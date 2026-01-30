<script lang="ts">
    import { convertFileSrc } from "@tauri-apps/api/core";
    import type { PhotoInfo } from "../../shared/types";

    interface Props {
        photos: PhotoInfo[];
        onPhotoClick: (photo: PhotoInfo) => void;
    }

    let { photos, onPhotoClick }: Props = $props();

    /** Format video duration (ms -> "m:ss" or "h:mm:ss") */
    function formatDuration(ms?: number): string {
        if (!ms) return "0:00";
        const totalSec = Math.floor(ms / 1000);
        const min = Math.floor(totalSec / 60);
        const sec = totalSec % 60;
        if (min >= 60) {
            const hr = Math.floor(min / 60);
            const remainMin = min % 60;
            return `${hr}:${remainMin.toString().padStart(2, '0')}:${sec.toString().padStart(2, '0')}`;
        }
        return `${min}:${sec.toString().padStart(2, '0')}`;
    }

    // ── Layout constants ──
    const GAP = 6;       // gap-1.5 = 6px
    const PADDING = 8;   // p-2 = 8px
    const OVERSCAN = 3;  // extra rows above/below viewport

    // ── DOM refs ──
    let scrollContainer: HTMLDivElement;
    let containerWidth = $state(0);

    // ── Scroll state ──
    let scrollTop = $state(0);

    // ── Responsive column count (mirrors Tailwind breakpoints) ──
    // grid-cols-4 sm:grid-cols-5 md:grid-cols-6
    let columns = $derived.by(() => {
        if (containerWidth >= 768) return 6;  // md
        if (containerWidth >= 640) return 5;  // sm
        return 4;
    });

    // ── Cell size: square cells that fit the grid ──
    let cellSize = $derived.by(() => {
        const available = containerWidth - 2 * PADDING - (columns - 1) * GAP;
        return Math.floor(available / columns);
    });

    // ── Row height including gap ──
    let rowHeight = $derived(cellSize + GAP);

    // ── Total row count ──
    let totalRows = $derived(Math.ceil(photos.length / columns));

    // ── Total scrollable height ──
    let totalHeight = $derived(
        totalRows > 0 ? totalRows * rowHeight - GAP + 2 * PADDING : 0
    );

    // ── Visible row range ──
    let viewportHeight = $state(0);

    let startRow = $derived.by(() => {
        const row = Math.floor((scrollTop - PADDING) / rowHeight);
        return Math.max(0, row - OVERSCAN);
    });

    let endRow = $derived.by(() => {
        const visibleRows = Math.ceil(viewportHeight / rowHeight);
        return Math.min(totalRows - 1, startRow + visibleRows + 2 * OVERSCAN);
    });

    // ── Visible photos slice ──
    let visiblePhotos = $derived.by(() => {
        const start = startRow * columns;
        const end = (endRow + 1) * columns;
        return photos.slice(start, end).map((photo, i) => ({
            photo,
            index: start + i,
        }));
    });

    // ── Offset for the visible chunk ──
    let offsetY = $derived(PADDING + startRow * rowHeight);

    // ── Event handlers ──
    function handleScroll() {
        scrollTop = scrollContainer.scrollTop;
    }

    // ── ResizeObserver to track container width/height ──
    $effect(() => {
        if (!scrollContainer) return;
        const ro = new ResizeObserver((entries) => {
            const entry = entries[0];
            if (entry) {
                containerWidth = entry.contentRect.width;
                viewportHeight = entry.contentRect.height;
            }
        });
        ro.observe(scrollContainer);
        return () => ro.disconnect();
    });
</script>

<div
    bind:this={scrollContainer}
    class="flex-1 overflow-y-auto"
    onscroll={handleScroll}
>
    <!-- Spacer to set total scrollable height -->
    <div style="height: {totalHeight}px; position: relative;">
        <!-- Visible rows only, positioned absolutely -->
        <div
            style="
                position: absolute;
                top: {offsetY}px;
                left: {PADDING}px;
                right: {PADDING}px;
                display: grid;
                grid-template-columns: repeat({columns}, {cellSize}px);
                gap: {GAP}px;
            "
        >
            {#each visiblePhotos as { photo } (photo.path)}
                <button
                    class="relative overflow-hidden rounded theme-bg-secondary hover:ring-2 hover:ring-[var(--accent)] transition-all group"
                    style="width: {cellSize}px; height: {cellSize}px;"
                    onclick={() => onPhotoClick(photo)}
                    title={photo.path.split("/").pop()}
                >
                    {#if photo.thumb_path}
                        <img
                            src={convertFileSrc(photo.thumb_path)}
                            alt=""
                            class="w-full h-full object-cover"
                            loading="lazy"
                        />
                    {:else if photo.media_type === 'video' || photo.media_type === 'live_photo'}
                        <!-- Video without thumbnail: show placeholder instead of loading raw video -->
                        <div class="w-full h-full flex items-center justify-center theme-bg-tertiary">
                            <i class="fa-solid fa-film text-2xl theme-text-muted"></i>
                        </div>
                    {:else}
                        <img
                            src={convertFileSrc(photo.path)}
                            alt=""
                            class="w-full h-full object-cover"
                            loading="lazy"
                        />
                    {/if}
                    {#if photo.hasRaw}
                        <div class="absolute top-1 right-1 bg-amber-600 text-white text-[8px] font-bold px-1 rounded">R</div>
                    {:else if photo.isRawOnly}
                        <div class="absolute top-1 right-1 bg-rose-600 text-white text-[8px] font-bold px-1 rounded">RAW</div>
                    {/if}
                    {#if photo.media_type === 'video'}
                        <div class="absolute bottom-1 left-1 px-1.5 py-0.5 rounded text-[10px] font-semibold bg-black/70 text-white flex items-center gap-1">
                            <i class="fa-solid fa-play text-[8px]"></i>
                            {formatDuration(photo.metadata?.duration_ms)}
                        </div>
                    {:else if photo.media_type === 'live_photo'}
                        <div class="absolute bottom-1 left-1 px-1.5 py-0.5 rounded text-[10px] font-semibold bg-black/70 text-white">
                            LIVE
                        </div>
                    {/if}
                    {#if photo.metadata?.lat && photo.metadata?.lon}
                        <div class="absolute bottom-1 {photo.media_type === 'video' || photo.media_type === 'live_photo' ? 'right-1' : 'left-1'} text-white/70 text-[10px]">
                            <i class="fa-solid fa-location-dot"></i>
                        </div>
                    {/if}
                </button>
            {/each}
        </div>
    </div>
</div>
