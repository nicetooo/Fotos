<script lang="ts">
    import { onMount } from "svelte";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import type { AlbumItem } from "../../shared/query";
    import type { Translations, Locale } from "../../shared/i18n/types";

    /**
     * The album as a grid, oldest at the top and opened at the bottom (newest),
     * like the system Photos app. Tapping a cell opens the viewer on that item.
     * Only the rows near the viewport are in the DOM.
     */
    let { items, total, onOpen, onClose, t, locale } = $props<{
        /** Time-ordered, oldest first. */
        items: AlbumItem[];
        /** How many matched in all (may be more than `items`). */
        total: number;
        onOpen: (index: number) => void;
        onClose: () => void;
        t: Translations;
        locale: Locale;
    }>();

    const GAP = 2;
    const OVERSCAN = 4;

    let scroller: HTMLDivElement;
    let width = $state(0);
    let height = $state(0);
    let scrollTop = $state(0);

    let columns = $derived(width >= 700 ? 7 : width >= 500 ? 5 : 4);
    let cell = $derived(width > 0 ? (width - (columns - 1) * GAP) / columns : 0);
    let rowHeight = $derived(cell + GAP);
    let rows = $derived(Math.ceil(items.length / columns));
    let totalHeight = $derived(rows > 0 ? rows * rowHeight - GAP : 0);

    let firstRow = $derived(rowHeight > 0 ? Math.max(0, Math.floor(scrollTop / rowHeight) - OVERSCAN) : 0);
    let lastRow = $derived(
        rowHeight > 0 ? Math.min(rows - 1, Math.ceil((scrollTop + height) / rowHeight) + OVERSCAN) : -1,
    );

    let visible = $derived.by(() => {
        const out: { item: AlbumItem; index: number }[] = [];
        const end = Math.min(items.length, (lastRow + 1) * columns);
        for (let index = firstRow * columns; index < end; index++) {
            out.push({ item: items[index], index });
        }
        return out;
    });

    /** Scroll so that `index` is on screen (used when the viewer closes). */
    export function reveal(index: number) {
        if (!scroller || rowHeight <= 0) return;
        const top = Math.floor(index / columns) * rowHeight;
        if (top < scroller.scrollTop || top + cell > scroller.scrollTop + height) {
            scroller.scrollTop = Math.max(0, top - (height - cell) / 2);
        }
    }

    onMount(() => {
        const ro = new ResizeObserver((entries) => {
            const r = entries[0]?.contentRect;
            if (!r) return;
            const first = width === 0;
            width = r.width;
            height = r.height;
            // Open on the newest photos, once the layout is known.
            if (first) requestAnimationFrame(() => { scroller.scrollTop = scroller.scrollHeight; });
        });
        ro.observe(scroller);
        return () => ro.disconnect();
    });

    function day(raw: string): string {
        const m = /^(\d{4})[-:](\d{2})[-:](\d{2})/.exec(raw || "");
        if (!m) return "";
        return new Date(Date.UTC(+m[1], +m[2] - 1, +m[3])).toLocaleDateString(locale, {
            year: "numeric", month: "short", day: "numeric", timeZone: "UTC",
        });
    }

    // First and last dated item give the span shown in the header.
    let span = $derived.by(() => {
        let from = "";
        for (const it of items) { if ((from = day(it.date_taken))) break; }
        const to = items.length > 0 ? day(items[items.length - 1].date_taken) : "";
        if (!from && !to) return "";
        return from === to || !from || !to ? from || to : `${from} – ${to}`;
    });

    function duration(ms: number): string {
        const s = Math.floor(ms / 1000);
        const m = Math.floor(s / 60);
        return `${m}:${String(s % 60).padStart(2, "0")}`;
    }
</script>

<div class="album" data-album-grid>
    <header>
        <button class="back" onclick={onClose} aria-label={t.common.close}>
            <i class="fa-solid fa-chevron-left"></i>
        </button>
        <div class="title">
            <div class="span">{span}</div>
            <div class="count">
                {items.length}{#if total > items.length}&nbsp;/&nbsp;{total}{/if} {t.nav.photos}
            </div>
        </div>
    </header>

    <div class="scroller" bind:this={scroller} onscroll={() => { scrollTop = scroller.scrollTop; }}>
        <div class="sizer" style="height: {totalHeight}px">
            {#each visible as { item, index } (index)}
                <button
                    class="cell"
                    data-album-index={index}
                    style="width: {cell}px; height: {cell}px; transform: translate3d({(index % columns) * (cell + GAP)}px, {Math.floor(index / columns) * rowHeight}px, 0)"
                    onclick={() => onOpen(index)}
                >
                    {#if item.thumb}
                        <img src={convertFileSrc(item.thumb)} alt="" loading="lazy" decoding="async" draggable="false" />
                    {/if}
                    {#if item.media_type === "video"}
                        <span class="badge">{duration(item.duration_ms)}</span>
                    {:else if item.media_type === "live_photo"}
                        <span class="badge live"><i class="fa-regular fa-circle-dot"></i></span>
                    {/if}
                </button>
            {/each}
        </div>
    </div>
</div>

<style>
    .album {
        position: fixed;
        inset: 0;
        z-index: 3000;
        display: flex;
        flex-direction: column;
        background: var(--bg-primary, #0f172a);
        color: var(--text-primary, #f1f5f9);
    }
    header {
        flex-shrink: 0;
        display: flex;
        align-items: center;
        gap: 4px;
        padding: calc(env(safe-area-inset-top, 0px) + 6px) 12px 8px 4px;
    }
    .back {
        width: 44px;
        height: 44px;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 20px;
        color: inherit;
    }
    .title {
        min-width: 0;
        flex: 1;
        text-align: center;
        padding-right: 44px;
    }
    .span {
        font-size: 16px;
        font-weight: 600;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .count {
        font-size: 12px;
        opacity: 0.6;
        font-variant-numeric: tabular-nums;
    }
    .scroller {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
        -webkit-overflow-scrolling: touch;
        overscroll-behavior: contain;
        padding-bottom: env(safe-area-inset-bottom, 0px);
    }
    .sizer {
        position: relative;
        width: 100%;
    }
    .cell {
        position: absolute;
        top: 0;
        left: 0;
        padding: 0;
        overflow: hidden;
        background: rgba(127, 127, 127, 0.18);
    }
    .cell img {
        width: 100%;
        height: 100%;
        object-fit: cover;
        display: block;
    }
    .badge {
        position: absolute;
        right: 4px;
        bottom: 3px;
        font-size: 11px;
        font-weight: 600;
        color: #fff;
        text-shadow: 0 0 3px rgba(0, 0, 0, 0.8);
        font-variant-numeric: tabular-nums;
    }
    .badge.live {
        left: 4px;
        right: auto;
        top: 3px;
        bottom: auto;
    }
</style>
