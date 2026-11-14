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
    let { items, total, initialIndex = null, onOpen, onLocate, nativeMenu = false, onClose, t, locale } = $props<{
        /** Time-ordered, oldest first. */
        items: AlbumItem[];
        /** How many matched in all (may be more than `items`). */
        total: number;
        /** Open scrolled to this item instead of the newest (coming back to a list). */
        initialIndex?: number | null;
        onOpen: (index: number) => void;
        /** Show this item's place on the map (offered on long press). */
        onLocate?: (index: number) => void;
        /**
         * The platform shows its own context menu on long press. Cells are then
         * links (`footos-album://item/<index>`) the native side recognises, and
         * this component draws no menu of its own.
         */
        nativeMenu?: boolean;
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
            // Once the layout is known: the item asked for, else the newest photos.
            if (first) {
                requestAnimationFrame(() => {
                    scroller.scrollTop = scroller.scrollHeight;
                    if (initialIndex !== null) reveal(initialIndex);
                });
            }
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

    // Long press on a cell opens our own menu. (The system image menu of the
    // WebView, "Save to Photos" / "Copy", is switched off in the styles: these
    // are thumbnails, so saving or copying one is never what is wanted.)
    const LONG_PRESS_MS = 420;
    const MOVE_TOLERANCE = 10;
    let menuIndex = $state<number | null>(null);
    let pressTimer: ReturnType<typeof setTimeout> | null = null;
    let pressX = 0;
    let pressY = 0;
    let suppressClick = false;

    function cancelPress() {
        if (pressTimer) {
            clearTimeout(pressTimer);
            pressTimer = null;
        }
    }

    function pressStart(e: TouchEvent, index: number) {
        cancelPress();
        if (nativeMenu || e.touches.length !== 1) return;
        pressX = e.touches[0].clientX;
        pressY = e.touches[0].clientY;
        suppressClick = false;
        pressTimer = setTimeout(() => {
            pressTimer = null;
            suppressClick = true;
            menuIndex = index;
        }, LONG_PRESS_MS);
    }

    function pressMove(e: TouchEvent) {
        if (!pressTimer) return;
        const touch = e.touches[0];
        if (Math.abs(touch.clientX - pressX) > MOVE_TOLERANCE || Math.abs(touch.clientY - pressY) > MOVE_TOLERANCE) {
            cancelPress();
        }
    }

    function cellClick(index: number, e?: MouseEvent) {
        // Cells may be links; a tap must never navigate.
        e?.preventDefault();
        // The click that ends a long press belongs to the menu, not the cell.
        if (suppressClick) {
            suppressClick = false;
            return;
        }
        onOpen(index);
    }

    function menuAction(run: (index: number) => void) {
        const index = menuIndex;
        menuIndex = null;
        if (index !== null) run(index);
    }

    function dateTime(raw: string): string {
        const d = day(raw);
        const m = /[ T](\d{2}):(\d{2})/.exec(raw || "");
        return m ? `${d} ${m[1]}:${m[2]}` : d;
    }

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

    <div class="scroller" bind:this={scroller} onscroll={() => { scrollTop = scroller.scrollTop; cancelPress(); }}>
        <div class="sizer" style="height: {totalHeight}px">
            {#each visible as { item, index } (index)}
                <svelte:element
                    this={nativeMenu ? "a" : "button"}
                    class="cell"
                    class:native={nativeMenu}
                    href={nativeMenu ? `footos-album://item/${index}` : undefined}
                    draggable="false"
                    role="button"
                    tabindex="0"
                    data-album-index={index}
                    style="width: {cell}px; height: {cell}px; transform: translate3d({(index % columns) * (cell + GAP)}px, {Math.floor(index / columns) * rowHeight}px, 0)"
                    onclick={(e: MouseEvent) => cellClick(index, e)}
                    ontouchstart={(e: TouchEvent) => pressStart(e, index)}
                    ontouchmove={pressMove}
                    ontouchend={cancelPress}
                    ontouchcancel={cancelPress}
                    oncontextmenu={(e: Event) => { if (!nativeMenu) e.preventDefault(); }}
                >
                    {#if item.thumb}
                        <img src={convertFileSrc(item.thumb)} alt="" loading="lazy" decoding="async" draggable="false" />
                    {/if}
                    {#if item.media_type === "video"}
                        <span class="badge">{duration(item.duration_ms)}</span>
                    {:else if item.media_type === "live_photo"}
                        <span class="badge live"><i class="fa-regular fa-circle-dot"></i></span>
                    {/if}
                </svelte:element>
            {/each}
        </div>
    </div>

    {#if menuIndex !== null}
        {@const item = items[menuIndex]}
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div class="sheet-backdrop" data-album-menu onclick={() => { menuIndex = null; }}>
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            <div class="sheet" onclick={(e) => e.stopPropagation()}>
                <div class="sheet-head">
                    {#if item.thumb}
                        <img src={convertFileSrc(item.thumb)} alt="" draggable="false" />
                    {/if}
                    <div class="sheet-meta">
                        <div>{dateTime(item.date_taken)}</div>
                        <div class="coords">{item.lat.toFixed(5)}, {item.lon.toFixed(5)}</div>
                    </div>
                </div>
                {#if onLocate}
                    <button data-album-locate onclick={() => menuAction((i) => onLocate(i))}>
                        <i class="fa-solid fa-location-dot"></i>{t.map.locateOnMap}
                    </button>
                {/if}
                <button onclick={() => menuAction(onOpen)}>
                    <i class="fa-solid fa-expand"></i>{t.map.viewPhoto}
                </button>
                <button class="cancel" onclick={() => { menuIndex = null; }}>{t.common.cancel}</button>
            </div>
        </div>
    {/if}
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
    /* No system image menu, no text selection, no drag ghost on long press. */
    .album,
    .album :global(img) {
        -webkit-touch-callout: none;
        -webkit-user-select: none;
        user-select: none;
    }
    /* Native menu: the system needs the long press on these links, but not
       text selection or a drag lift. */
    .album :global(a.cell.native) {
        -webkit-touch-callout: default;
        -webkit-user-drag: none;
        display: block;
    }
    .cell img {
        pointer-events: none;
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
    .sheet-backdrop {
        position: absolute;
        inset: 0;
        z-index: 10;
        background: rgba(0, 0, 0, 0.45);
        display: flex;
        align-items: flex-end;
    }
    .sheet {
        width: 100%;
        padding: 12px 12px calc(env(safe-area-inset-bottom, 0px) + 12px);
        background: var(--bg-secondary, #1e293b);
        border-radius: 16px 16px 0 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }
    .sheet-head {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 4px 4px 12px;
    }
    .sheet-head img {
        width: 52px;
        height: 52px;
        border-radius: 8px;
        object-fit: cover;
    }
    .sheet-meta {
        font-size: 14px;
        font-weight: 600;
    }
    .sheet-meta .coords {
        font-size: 12px;
        font-weight: 400;
        opacity: 0.6;
        font-variant-numeric: tabular-nums;
    }
    .sheet button {
        display: flex;
        align-items: center;
        gap: 14px;
        min-height: 50px;
        padding: 0 12px;
        font-size: 16px;
        text-align: left;
        color: inherit;
        border-radius: 10px;
    }
    .sheet button:active {
        background: rgba(127, 127, 127, 0.2);
    }
    .sheet button i {
        width: 20px;
        text-align: center;
        color: var(--accent, #fbbf24);
    }
    .sheet button.cancel {
        justify-content: center;
        margin-top: 6px;
        opacity: 0.75;
    }
    .badge.live {
        left: 4px;
        right: auto;
        top: 3px;
        bottom: auto;
    }
</style>
