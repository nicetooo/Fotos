<script lang="ts">
    import { onMount, onDestroy, untrack } from "svelte";
    import TimelineSlider from "./TimelineSlider.svelte";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";
    import {
        queryMap,
        geoSummary,
        queryTimeline,
        paddedBbox,
        abbreviateCount,
        type Bbox,
        type MapFeature,
    } from "../../shared/query";

    /**
     * The map drawn by the platform's own map view, which sits underneath the
     * (transparent) web view. This component keeps everything the web map
     * component does except drawing: it asks Rust for what is inside the region
     * the native map reports, hands the result back to be drawn, and keeps the
     * timeline in step. Same props as Map.svelte.
     */
    let {
        refreshKey = 0,
        libraryCount = 0,
        onOpenAlbum,
        focusRequest = null,
        onOpenPreview,
        theme = "dark",
        command,
        t,
        locale,
        isSyncing = false,
    } = $props<{
        refreshKey?: number;
        focusRequest?: { lon: number; lat: number; seq: number } | null;
        onOpenAlbum?: (bbox: Bbox, startMs: number | null, endMs: number | null) => void;
        libraryCount?: number;
        onOpenPreview?: (item: { id: number; path: string }) => void;
        theme?: "dark" | "light";
        /** Sends one map command to the native side. */
        command: (name: string, payload?: Record<string, unknown>) => void;
        t: Translations;
        locale: Locale;
        isSyncing?: boolean;
    }>();

    interface Region {
        west: number;
        south: number;
        east: number;
        north: number;
        zoom: number;
        lon: number;
        lat: number;
        moving: boolean;
    }

    let mapArea: HTMLDivElement;
    let attached = $state(false);
    let region: Region | null = null;

    let total = $state(0);
    let summaryLoaded = $state(false);
    let hasGeotaggedPhotos = $derived(total > 0);

    // Time filter (wall-clock ms). Set only by the user through the timeline.
    let timeFilterStart = $state<number | null>(null);
    let timeFilterEnd = $state<number | null>(null);

    let timelineStamps = $state.raw<Float64Array>(new Float64Array(0));
    let mapConstrained = $state(false);

    function handleTimeRangeChange(start: number | null, end: number | null) {
        if (start === timeFilterStart && end === timeFilterEnd) return;
        timeFilterStart = start;
        timeFilterEnd = end;
        refresh();
        if (start === null) scheduleTimelineSync();
    }

    function formatTimeRange(start: number | null, end: number | null): string {
        if (start === null || end === null) return "";
        const a = new Date(start);
        const b = new Date(end);
        const sameDay = a.toISOString().slice(0, 10) === b.toISOString().slice(0, 10);
        const formatDate = (d: Date) => d.toLocaleDateString(locale, {
            month: "short", day: "numeric", year: "2-digit", timeZone: "UTC",
        });
        const formatTime = (d: Date) => d.toLocaleTimeString(locale, {
            hour: "numeric", minute: "2-digit", timeZone: "UTC",
        });
        if (sameDay) return `${formatDate(a)} ${formatTime(a)} - ${formatTime(b)}`;
        return `${formatDate(a)} - ${formatDate(b)}`;
    }

    // The pill at the top: the range the user picked, or else the span of what is in view.
    let timeRangeDisplay = $derived.by(() => {
        if (timeFilterStart !== null && timeFilterEnd !== null) {
            return formatTimeRange(timeFilterStart, timeFilterEnd);
        }
        if (timelineStamps.length === 0) return "";
        return formatTimeRange(timelineStamps[0], timelineStamps[timelineStamps.length - 1]);
    });

    // ============================================================
    // Viewport query
    // ============================================================

    let currentFeatures: MapFeature[] = [];
    let featureByKey = new Map<string, MapFeature>();
    let lastFingerprint = "";
    let lastVersion = "";
    let querySeq = 0;

    // Counters for the agent self-test (shared/agent-test.ts).
    const stats = { queries: 0, setData: 0, lastQueryMs: 0 };

    // One query at a time; requests that arrive meanwhile collapse into one follow-up.
    let queryInFlight = false;
    let queryQueued = false;

    async function refresh() {
        if (queryInFlight) {
            queryQueued = true;
            return;
        }
        queryInFlight = true;
        try {
            await runQuery();
        } finally {
            queryInFlight = false;
            if (queryQueued) {
                queryQueued = false;
                refresh();
            }
        }
    }

    /** The region as a query box, padded so short pans need no refetch. */
    function queryBox(r: Region, pad: number): Bbox {
        // Across the antimeridian the native side reports west > east; the
        // Rust index understands that form, but the padding helper does not.
        if (r.west > r.east) return { west: r.west, south: r.south, east: r.east, north: r.north };
        return paddedBbox(r.west, r.south, r.east, r.north, pad);
    }

    async function runQuery() {
        const r = region;
        if (!r) return;
        const seq = ++querySeq;
        try {
            const t0 = performance.now();
            const res = await queryMap(queryBox(r, 0.5), r.zoom, timeFilterStart, timeFilterEnd);
            stats.queries++;
            stats.lastQueryMs = Math.round((performance.now() - t0) * 10) / 10;
            if (seq !== querySeq) return;
            const becameNonEmpty = total === 0 && res.total > 0;
            total = res.total;
            summaryLoaded = true;
            applyFeatures(res.features);
            if (res.version !== lastVersion) {
                lastVersion = res.version;
                // Frame everything only the first time photos appear.
                if (becameNonEmpty) fitAll(true);
                loadStamps(timelineBbox);
            }
        } catch (e) {
            console.error("[Map] query_map failed", e);
        }
    }

    function applyFeatures(features: MapFeature[]) {
        currentFeatures = features;
        const byKey = new Map<string, MapFeature>();
        let fingerprint = "";
        const out = new Array(features.length);
        for (let i = 0; i < features.length; i++) {
            const f = features[i];
            byKey.set(f.key, f);
            fingerprint += f.key + ":" + f.count + ":" + f.id + "|";
            out[i] = {
                key: f.key,
                lat: f.lat,
                lon: f.lon,
                count: f.count,
                label: abbreviateCount(f.count),
                thumb: f.thumb,
            };
        }
        featureByKey = byKey;
        if (fingerprint === lastFingerprint) return;
        lastFingerprint = fingerprint;
        stats.setData++;
        command("mapSetFeatures", { features: out });
    }

    // ============================================================
    // Native map events
    // ============================================================

    const moveEndListeners = new Set<() => void>();

    function onRegion(r: Region) {
        region = r;
        refresh();
        if (!r.moving) {
            scheduleTimelineSync();
            moveEndListeners.forEach((fn) => fn());
        }
    }

    // Room kept around a fitted box, so markers do not end up under the status
    // bar, the date pill or the buttons along the edges.
    const FIT_PADDING = { padding: 64, paddingTop: 130 };

    // Above this zoom a cluster is photos taken at the same spot: open it instead of zooming.
    const CLUSTER_OPEN_ZOOM = 17;

    function onTap(e: { key: string }) {
        const f = featureByKey.get(e.key);
        if (!f) return;
        const [w, s, east, n] = f.bounds;
        const degenerate = w === east && s === n;
        if (f.count > 1 && !degenerate && (region?.zoom ?? 0) < CLUSTER_OPEN_ZOOM) {
            // Zoom to the extent of the cluster's members (computed in Rust).
            // Clusters are computed per whole zoom level, so the tap must reach
            // at least the next one or the same cluster may come back.
            const minZoom = Math.floor(region?.zoom ?? 0) + 1.05;
            command("mapFitBounds", { w, s, e: east, n, ...FIT_PADDING, maxZoom: 18, minZoom, animated: true });
            return;
        }
        onOpenPreview?.({ id: f.id, path: f.path });
    }

    // ============================================================
    // Timeline data
    // ============================================================

    let timelineBbox: Bbox | null = null;
    let stampSeq = 0;
    let timelineSyncTimer: ReturnType<typeof setTimeout> | null = null;

    async function loadStamps(bbox: Bbox | null) {
        const seq = ++stampSeq;
        try {
            const stamps = await queryTimeline(bbox);
            if (seq !== stampSeq) return;
            // An empty viewport keeps the previous timeline instead of blanking it.
            if (stamps.length === 0 && bbox !== null && timelineStamps.length > 0) return;
            timelineBbox = bbox;
            timelineStamps = stamps;
            mapConstrained = bbox !== null && stamps.length < total;
        } catch (e) {
            console.error("[Map] query_timeline failed", e);
        }
    }

    /** After the map settles, show the timeline of what is in view (unless the user set a range). */
    function scheduleTimelineSync() {
        if (timelineSyncTimer) clearTimeout(timelineSyncTimer);
        timelineSyncTimer = setTimeout(() => {
            timelineSyncTimer = null;
            if (!region || timeFilterStart !== null) return;
            loadStamps(queryBox(region, 0));
        }, 300);
    }

    // ============================================================
    // Camera
    // ============================================================

    // The system map cannot zoom out far enough to show several continents at
    // once on a phone. Past this width "show everything" goes to where most of
    // the photos are instead of to the middle of the extent, which may be empty.
    const MAX_FIT_SPAN_DEG = 150;

    async function fitAll(delay: boolean = false) {
        try {
            const summary = await geoSummary();
            if (!summary.extent) return;
            const [w, s, e, n] = summary.extent;
            let doFit = () => command("mapFitBounds", { w, s, e, n, ...FIT_PADDING, maxZoom: 15, animated: true });
            if (e - w > MAX_FIT_SPAN_DEG) {
                const world = await queryMap({ west: -180, south: -85, east: 180, north: 85 }, 1, timeFilterStart, timeFilterEnd);
                const densest = [...world.features].sort((a, b) => b.count - a.count)[0];
                if (densest) {
                    doFit = () => command("mapEaseTo", { lon: densest.lon, lat: densest.lat, zoom: 2, duration: 500 });
                }
            }
            if (delay) setTimeout(doFit, 150);
            else doFit();
        } catch (e) {
            console.error("[Map] geo_summary failed", e);
        }
    }

    function handleOpenAlbum() {
        if (!region) return;
        onOpenAlbum?.(queryBox(region, 0), timeFilterStart, timeFilterEnd);
    }

    function zoomBy(delta: number) {
        if (!region) return;
        command("mapEaseTo", { lon: region.lon, lat: region.lat, zoom: region.zoom + delta, duration: 250 });
    }

    // ============================================================
    // Layout: where the map is, and where the web UI covers it
    // ============================================================

    let lastFrame = "";
    let lastOverlays = "";

    function frameOf(el: Element) {
        const r = el.getBoundingClientRect();
        return { x: r.left, y: r.top, w: r.width, h: r.height };
    }

    function reportFrame() {
        if (!mapArea) return;
        const f = frameOf(mapArea);
        if (f.w <= 0 || f.h <= 0) return;
        const key = `${f.x},${f.y},${f.w},${f.h}`;
        if (key === lastFrame) return;
        lastFrame = key;
        if (!attached) {
            command("mapAttach", { ...f, dark: theme === "dark" });
            attached = true;
        } else {
            command("mapSetFrame", f);
        }
    }

    /**
     * Touches on the map area go to the native map, except where a web control
     * lies over it (`data-map-overlay`), and not at all while a web screen
     * covers the map (`data-map-blocker`).
     */
    function reportOverlays() {
        const blocked = !!document.querySelector("[data-map-blocker], [data-album-grid], [data-photo-viewer]");
        const rects: number[][] = [];
        if (!blocked) {
            document.querySelectorAll("[data-map-overlay]").forEach((el) => {
                const r = el.getBoundingClientRect();
                if (r.width > 0 && r.height > 0) {
                    rects.push([Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)]);
                }
            });
        }
        const key = (blocked ? "0" : "1") + JSON.stringify(rects);
        if (key === lastOverlays) return;
        lastOverlays = key;
        command("mapSetOverlays", { enabled: !blocked, rects });
    }

    let layoutRaf = 0;
    function scheduleLayoutReport() {
        if (layoutRaf) return;
        layoutRaf = requestAnimationFrame(() => {
            layoutRaf = 0;
            reportFrame();
            reportOverlays();
        });
    }

    let resizeObserver: ResizeObserver | null = null;
    let mutationObserver: MutationObserver | null = null;
    let layoutTimer: ReturnType<typeof setInterval> | null = null;

    onMount(() => {
        document.documentElement.classList.add("native-map");
        (window as any).__footosNativeMap = { onRegion, onTap };

        // What the agent self-test drives a map through (shared/agent-test.ts).
        (window as any).__footosMap = {
            easeTo: (o: { center?: [number, number]; zoom?: number; duration?: number }) => {
                const lon = o.center?.[0] ?? region?.lon ?? 0;
                const lat = o.center?.[1] ?? region?.lat ?? 0;
                command("mapEaseTo", { lon, lat, zoom: o.zoom ?? null, duration: o.duration ?? 500 });
            },
            on: (event: string, fn: () => void) => { if (event === "moveend") moveEndListeners.add(fn); },
            off: (event: string, fn: () => void) => { if (event === "moveend") moveEndListeners.delete(fn); },
            getZoom: () => region?.zoom ?? 0,
        };
        (window as any).__footosSetTimeFilter = handleTimeRangeChange;
        (window as any).__footosMapStats = () => ({
            ...stats,
            total,
            features: currentFeatures.length,
            stamps: timelineStamps.length,
            atlasImages: 0,
        });

        reportFrame();
        reportOverlays();
        resizeObserver = new ResizeObserver(scheduleLayoutReport);
        resizeObserver.observe(mapArea);
        // Screens and controls come and go as DOM changes; a slow timer catches
        // what moves without one (transitions, safe-area changes).
        mutationObserver = new MutationObserver(scheduleLayoutReport);
        mutationObserver.observe(document.body, { childList: true, subtree: true });
        layoutTimer = setInterval(scheduleLayoutReport, 500);
        window.addEventListener("resize", scheduleLayoutReport);
    });

    onDestroy(() => {
        document.documentElement.classList.remove("native-map");
        delete (window as any).__footosNativeMap;
        delete (window as any).__footosMap;
        delete (window as any).__footosMapStats;
        delete (window as any).__footosSetTimeFilter;
        resizeObserver?.disconnect();
        mutationObserver?.disconnect();
        if (layoutTimer) clearInterval(layoutTimer);
        if (timelineSyncTimer) clearTimeout(timelineSyncTimer);
        window.removeEventListener("resize", scheduleLayoutReport);
        command("mapDetach");
    });

    // ============================================================
    // Reactive effects
    // ============================================================

    $effect(() => {
        const dark = theme === "dark";
        if (!attached) return;
        untrack(() => command("mapSetTheme", { dark }));
    });

    // Asked to show a place (an item picked in the album grid).
    $effect(() => {
        const request = focusRequest;
        if (!request || !attached) return;
        untrack(() => command("mapEaseTo", { lon: request.lon, lat: request.lat, minZoom: 16, duration: 600 }));
    });

    // Library changed (import progress, delete, clear): query again.
    $effect(() => {
        refreshKey;
        if (!attached) return;
        untrack(() => refresh());
    });
