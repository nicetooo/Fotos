<script lang="ts">
    import { convertFileSrc, invoke } from "@tauri-apps/api/core";
    import type { PhotoInfo } from "../../shared/types";

    interface Props {
        /** Number of entries in the list. */
        total: number;
        /** The entry at an index, or undefined while its page is loading. */
        getItem: (index: number) => PhotoInfo | undefined;
        /** Changes whenever `getItem` may return something new. */
        version: number;
        onPhotoClick: (photo: PhotoInfo, index: number) => void;
    }

    let { total, getItem, version, onPhotoClick }: Props = $props();

    let hovered = $state<number | null>(null);

    /**
     * Shown on hover: when the photo was taken. A photo that records no
     * capture time shows its file's date instead; the file name only when
     * there is neither.
     */
    function hoverText(photo: PhotoInfo): string {
        const taken = photo.metadata?.date_taken?.replace(/"/g, "").trim();
        if (taken) return taken;
        const fileMs = (photo as { takenMs?: number | null }).takenMs;
        if (fileMs) {
            const d = new Date(fileMs);
            const pad = (n: number) => String(n).padStart(2, "0");
            return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
        }
        return photo.path.split("/").pop() || "";
    }

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

    // ── Thumbnail size: set by pinching, remembered ──
    // The wanted cell size decides how many columns fit; the cells then
    // stretch to fill the row exactly.
    const SIZE_KEY = "footos.libraryCellSize";
    const MIN_SIZE = 56;
    const MAX_SIZE = 420;
    let wantedSize = $state((() => {
        try {
            const saved = Number(localStorage.getItem(SIZE_KEY));
            if (saved >= MIN_SIZE && saved <= MAX_SIZE) return saved;
        } catch { /* not essential */ }
        return 120;
    })());
    $effect(() => {
        try { localStorage.setItem(SIZE_KEY, String(Math.round(wantedSize))); } catch { /* not essential */ }
    });

    let columns = $derived.by(() => {
        const usable = containerWidth - 2 * PADDING + GAP;
        return Math.min(24, Math.max(2, Math.round(usable / (wantedSize + GAP))));
    });

    // Pinching changes the number of columns: keep the photo that was at the
    // top of the view at the top.
    let anchorIndex: number | null = null;
    function setWantedSize(next: number) {
        const clamped = Math.min(MAX_SIZE, Math.max(MIN_SIZE, next));
        if (clamped === wantedSize) return;
        if (anchorIndex === null && rowHeight > 0) {
            anchorIndex = Math.floor(scrollTop / rowHeight) * columns;
        }
        wantedSize = clamped;
    }
    $effect(() => {
        columns;
        rowHeight;
        if (anchorIndex === null || !scrollContainer) return;
        const top = Math.floor(anchorIndex / columns) * rowHeight;
        anchorIndex = null;
        scrollContainer.scrollTop = top;
        scrollTop = top;
    });

    // A pinch on a trackpad: WebKit reports it as gesture events with a
    // scale, other engines as a wheel event with the control key down.
    let pinchBase = 0;
    function handleGestureStart(e: Event) {
        e.preventDefault();
        pinchBase = wantedSize;
    }
    function handleGestureChange(e: Event) {
        e.preventDefault();
        const scale = (e as Event & { scale?: number }).scale ?? 1;
        setWantedSize(pinchBase * scale);
    }
    function handleWheel(e: WheelEvent) {
        if (!e.ctrlKey) return;
        e.preventDefault();
        setWantedSize(wantedSize * Math.exp(-e.deltaY * 0.01));
    }
    function pinchable(node: HTMLElement) {
        node.addEventListener("gesturestart", handleGestureStart);
        node.addEventListener("gesturechange", handleGestureChange);
        node.addEventListener("gestureend", handleGestureStart);
        node.addEventListener("wheel", handleWheel, { passive: false });
        return {
            destroy() {
                node.removeEventListener("gesturestart", handleGestureStart);
                node.removeEventListener("gesturechange", handleGestureChange);
                node.removeEventListener("gestureend", handleGestureStart);
                node.removeEventListener("wheel", handleWheel);
            },
        };
    }

    // ── Cell size: square cells that fit the grid ──
    let cellSize = $derived.by(() => {
        const available = containerWidth - 2 * PADDING - (columns - 1) * GAP;
        return Math.floor(available / columns);
    });

    // ── Row height including gap ──
    let rowHeight = $derived(cellSize + GAP);

    // ── Total row count ──
    let totalRows = $derived(Math.ceil(total / columns));

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
        version;
        const start = startRow * columns;
        const end = Math.min(total, (endRow + 1) * columns);
        const cells: { photo: PhotoInfo | undefined; index: number }[] = [];
        for (let index = start; index < end; index++) {
            cells.push({ photo: getItem(index), index });
        }
        return cells;
    });

    // Freshly imported items show the camera's small embedded thumbnail (kept
    // under ".../quick/") or none at all. Once scrolling settles, ask for the
    // real thumbnails of what is on screen; the rest follow in the background.
    $effect(() => {
        const waiting: string[] = [];
        for (const { photo } of visiblePhotos) {
            // Undefined while its page is still loading.
            if (!photo || photo.media_type === "video") continue;
            if (!photo.thumb_path || photo.thumb_path.includes("/quick/")) waiting.push(photo.path);
        }
        if (waiting.length === 0) return;
        const timer = setTimeout(() => {
            invoke("sharpen_paths", { paths: waiting }).catch(() => {});
        }, 250);
        return () => clearTimeout(timer);
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
    use:pinchable
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
            {#each visiblePhotos as { photo, index } (index)}
                {#if !photo}
                    <div class="rounded theme-bg-secondary" style="width: {cellSize}px; height: {cellSize}px;"></div>
                {:else}
                <button
                    class="relative overflow-hidden rounded theme-bg-secondary hover:ring-2 hover:ring-[var(--accent)] transition-all group"
                    style="width: {cellSize}px; height: {cellSize}px;"
                    onclick={() => onPhotoClick(photo, index)}
                    onmouseenter={() => { hovered = index; }}
                    onmouseleave={() => { if (hovered === index) hovered = null; }}
                    data-hover-text={hoverText(photo)}
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
                    {#if hovered === index}
                        <!-- When it was taken, on the cell itself: the system
                             tooltip of a title attribute does not show up in
                             this web view. -->
                        <div class="absolute inset-x-0 bottom-0 z-10 px-1.5 py-1 bg-black/75 text-white text-[11px] leading-tight text-left tabular-nums truncate" data-cell-time>
                            {hoverText(photo)}
                        </div>
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
                {/if}
            {/each}
        </div>
    </div>
</div>
