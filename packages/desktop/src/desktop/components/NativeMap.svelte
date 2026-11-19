<script lang="ts">
    import { onMount, onDestroy, untrack } from "svelte";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import TimelineSlider from "./TimelineSlider.svelte";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";
    import {
        queryMap,
        geoSummary,
        queryTimeline,
        photoDetail,
        paddedBbox,
        abbreviateCount,
        type Bbox,
        type MapFeature,
    } from "../../shared/query";

    /**
     * The map drawn by the system map view (MapKit on macOS), which sits
     * underneath the transparent web view. This component does everything
     * Map.svelte does except drawing: it asks Rust for what is inside the
     * region the native map reports, hands the result back to be drawn, keeps
     * the timeline in step, and provides the controls, the hover preview and
     * box selection. Same props as Map.svelte, plus `command`.
     */
    let {
        refreshKey = 0,
        libraryCount = 0,
        onOpenPreview,
        onOpenLibrary,
        theme = "dark",
        command,
        t,
        locale = "en" as Locale,
    } = $props<{
        refreshKey?: number;
        libraryCount?: number;
        onOpenPreview?: (item: { id: number; path: string }) => void;
        onOpenLibrary?: () => void;
        theme?: "dark" | "light";
        /** Sends one map command to the native side. */
        command: (name: string, payload?: Record<string, unknown>) => void;
        t: Translations;
        locale?: Locale;
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

    // Clusters are computed per whole zoom level. The system map keeps the
    // camera height while it is dragged, so its zoom level drifts with
    // latitude; without some slack a drag near a whole number flips the level
    // back and forth and every cluster is rebuilt each time.
    const LEVEL_SLACK = 0.3;
    let stickyLevel: number | null = null;
    function clusterLevel(zoom: number): number {
        if (stickyLevel === null || zoom < stickyLevel - LEVEL_SLACK || zoom >= stickyLevel + 1 + LEVEL_SLACK) {
            stickyLevel = Math.floor(zoom);
        }
        return stickyLevel;
    }

    async function runQuery() {
        const r = region;
        if (!r) return;
        const seq = ++querySeq;
        try {
            const t0 = performance.now();
            const res = await queryMap(queryBox(r, 0.5), clusterLevel(r.zoom) + 0.5, timeFilterStart, timeFilterEnd);
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
        if (r.moving) {
            hideHover();
        } else {
            scheduleTimelineSync();
            moveEndListeners.forEach((fn) => fn());
        }
    }

    // Room kept around a fitted box, so markers do not end up under the date
    // pill or the buttons along the edges.
    const FIT_PADDING = { padding: 80, paddingTop: 90 };

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
            const minZoom = (stickyLevel ?? Math.floor(region?.zoom ?? 0)) + 1 + LEVEL_SLACK + 0.05;
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

    // The system map cannot zoom out far enough to show the whole world in a
    // narrow window. Past this width "show everything" goes to where most of
    // the photos are instead of to the middle of the extent, which may be empty.
    const MAX_FIT_SPAN_DEG = 300;

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


    function zoomBy(delta: number) {
        if (!region) return;
        command("mapEaseTo", { lon: region.lon, lat: region.lat, zoom: region.zoom + delta, duration: 250 });
    }

    // ============================================================
    // Hover preview
    // ============================================================

    let hover = $state<{ key: string; x: number; y: number; path: string; thumb: string; date: string; hasRaw: boolean } | null>(null);

    function onHover(e: { key: string; x: number; y: number }) {
        const f = featureByKey.get(e.key);
        if (!f || boxMode) return;
        hover = { key: e.key, x: e.x, y: e.y, path: f.path, thumb: f.thumb, date: "", hasRaw: false };
        // Thumbnail and name come with the feature; date and RAW badge follow.
        photoDetail(f.id).then((detail) => {
            if (!detail || hover?.key !== e.key) return;
            hover = {
                ...hover,
                date: detail.metadata?.date_taken?.replace(/"/g, "") || "",
                hasRaw: !!(detail as any).hasRaw || !!detail.rawId,
            };
        }).catch(() => {});
    }

    function onHoverEnd(e: { key: string }) {
        if (hover?.key === e.key) hover = null;
    }

    function hideHover() {
        hover = null;
    }

    // ============================================================
    // Box selection: drag a rectangle, go there, and show its time span
    // ============================================================

    let boxMode = $state(false);
    let boxStart: { x: number; y: number } | null = null;
    let box = $state<{ left: number; top: number; width: number; height: number } | null>(null);
    let boxSelectedTimeRange = $state<{ startMs: number; endMs: number } | null>(null);

    /** The coordinate under a point of the map area, from the last reported region. */
    function unproject(x: number, y: number): { lon: number; lat: number } | null {
        if (!region || !mapArea) return null;
        const rect = mapArea.getBoundingClientRect();
        const world = 512 * Math.pow(2, region.zoom);
        const mercatorY = (lat: number) => {
            const s = Math.sin((lat * Math.PI) / 180);
            return (0.5 - Math.log((1 + s) / (1 - s)) / (4 * Math.PI)) * world;
        };
        const px = ((region.lon + 180) / 360) * world + (x - rect.width / 2);
        const py = mercatorY(region.lat) + (y - rect.height / 2);
        const lon = (px / world) * 360 - 180;
        const lat = (Math.atan(Math.sinh(Math.PI * (1 - (2 * py) / world))) * 180) / Math.PI;
        return { lon, lat };
    }

    function boxDown(e: MouseEvent) {
        const rect = mapArea.getBoundingClientRect();
        boxStart = { x: e.clientX - rect.left, y: e.clientY - rect.top };
        box = { left: boxStart.x, top: boxStart.y, width: 0, height: 0 };
        e.preventDefault();
    }

    function boxMove(e: MouseEvent) {
        if (!boxStart) return;
        const rect = mapArea.getBoundingClientRect();
        const x = e.clientX - rect.left;
        const y = e.clientY - rect.top;
        box = {
            left: Math.min(boxStart.x, x),
            top: Math.min(boxStart.y, y),
            width: Math.abs(x - boxStart.x),
            height: Math.abs(y - boxStart.y),
        };
    }

    function boxUp() {
        const drawn = box;
        boxStart = null;
        box = null;
        if (!boxMode) return;
        boxMode = false;
        if (!drawn || drawn.width < 4 || drawn.height < 4) return;
        const sw = unproject(drawn.left, drawn.top + drawn.height);
        const ne = unproject(drawn.left + drawn.width, drawn.top);
        if (!sw || !ne) return;
        const bounds = { west: sw.lon, south: sw.lat, east: ne.lon, north: ne.lat };
        // Time span of what is inside the box, straight from the index.
        queryTimeline(bounds).then((stamps) => {
            if (stamps.length === 0) return;
            boxSelectedTimeRange = { startMs: stamps[0], endMs: stamps[stamps.length - 1] };
            command("mapFitBounds", { w: bounds.west, s: bounds.south, e: bounds.east, n: bounds.north, padding: 100, maxZoom: 18, animated: true });
        }).catch((e) => console.error("[Map] box select failed", e));
    }

    // ============================================================
    // Layout: where the map is, and where the web UI covers it
    // ============================================================

    let lastFrame = "";
    let lastOverlays = "";

    function reportFrame() {
        if (!mapArea) return;
        const r = mapArea.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0) return;
        // `vh`: the native side needs the viewport height to know where the
        // page starts inside the web view (it reaches under the title bar).
        const f = { x: r.left, y: r.top, w: r.width, h: r.height, vh: window.innerHeight };
        const key = `${f.x},${f.y},${f.w},${f.h},${f.vh}`;
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
     * Mouse events on the map area go to the native map, except where
     * something of the page lies over it. Rather than have every control of
     * the app mark itself, this collects what can be over the map: the
     * controls inside the map area, and everything that is a sibling of the
     * map on the way up to the document (toolbars, drawers, menus, dialogs).
     */
    function reportOverlays() {
        if (!mapArea) return;
        const area = mapArea.getBoundingClientRect();
        const rects: number[][] = [];
        // An element's box does not bound what is inside it: a menu that
        // drops out of a 48 px button column is far wider than the column. So
        // go down the tree, stopping at anything that clips its content
        // (nothing sticks out of that) and at a budget, in case of a huge one.
        let budget = 1500;
        const add = (el: Element) => {
            if (budget-- <= 0) return;
            const style = getComputedStyle(el);
            if (style.display === "none" || style.visibility === "hidden") return;
            const r = el.getBoundingClientRect();
            const inArea = r.width > 0 && r.height > 0
                && r.right > area.left && r.left < area.right && r.bottom > area.top && r.top < area.bottom;
            if (inArea && style.pointerEvents !== "none") {
                rects.push([Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)]);
            }
            const clips = style.overflowX !== "visible" || style.overflowY !== "visible";
            // Covering the whole map already: nothing inside can add to that.
            const coversAll = inArea && style.pointerEvents !== "none"
                && r.left <= area.left && r.top <= area.top && r.right >= area.right && r.bottom >= area.bottom;
            if (clips || coversAll) return;
            for (const child of el.children) add(child);
        };
        mapArea.querySelectorAll("[data-map-overlay]").forEach(add);
        for (let node: Element = mapArea; node.parentElement; node = node.parentElement) {
            for (const sibling of node.parentElement.children) {
                if (sibling !== node && !(sibling instanceof HTMLScriptElement) && !(sibling instanceof HTMLStyleElement)) {
                    add(sibling);
                }
            }
        }
        const key = JSON.stringify(rects);
        if (key === lastOverlays) return;
        lastOverlays = key;
        command("mapSetOverlays", { enabled: true, rects });
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
        (window as any).__footosNativeMap = { onRegion, onTap, onHover, onHoverEnd };

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
        // Panels and menus come and go as DOM changes; a slow timer catches
        // what moves without one (transitions).
        mutationObserver = new MutationObserver(scheduleLayoutReport);
        mutationObserver.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["class", "style"] });
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

    $effect(() => {
        const dark = theme === "dark";
        if (!attached) return;
        untrack(() => command("mapSetTheme", { dark }));
    });

    // Library changed (import progress, delete, clear): query again.
    $effect(() => {
        refreshKey;
        if (!attached) return;
        untrack(() => refresh());
    });
</script>

<svelte:window onmousemove={boxMove} onmouseup={boxUp} />

<div class="w-full h-full flex flex-col overflow-hidden">
    <!-- Map area: left unpainted, the native map shows through -->
    <div class="flex-1 min-h-0 relative" bind:this={mapArea}>
        <!-- Time range indicator -->
        {#if timeRangeDisplay && hasGeotaggedPhotos}
            <div class="absolute top-4 left-1/2 -translate-x-1/2 z-[1000] pointer-events-none">
                <div class="px-5 py-2 text-base theme-bg-card rounded-full theme-text-primary font-medium flex items-center gap-1.5">
                    <i class="fa-regular fa-calendar text-[var(--accent)] flex-shrink-0"></i>
                    <span class="tabular-nums whitespace-nowrap">{timeRangeDisplay}</span>
                </div>
            </div>
        {/if}

        <!-- No geotagged photos message -->
        {#if summaryLoaded && libraryCount > 0 && !hasGeotaggedPhotos}
            <div class="absolute inset-0 flex items-center justify-center bg-black/30 z-[1000] pointer-events-none">
                <div class="text-center p-6 theme-bg-secondary rounded-2xl border theme-border">
                    <i class="fa-solid fa-location-slash text-4xl theme-text-muted mb-4"></i>
                    <h3 class="text-xl font-bold theme-text-primary mb-2">{t.map.noGeotaggedPhotos}</h3>
                    <p class="theme-text-secondary max-w-xs">{t.map.noGeotaggedPhotosDesc}</p>
                </div>
            </div>
        {/if}

        <!-- While selecting, the page takes the mouse over the whole map -->
        {#if boxMode}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="absolute inset-0 z-[999] cursor-crosshair" data-map-overlay onmousedown={boxDown}>
                {#if box}
                    <div class="selection" style="left: {box.left}px; top: {box.top}px; width: {box.width}px; height: {box.height}px"></div>
                {/if}
            </div>
        {/if}

        <!-- Preview of the marker under the pointer -->
        {#if hover}
            <div class="hover" style="left: {hover.x - (mapArea?.getBoundingClientRect().left ?? 0)}px; top: {hover.y - (mapArea?.getBoundingClientRect().top ?? 0) - 8}px">
                <img src={convertFileSrc(hover.thumb || hover.path)} alt="" />
                <div class="hover-info">
                    <div class="hover-name">{hover.path.split("/").pop()}</div>
                    {#if hover.date}<div class="hover-date">{hover.date}</div>{/if}
                </div>
                {#if hover.hasRaw}<div class="hover-raw">RAW</div>{/if}
            </div>
        {/if}

        <!-- Controls, laid out as on the phone: show all (and box selection,
             which only the desktop has), zoom, and the library on its own at
             the bottom. -->
        <div class="controls">
            <div class="group" data-map-overlay>
                <button onclick={() => fitAll(false)} title={t.map.showAllPhotos} aria-label={t.map.showAllPhotos}>
                    <i class="fa-solid fa-expand"></i>
                </button>
                <button class:active={boxMode} onclick={() => { boxMode = !boxMode; hideHover(); }} title={t.map.boxSelect} aria-label={t.map.boxSelect}>
                    <i class="fa-solid fa-vector-square"></i>
                </button>
            </div>
            <div class="group" data-map-overlay>
                <button onclick={() => zoomBy(1)} aria-label="+"><i class="fa-solid fa-plus"></i></button>
                <button onclick={() => zoomBy(-1)} aria-label="-"><i class="fa-solid fa-minus"></i></button>
            </div>
            <div class="group" data-map-overlay>
                <button onclick={() => onOpenLibrary?.()} data-open-library title={t.nav.library} aria-label={t.nav.library}>
                    <i class="fa-solid fa-images"></i>
                </button>
            </div>
        </div>
    </div>

    <!-- Timeline slider -->
    {#if hasGeotaggedPhotos}
        <div class="flex-shrink-0 min-h-[120px] theme-bg-secondary">
            {#if attached && timelineStamps.length > 0}
                <TimelineSlider
                    stamps={timelineStamps}
                    {mapConstrained}
                    externalTimeRange={boxSelectedTimeRange}
                    onTimeRangeChange={handleTimeRangeChange}
                    onExternalRangeConsumed={() => { boxSelectedTimeRange = null; }}
                    onShowAll={() => fitAll(false)}
                    {t}
                    {locale}
                />
            {/if}
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
        /* Flat: no shadows on controls anywhere in the app. */
        background: rgba(15, 23, 42, 0.9);
        border: 1px solid rgba(255, 255, 255, 0.1);
    }
    :global(:root.light) .group {
        background: rgba(255, 255, 255, 0.95);
        border-color: rgba(0, 0, 0, 0.1);
    }
    .group button {
        width: 40px;
        height: 40px;
        display: flex;
        align-items: center;
        justify-content: center;
        color: #e2e8f0;
        font-size: 15px;
    }
    .group button + button {
        border-top: 1px solid rgba(255, 255, 255, 0.1);
    }
    :global(:root.light) .group button {
        color: #475569;
    }
    :global(:root.light) .group button + button {
        border-top-color: rgba(0, 0, 0, 0.1);
    }
    .group button:hover {
        color: var(--accent);
    }
    .group button.active {
        background: var(--accent);
        color: black;
    }

    .selection {
        position: absolute;
        border: 2px dashed var(--accent);
        background-color: rgba(99, 102, 241, 0.15);
        pointer-events: none;
    }

    .hover {
        position: absolute;
        z-index: 1001;
        width: 240px;
        transform: translate(-50%, -100%);
        pointer-events: none;
        border-radius: 10px;
        overflow: hidden;
        background: rgba(15, 23, 42, 0.95);
        border: 1px solid rgba(255, 255, 255, 0.1);
        color: white;
    }
    :global(:root.light) .hover {
        background: rgba(255, 255, 255, 0.98);
        border-color: rgba(0, 0, 0, 0.1);
        color: #1e293b;
    }
    .hover img {
        display: block;
        width: 100%;
        height: 170px;
        object-fit: cover;
    }
    .hover-info {
        padding: 8px 10px;
    }
    .hover-name {
        font-size: 12px;
        font-weight: 600;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .hover-date {
        font-size: 11px;
        opacity: 0.65;
        margin-top: 3px;
    }
    .hover-raw {
        position: absolute;
        top: 8px;
        right: 8px;
        background: #d97706;
        color: white;
        font-size: 10px;
        font-weight: bold;
        padding: 2px 6px;
        border-radius: 4px;
    }
</style>