</script>

<div class="w-full h-full flex flex-col overflow-hidden">
    <!-- Map area: left unpainted, the native map shows through -->
    <div class="flex-1 min-h-0 relative" bind:this={mapArea}>
        <!-- Time range indicator -->
        {#if timeRangeDisplay && hasGeotaggedPhotos}
            <div class="absolute left-1/2 -translate-x-1/2 z-[1000] pointer-events-none max-w-[95%]" style="top: calc(max(47px, env(safe-area-inset-top, 0px)) + 12px)">
                <div class="px-3 py-1.5 text-[11px] theme-bg-card rounded-full theme-text-primary font-medium shadow-lg flex items-center gap-1.5">
                    <i class="fa-regular fa-calendar text-[var(--accent)] text-[10px] flex-shrink-0"></i>
                    <span class="tabular-nums whitespace-nowrap">{timeRangeDisplay}</span>
                </div>
            </div>
        {/if}

        <!-- No geotagged photos message -->
        {#if !isSyncing && summaryLoaded && libraryCount > 0 && !hasGeotaggedPhotos}
            <div class="absolute inset-0 flex items-center justify-center bg-black/30 z-[1000] pointer-events-none">
                <div class="text-center p-6 theme-bg-secondary rounded-2xl border theme-border shadow-xl">
                    <i class="fa-solid fa-location-slash text-4xl theme-text-muted mb-4"></i>
                    <h3 class="text-xl font-bold theme-text-primary mb-2">{t.map.noGeotaggedPhotos}</h3>
                    <p class="theme-text-secondary max-w-xs">{t.map.noGeotaggedPhotosDesc}</p>
                </div>
            </div>
        {/if}

        <!-- Controls: show all, zoom, album of what is in view -->
        <div class="controls">
            <div class="group" data-map-overlay>
                <button onclick={() => fitAll(false)} title={t.map.showAllPhotos} aria-label={t.map.showAllPhotos}>
                    <i class="fa-solid fa-expand"></i>
                </button>
            </div>
            <div class="group" data-map-overlay>
                <button onclick={() => zoomBy(1)} aria-label="+"><i class="fa-solid fa-plus"></i></button>
                <button onclick={() => zoomBy(-1)} aria-label="-"><i class="fa-solid fa-minus"></i></button>
            </div>
            <div class="group" data-map-overlay>
                <button onclick={handleOpenAlbum} data-open-album title={t.map.openAlbum} aria-label={t.map.openAlbum}>
                    <i class="fa-solid fa-images"></i>
                </button>
            </div>
        </div>
    </div>

    <!-- Timeline slider -->
    {#if hasGeotaggedPhotos && attached && timelineStamps.length > 0}
        <div class="flex-shrink-0">
            <TimelineSlider
                stamps={timelineStamps}
                {mapConstrained}
                onTimeRangeChange={handleTimeRangeChange}
                onShowAll={() => fitAll(false)}
                {t}
                {locale}
            />
        </div>
    {/if}
</div>

<style>
    /* The page must not paint over the native map underneath the web view. */
    :global(:root.native-map),
    :global(:root.native-map body),
    :global(:root.native-map main.theme-bg-primary) {
        background-color: transparent !important;
    }

    .controls {
        position: absolute;
        right: 10px;
        bottom: 30px;
        z-index: 1000;
        display: flex;
        flex-direction: column;
        gap: 10px;
    }
    .group {
        display: flex;
        flex-direction: column;
        border-radius: 8px;
        overflow: hidden;
        background: rgba(15, 23, 42, 0.9);
        box-shadow: 0 1px 4px rgba(0, 0, 0, 0.3);
    }
    :global(:root.light) .group {
        background: rgba(255, 255, 255, 0.95);
    }
    .group button {
        width: 40px;
        height: 40px;
        display: flex;
        align-items: center;
        justify-content: center;
        color: #e2e8f0;
        touch-action: manipulation;
        -webkit-tap-highlight-color: transparent;
    }
    :global(:root.light) .group button {
        color: #475569;
    }
    .group button + button {
        border-top: 1px solid rgba(127, 127, 127, 0.25);
    }
    .group button:active {
        opacity: 0.6;
    }
</style>
