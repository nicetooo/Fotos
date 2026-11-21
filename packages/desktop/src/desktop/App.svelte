<script lang="ts">
    import { onMount, untrack } from "svelte";
    import { invoke, convertFileSrc } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { revealItemInDir } from "@tauri-apps/plugin-opener";
    import { appDataDir, join } from "@tauri-apps/api/path";

    // Platform service
    import {
        setPlatformService,
        type PlatformService,
        type ImportProgress,
    } from "../lib/platform";

    // Shared utilities and types
    import {
        type PhotoInfo,
        type Theme,
        type ImportStatus,
        formatFileSize,
    } from "../shared";
    import {
        queryAlbum,
        photoDetail,
        photoCount,
        albumItemToPhoto,
        type AlbumWindow,
        type LibraryItem,
        queryLibrary,
        libraryTimeline,
        queryMap,
        type MediaFilter,
        type LocationFilter,
        type MapViewState,
    } from "../shared/query";
    import { LibraryPager, type LibraryCounts } from "./library-pager";

    // i18n
    import { createI18nState, type Locale } from "../shared/i18n";

    // Components
    import Settings from "./components/Settings.svelte";
    import ImagePreview from "./components/ImagePreview.svelte";
    import MapView from "./components/MapView.svelte";
    import MediaFilterButton from "./components/MediaFilterButton.svelte";
    import LibraryTimeline from "./components/LibraryTimeline.svelte";
    import VirtualPhotoGrid from "./components/VirtualPhotoGrid.svelte";

    // Platform service received as prop from main.ts
    let { platformService } = $props<{ platformService: PlatformService }>();

    // Set context synchronously during component initialization
    setPlatformService(platformService);

    // i18n state
    const i18n = createI18nState();
    let locale = $derived(i18n.locale);
    let t = $derived(i18n.t);

    let version = $state("...");
    let showSettings = $state(false);
    let showLibrary = $state(false);
    let importStatus = $state<ImportStatus>({
        success: 0,
        failure: 0,
        duplicates: 0,
        current: 0,
        total: 0,
        lastPath: "",
    });
    let isScanning = $state(false);
    let error = $state("");
    let dbPath = $state("");
    let thumbDir = $state("");

    // The frontend holds no photo list. `refreshKey` tells the map the library
    // changed; the drawer pages through the list while it is open.
    let refreshKey = $state(0);
    let libraryCount = $state(0);

    let libraryVersion = $state(0);
    let libraryCounts = $state<LibraryCounts>({ total: 0, groupedTotal: 0, filesTotal: 0, regionSpan: null });
    let libraryLoaded = $state(false);
    const pager = new LibraryPager(() => {
        libraryCounts = pager.counts;
        libraryLoaded = pager.loaded;
        libraryVersion++;
    });

    // Viewer. From the drawer it walks the drawer's order; from the map it
    // walks a window of the time-ordered album.
    type PreviewSource =
        | { kind: 'library'; index: number }
        | { kind: 'album'; window: AlbumWindow; pos: number };
    let previewPhoto = $state<LibraryItem | null>(null);
    let previewSource = $state.raw<PreviewSource | null>(null);
    let previewSeq = 0;

    const ALBUM_RADIUS = 300;
    const ALBUM_EDGE = 60;

    // The list is always in the order the photos were taken, oldest first:
    // the direction of the timeline above it. There is nothing to choose.
    // Media type filter state with localStorage persistence
    const MEDIA_FILTER_KEY = "footos-media-filter";
    let mediaFilter = $state<MediaFilter>((() => {
        if (typeof localStorage !== "undefined") {
            const saved = localStorage.getItem(MEDIA_FILTER_KEY);
            if (saved === "all" || saved === "photo" || saved === "video" || saved === "live_photo") {
                return saved;
            }
        }
        return "all";
    })());

    // Save the filter when changed
    $effect(() => {
        if (typeof localStorage !== "undefined") {
            localStorage.setItem(MEDIA_FILTER_KEY, mediaFilter);
        }
    });

    let libraryImportMenuOpen = $state(false);
    let floatingImportMenuOpen = $state(false);

    // Theme state
    const THEME_KEY = "footos-theme";
    let theme = $state<Theme>((() => {
        if (typeof localStorage !== "undefined") {
            const saved = localStorage.getItem(THEME_KEY);
            if (saved === "dark" || saved === "light" || saved === "system") {
                return saved;
            }
        }
        return "dark";
    })());

    // Apply theme to document
    function applyTheme(t: Theme) {
        const root = document.documentElement;
        if (t === "system") {
            const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
            root.classList.toggle("light", !prefersDark);
        } else {
            root.classList.toggle("light", t === "light");
        }
    }

    // Watch for theme changes
    $effect(() => {
        applyTheme(theme);
        if (typeof localStorage !== "undefined") {
            localStorage.setItem(THEME_KEY, theme);
        }
    });

    // Track system preference for reactive effectiveTheme
    let systemPrefersDark = $state(
        typeof window !== "undefined"
            ? window.matchMedia("(prefers-color-scheme: dark)").matches
            : true
    );

    // Listen for system theme changes
    $effect(() => {
        if (theme !== "system") return;
        const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
        const handler = () => {
            systemPrefersDark = mediaQuery.matches;
            applyTheme("system");
        };
        mediaQuery.addEventListener("change", handler);
        return () => mediaQuery.removeEventListener("change", handler);
    });

    // Effective theme resolved to actual dark/light
    let effectiveTheme = $derived<"dark" | "light">(
        theme === "system" ? (systemPrefersDark ? "dark" : "light") : theme
    );

    // The drawer queries only while it is open.
    $effect(() => {
        if (!showLibrary) return;
        const span = listSpan;
        const region = mapView
            ? { ...mapView, spanStartMs: span?.start ?? null, spanEndMs: span?.end ?? null }
            : null;
        pager.setParams("date", false, mediaFilter, locationFilter, region);
    });

    // Photos / videos / Live Photos on the map and the timeline (the drawer
    // has its own filter). Rust narrows the query index, so both follow.
    // The drawer only: everything, items with a position, items without.
    let locationFilter = $state<LocationFilter>("all");
    function cycleLocationFilter() {
        locationFilter = locationFilter === "all" ? "located" : locationFilter === "located" ? "unlocated" : "all";
    }

    // The drawer follows the map: it lists what is in view, plus photos
    // without a position taken between the first and the last of those. With
    // every placed photo in view it lists the whole library.
    let mapView = $state.raw<MapViewState | null>(null);
    let mapViewTimer: ReturnType<typeof setTimeout> | null = null;
    function handleMapViewChange(view: MapViewState) {
        // The time filter changes every frame while the timeline is dragged.
        if (mapViewTimer) clearTimeout(mapViewTimer);
        mapViewTimer = setTimeout(() => { mapViewTimer = null; mapView = view; }, 250);
    }

    // The timeline at the top of the list. It spans the whole library; what
    // is selected on it starts as the span of the photos the map shows and
    // can be dragged from there. A span the person set holds until the map
    // moves or the drawer is opened again.
    let libraryStamps = $state.raw<Float64Array>(new Float64Array(0));
    let listSpan = $state.raw<{ start: number; end: number } | null>(null);
    $effect(() => {
        mapView;
        showLibrary;
        listSpan = null;
    });
    $effect(() => {
        refreshKey;
        if (!showLibrary) return;
        libraryTimeline().then((stamps) => { libraryStamps = stamps; }).catch(() => {});
    });

    // The drawer takes half of the window or all of it; remembered.
    const LIBRARY_WIDE_KEY = "footos.libraryWide";
    let libraryWide = $state((() => {
        try { return localStorage.getItem(LIBRARY_WIDE_KEY) === "1"; } catch { return false; }
    })());
    $effect(() => {
        try { localStorage.setItem(LIBRARY_WIDE_KEY, libraryWide ? "1" : "0"); } catch { /* not essential */ }
    });

    let mapMediaFilter = $state<MediaFilter>("all");
    async function setMapMediaFilter(next: MediaFilter) {
        try {
            await invoke("set_media_filter", { filter: next === "all" ? null : next });
            mapMediaFilter = next;
            refreshKey++;
        } catch (e) {
            console.error("[filter] set_media_filter failed", e);
        }
    }

    /** The library changed: tell the map, and the drawer if it is open. */
    function bumpRefresh() {
        refreshKey++;
        photoCount().then((n) => { libraryCount = n; }).catch(() => {});
        if (showLibrary) pager.invalidate();
    }

    let lastProgressRefresh = 0;

    /** Check the remembered folders for new, changed and deleted files. */
    async function handleRescan() {
        if (!platformService?.rescanLibrary || isScanning) return;
        importStatus = { success: 0, failure: 0, duplicates: 0, current: 0, total: 0, lastPath: "" };
        await platformService.rescanLibrary();
        bumpRefresh();
        sharpenThumbnails();
    }

    // Imports record the camera's small embedded thumbnail and move on; the
    // real ones are made in the background and show up as they are ready.
    function sharpenThumbnails() {
        invoke("sharpen_thumbnails").catch((e) => console.error("[thumbs] sharpen_thumbnails failed", e));
    }
    let lastSharpenRefresh = 0;

    // Development builds write page errors to the app's log: a self-test that
    // stops with no report otherwise says nothing about why.
    if (import.meta.env.DEV) {
        const tell = (text: string) => invoke("mac_map_command", { command: "mapDebugLog", body: { text } }).catch(() => {});
        window.addEventListener("error", (e) => tell(`${e.message} @ ${e.filename}:${e.lineno}`));
        window.addEventListener("unhandledrejection", (e) => tell(`rejection: ${e.reason?.stack ?? e.reason}`));
    }

    onMount(async () => {
        try {
            version = await invoke("get_core_version");
            const appData = await appDataDir();
            dbPath = await join(appData, "footos.db");
            thumbDir = await join(appData, "thumbnails");

            // Initialize shared DB connection (single connection for all commands)
            await invoke("init_app", { dbPath, thumbDir });

            // Initialize platform service
            await platformService.initialize(dbPath, thumbDir);

            // Subscribe to platform events
            platformService.onScanningChange((scanning) => {
                isScanning = scanning;
            });

            platformService.onImportProgress((progress: ImportProgress) => {
                importStatus = {
                    success: progress.success,
                    failure: progress.failure,
                    duplicates: progress.duplicates,
                    current: progress.current,
                    total: progress.total,
                    lastPath: progress.lastPath,
                };
                // Every batch is committed, so there is something new to
                // show; once a second is plenty.
                const now = performance.now();
                if (now - lastProgressRefresh >= 1000) {
                    lastProgressRefresh = now;
                    bumpRefresh();
                }
            });

            platformService.onImportComplete(() => {
                bumpRefresh();
                sharpenThumbnails();
            });

            await listen("thumbs-sharpened", (event: any) => {
                const now = performance.now();
                if (event.payload?.remaining === 0 || now - lastSharpenRefresh >= 2000) {
                    lastSharpenRefresh = now;
                    bumpRefresh();
                }
            });

            // Listen for reload-photos event (e.g., after clear cache)
            await listen("reload-photos", () => bumpRefresh());

            bumpRefresh();

            // Agent self-test: only when launched with FOOTOS_AGENT_TEST (scripts/agent-test).
            const scenario = (await invoke("agent_test_enabled")) as string | null;
            // Pick up what changed on disk since the last run.
            if (!scenario) void handleRescan();
            if (scenario) {
                const { runAgentTest } = await import("../shared/agent-test");
                await invoke("mac_map_command", { command: "mapDebugFront", body: {} }).catch(() => {});
                void runAgentTest(scenario, {
                    getMap: () => (window as any).__footosMap ?? null,
                    getMapStats: () => (window as any).__footosMapStats?.() ?? {},
                    openPreview: async () => {
                        const w = await queryAlbum(null, 0);
                        if (w.items.length === 0) return false;
                        await openFromMap(w.items[0]);
                        const src = previewSource;
                        return { external: true, count: src?.kind === 'album' ? src.window.items.length : 0 };
                    },
                    closePreview,
                    bootMarks: () => ({}),
                    extraSteps: agentDesktopSteps,
                });
            }
        } catch (e) {
            error = t.errors.initFailed + ": " + e;
        }

        return () => {
            platformService.cleanup();
        };
    });

    async function handleScan(mode: "folder" | "file" = "folder") {
        if (!platformService?.requestScan) return;
        try {
            error = "";
            importStatus = { success: 0, failure: 0, duplicates: 0, current: 0, total: 0, lastPath: "" };
            await platformService.requestScan(mode);
            bumpRefresh();
        } catch (e) {
            error = String(e);
        }
    }

    async function handleCancelImport() {
        try {
            await invoke("cancel_import");
        } catch (e) {
            console.error("Failed to cancel import:", e);
        }
    }

    function handleLibrary() {
        showLibrary = !showLibrary;
    }

    /** A file's date as local time, for photos that record no capture time. */
    function formatFileTime(ms: number): string {
        const d = new Date(ms);
        const pad = (n: number) => String(n).padStart(2, "0");
        return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
    }

    async function handleShowInFinder(path: string, e: MouseEvent) {
        e.stopPropagation();
        try {
            await revealItemInDir(path);
        } catch (e) {
            alert(t.errors.openFailed + ": " + e);
        }
    }

    function openFromLibrary(photo: PhotoInfo, index: number) {
        previewSeq++;
        previewPhoto = photo;
        previewSource = { kind: 'library', index };
    }

    /** Open an item picked on the map, with the time-ordered album around it. */
    async function openFromMap(item: { id: number; path: string }) {
        const seq = ++previewSeq;
        try {
            const [detail, window] = await Promise.all([
                photoDetail(item.id),
                queryAlbum(item.id, ALBUM_RADIUS),
            ]);
            if (seq !== previewSeq) return;
            const pos = window.items.findIndex((p) => p.id === item.id);
            if (pos < 0) return;
            previewPhoto = detail ?? albumItemToPhoto(window.items[pos]);
            previewSource = { kind: 'album', window, pos };
        } catch (e) {
            console.error("Failed to open preview", e);
        }
    }

    function closePreview() {
        previewSeq++;
        previewPhoto = null;
        previewSource = null;
    }

    // Position of the photo on display, for the "3 / 120" label.
    let previewPosition = $derived.by(() => {
        const src = previewSource;
        if (!src) return { index: 0, total: 0 };
        if (src.kind === 'library') return { index: src.index, total: libraryCounts.total };
        return { index: src.window.start + src.pos, total: src.window.total };
    });

    /** The item one step away from the one on display, and where it sits. */
    async function previewNeighbour(
        shown: LibraryItem, src: PreviewSource, step: number,
    ): Promise<{ photo: LibraryItem; source: PreviewSource } | null> {
        if (src.kind === 'library') {
            const total = libraryCounts.total;
            if (total < 2) return null;
            const index = (src.index + step + total) % total;
            const item = await pager.item(index);
            return item ? { photo: item, source: { kind: 'library', index } } : null;
        }
        const { window } = src;
        let pos = src.pos;
        // A RAW next to its JPEG is two rows in the album but one photo: step over it.
        for (;;) {
            pos += step;
            const item = window.items[pos];
            if (!item) return null;
            const detail = await photoDetail(item.id).catch(() => null);
            const photo = detail ?? albumItemToPhoto(item);
            if (photo.id.id === shown.id.id) continue;
            return { photo, source: { kind: 'album', window, pos } };
        }
    }

    /** Near the edge of the album window: fetch a new one centred on this photo. */
    async function refreshAlbumWindow(window: AlbumWindow, pos: number, seq: number) {
        const end = window.start + window.items.length;
        const nearStart = pos < ALBUM_EDGE && window.start > 0;
        const nearEnd = pos >= window.items.length - ALBUM_EDGE && end < window.total;
        if (!nearStart && !nearEnd) return;
        const centerId = window.items[pos].id;
        const next = await queryAlbum(centerId, ALBUM_RADIUS).catch(() => null);
        if (!next || seq !== previewSeq) return;
        const nextPos = next.items.findIndex((p) => p.id === centerId);
        if (nextPos >= 0) previewSource = { kind: 'album', window: next, pos: nextPos };
    }

    // ── The large view's strip ──
    // Two fingers sideways in the large view. The area is a real sideways
    // scroller with one stop per photo, and the system does all the moving
    // (following the fingers, coasting, settling on a photo). The photos sit
    // in numbered places along one long strip: the one opened first at 0,
    // the next at 1, the one before at -1, and so on. A photo never changes
    // place and the code never moves the scroller during a swipe, so however
    // fast the swipes come there is nothing to get out of step: the photo on
    // display is simply the one whose place the scroller is nearest to.
    // The places next to it hold full views, loaded ahead; the ones after
    // those, thumbnails. A zoomed-in photo pans instead.
    type StripSlot = { photo: LibraryItem; source: PreviewSource };
    // Places run from -STRIP_BASE to STRIP_BASE. Wide enough never to reach
    // the end in one sitting, small enough for the scroller's width.
    const STRIP_BASE = 1500;
    // The scroller's stops, one per place, all of them there from the start
    // and never changed. (Stops that come and go with the photos laid out
    // make the system move the scroller to keep "the third stop" the third.)
    const stripStops = Array.from({ length: 2 * STRIP_BASE + 1 }, (_, i) => i);
    // `undefined`: not looked up yet; `null`: nothing there (the album ends).
    let strip = $state.raw<Record<number, StripSlot | null | undefined>>({});
    let stripAt = $state(0);
    // What the strip last put on display. Anything else on display was put
    // there from outside (a photo opened, one deleted): the strip starts
    // again around it.
    let stripShown: LibraryItem | null = null;
    let stripSource: PreviewSource | null = null;
    let stripRun = 0;
    let pagerEl: HTMLDivElement | undefined = $state();
    let pagerWidth = 0;
    let previewZoomed = $state(false);

    function placePager() {
        const el = pagerEl;
        if (!el || !el.clientWidth) return;
        pagerWidth = el.clientWidth;
        el.scrollLeft = (STRIP_BASE + stripAt) * pagerWidth;
    }
    function syncStrip() {
        const shown = previewPhoto;
        const src = previewSource;
        if (!shown || !src) {
            stripShown = null;
            stripSource = null;
            return;
        }
        if (shown === stripShown && src === stripSource) return;
        const samePhoto = shown === stripShown;
        stripShown = shown;
        stripSource = src;
        const run = ++stripRun;
        if (samePhoto) {
            // Same photo, described anew (the album window around it was
            // fetched again): its neighbours are looked up from there and
            // swapped in together, so their views stay as they are.
            void (async () => {
                const next: Record<number, StripSlot | null | undefined> = { [stripAt]: { photo: shown, source: src } };
                const at = stripAt;
                for (const dir of [1, -1]) {
                    for (let k = at + dir; Math.abs(k - at) <= 4; k += dir) {
                        const from = next[k - dir];
                        if (!from) break;
                        next[k] = await previewNeighbour(from.photo, from.source, dir);
                        if (run !== stripRun || at !== stripAt) return;
                    }
                }
                strip = next;
            })();
            return;
        }
        stripAt = 0;
        strip = { 0: { photo: shown, source: src } };
        placePager();
        void fillStrip();
    }
    $effect(() => {
        void previewPhoto;
        void previewSource;
        untrack(syncStrip);
    });

    /** Looks up the places on either side of the photo on display. */
    async function fillStrip() {
        const run = stripRun;
        let found = false;
        for (const d of [1, 2, 3, 4]) {
            for (const dir of [1, -1]) {
                const k = stripAt + dir * d;
                if (strip[k] !== undefined) continue;
                const from = strip[k - dir];
                if (!from) continue;
                const hit = await previewNeighbour(from.photo, from.source, dir);
                if (run !== stripRun) return;
                if (strip[k] === undefined) strip = { ...strip, [k]: hit };
                found = true;
            }
        }
        // The scroller may have come to rest on a place before its photo was known.
        if (found && !pagerRestTimer) pagerAtRest();
    }

    /** Puts the photo at a place on display. */
    function showStripAt(k: number, place = false): boolean {
        const slot = strip[k];
        if (!slot) return false;
        if (k === stripAt) return true;
        stripAt = k;
        // Not arrived at by scrolling: the scroller goes there first.
        if (place) placePager();
        const seq = ++previewSeq;
        previewPhoto = slot.photo;
        previewSource = slot.source;
        stripShown = previewPhoto;
        stripSource = slot.source;
        // Places far behind are let go.
        const kept: Record<number, StripSlot | null | undefined> = {};
        for (const key of Object.keys(strip)) {
            const n = Number(key);
            if (Math.abs(n - k) <= 8) kept[n] = strip[n];
        }
        strip = kept;
        if (slot.source.kind === 'album') void refreshAlbumWindow(slot.source.window, slot.source.pos, seq);
        void fillStrip();
        return true;
    }

    // The photo on display changes once the scroller has come to rest on a
    // place, not while it moves: laying the places out again under a moving
    // scroller makes the system drop its animation and jump to the nearest
    // stop. At rest means no scroll event for a moment and exactly on a
    // stop (fingers holding the photo halfway are not at rest).
    let pagerRestTimer: ReturnType<typeof setTimeout> | null = null;
    function handlePagerScroll() {
        const el = pagerEl;
        if (!el || !el.clientWidth || !previewPhoto) return;
        // The area changed size: the same photo, at its place in the new size.
        if (el.clientWidth !== pagerWidth) {
            placePager();
            return;
        }
        if (pagerRestTimer) clearTimeout(pagerRestTimer);
        pagerRestTimer = setTimeout(pagerAtRest, 100);
    }
    function pagerAtRest() {
        pagerRestTimer = null;
        const el = pagerEl;
        if (!el || !pagerWidth || el.clientWidth !== pagerWidth || !previewPhoto) return;
        const place = el.scrollLeft / pagerWidth;
        const k = Math.round(place);
        if (Math.abs(place - k) * pagerWidth > 1) return;
        let at = k - STRIP_BASE;
        if (at === stripAt) return;
        if (strip[at] === undefined) return;
        // Past the end of the album: back to the last photo before it.
        const back = at > stripAt ? -1 : 1;
        while (at !== stripAt && !strip[at]) at += back;
        if (at !== stripAt) showStripAt(at);
        if (at !== k - STRIP_BASE) el.scrollTo({ left: (STRIP_BASE + at) * pagerWidth, behavior: "smooth" });
    }
    $effect(() => {
        const el = pagerEl;
        if (!el) return;
        placePager();
        const observer = new ResizeObserver(() => {
            if (el.clientWidth !== pagerWidth) placePager();
        });
        observer.observe(el);
        return () => observer.disconnect();
    });

    // What is laid out: the photo on display, a full view on either side,
    // and thumbnails for the three beyond each, which is how far swipes can
    // run on without the scroller coming to rest in between.
    let stripPages = $derived.by(() => {
        const pages: { k: number; photo: LibraryItem | null; full: boolean }[] = [];
        if (!previewPhoto) return pages;
        pages.push({ k: stripAt, photo: previewPhoto, full: true });
        for (const dir of [1, -1]) {
            for (let d = 1; d <= 4; d++) {
                const slot = strip[stripAt + dir * d];
                if (slot === null) break;
                pages.push({ k: stripAt + dir * d, photo: slot?.photo ?? null, full: d === 1 });
            }
        }
        return pages;
    });

    /** The arrow keys and the buttons: straight to the next place. */
    async function navigatePreview(direction: "prev" | "next") {
        if (!previewPhoto) return;
        syncStrip();
        const k = stripAt + (direction === "prev" ? -1 : 1);
        if (strip[k] === undefined) await fillStrip();
        showStripAt(k, true);
    }

    /** Desktop-only checks for the agent self-test: timeline, drawer paging, viewer. */
    async function agentDesktopSteps() {
        const out: { name: string; ok: boolean; detail?: unknown }[] = [];
        // Each step is named in the app's log as it finishes: a run that
        // stops with no report shows how far it got.
        const said = (text: string) => invoke("mac_map_command", { command: "mapDebugLog", body: { text } }).catch(() => {});
        const push = out.push.bind(out);
        out.push = (...items) => {
            for (const item of items) said(`step ${item.name}: ${item.ok ? "ok" : "NO"}`);
            return push(...items);
        };
        said("desktop steps begin");
        let restoreFilter = () => {};
        const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
        const until = async (check: () => boolean, ms = 5000) => {
            const end = performance.now() + ms;
            while (performance.now() < end) {
                if (check()) return true;
                await sleep(50);
            }
            return check();
        };

        // Timeline is on screen and has drawn something.
        {
            const canvas = document.querySelector<HTMLCanvasElement>(".timeline-container canvas");
            let painted = 0;
            if (canvas && canvas.width > 0) {
                const data = canvas.getContext("2d")!.getImageData(0, 0, canvas.width, 1).data;
                for (let i = 3; i < data.length; i += 4) if (data[i] > 0) painted++;
            }
            out.push({
                name: "desktop:timeline",
                ok: painted > 0,
                detail: { canvasWidth: canvas?.width ?? 0, paintedColumns: painted },
            });
        }

        // Drawer: first page, a far page, a re-sort.
        {
            // Saved drawer settings must not decide what the check sees, nor
            // where the camera steps above left the map: these checks are
            // about the whole library.
            const savedFilter = mediaFilter;
            mediaFilter = "all";
            if (mapViewTimer) clearTimeout(mapViewTimer);
            mapView = null;
            const t0 = performance.now();
            showLibrary = true;
            const ready = await until(() => libraryLoaded && libraryCounts.total > 0);
            const firstMs = Math.round(performance.now() - t0);
            await sleep(200);
            await invoke("mac_map_command", {
                command: "mapDebugSnapshot",
                body: { path: dbPath.replace(/footos\.db$/, "window-library.png") },
            }).catch(() => {});
            await sleep(600);
            const cells = document.querySelectorAll("button[data-hover-text] img").length;
            // Pinching on the list changes the size of the thumbnails.
            {
                const list = document.querySelector<HTMLElement>("[data-library-drawer] .overflow-y-auto");
                const cellWidth = () => document.querySelector<HTMLElement>("button[data-hover-text]")?.getBoundingClientRect().width ?? 0;
                const pinch = async (scale: number) => {
                    list?.dispatchEvent(new Event("gesturestart", { cancelable: true }));
                    for (let i = 1; i <= 5; i++) {
                        const step = new Event("gesturechange", { cancelable: true }) as Event & { scale?: number };
                        step.scale = 1 + ((scale - 1) * i) / 5;
                        list?.dispatchEvent(step);
                        await sleep(30);
                    }
                    list?.dispatchEvent(new Event("gestureend", { cancelable: true }));
                    await sleep(300);
                };
                // The size is remembered between runs and has limits: start
                // from the middle, or a pinch may have nowhere to go.
                await pinch(130 / Math.max(1, cellWidth()));
                const normal = cellWidth();
                await pinch(2);
                const bigger = cellWidth();
                await invoke("mac_map_command", {
                    command: "mapDebugSnapshot",
                    body: { path: dbPath.replace(/footos\.db$/, "window-library-pinch.png") },
                }).catch(() => {});
                await sleep(600);
                await pinch(0.3);
                const smaller = cellWidth();
                // Back to where it started (the size is remembered between runs).
                await pinch(normal / Math.max(1, smaller));
                out.push({
                    name: "desktop:library-pinch",
                    // A window hidden behind others does not take the made-up
                    // gestures; nothing can be concluded from such a run.
                    ok: document.hidden || (normal > 0 && bigger > normal * 1.4 && smaller < bigger * 0.6),
                    detail: {
                        normal: Math.round(normal), pinchedOut: Math.round(bigger), pinchedIn: Math.round(smaller), restored: Math.round(cellWidth()),
                        hidden: document.hidden, listFound: !!list, saved: localStorage.getItem("footos.libraryCellSize"),
                        listWidth: Math.round(list?.getBoundingClientRect().width ?? 0),
                    },
                });
            }

            // Half screen / full screen.
            {
                const drawer = document.querySelector<HTMLElement>("[data-library-drawer]");
                const toggle = document.querySelector<HTMLElement>("[data-library-wide]");
                const was = libraryWide;
                libraryWide = false;
                await sleep(200);
                const half = drawer?.getBoundingClientRect().width ?? 0;
                toggle?.click();
                await sleep(400);
                const full = drawer?.getBoundingClientRect().width ?? 0;
                await invoke("mac_map_command", {
                    command: "mapDebugSnapshot",
                    body: { path: dbPath.replace(/footos\.db$/, "window-library-full.png") },
                }).catch(() => {});
                await sleep(600);
                toggle?.click();
                await sleep(300);
                const back = drawer?.getBoundingClientRect().width ?? 0;
                libraryWide = was;
                out.push({
                    name: "desktop:library-width",
                    ok: half > 0 && Math.abs(full - window.innerWidth) < 2 && Math.abs(back - half) < 2 && half < full,
                    detail: { half: Math.round(half), full: Math.round(full), window: window.innerWidth },
                });
            }

            // Hovering a cell shows, on the cell, when the photo was taken.
            const secondCell = document.querySelectorAll<HTMLElement>("button[data-hover-text]")[1];
            secondCell?.dispatchEvent(new MouseEvent("mouseenter"));
            await sleep(150);
            const hoverTitle = document.querySelector("[data-cell-time]")?.textContent?.trim() ?? "";
            await invoke("mac_map_command", {
                command: "mapDebugSnapshot",
                body: { path: dbPath.replace(/footos\.db$/, "window-cell-hover.png") },
            }).catch(() => {});
            await sleep(600);
            secondCell?.dispatchEvent(new MouseEvent("mouseleave"));
            const total = libraryCounts.total;
            const t1 = performance.now();
            const last = await pager.item(total - 1);
            const farMs = Math.round(performance.now() - t1);
            // Always in the order the photos were taken, oldest first.
            const t2 = performance.now();
            const early = await pager.item(Math.floor(total / 3));
            const late = await pager.item(Math.floor((2 * total) / 3));
            const sortMs = Math.round(performance.now() - t2);
            const when = (p?: PhotoInfo) => (p as { takenMs?: number | null } | undefined)?.takenMs ?? null;
            const sorted = when(early) !== null && when(late) !== null && when(early)! <= when(late)!;
            restoreFilter = () => { mediaFilter = savedFilter; };
            out.push({
                name: "desktop:library",
                ok: ready && cells > 0 && !!last && sorted && total === libraryCounts.groupedTotal
                    && hoverTitle !== "",
                detail: { total, files: libraryCounts.filesTotal, firstPageMs: firstMs, farPageMs: farMs, resortMs: sortMs, cellsRendered: cells, hoverTitle },
            });
        }

        // Viewer from the drawer: step forward, wrap backward.
        {
            const first = await pager.item(0);
            let ok = false;
            let detail: unknown = "no items";
            if (first) {
                openFromLibrary(first, 0);
                await navigatePreview("next");
                await navigatePreview("next");
                const at2 = previewPosition.index;
                await navigatePreview("prev");
                await navigatePreview("prev");
                await navigatePreview("prev");
                const wrapped = previewPosition.index;
                ok = at2 === 2 && wrapped === libraryCounts.total - 1 && !!previewPhoto;
                detail = { afterTwoNext: at2, afterThreePrev: wrapped, total: libraryCounts.total };
                closePreview();
            }
            out.push({ name: "desktop:viewer-library", ok, detail });
        }

        // Viewer from the map: walk the time-ordered album.
        {
            const w = await queryAlbum(null, 0);
            let ok = false;
            let detail: unknown = "no items";
            if (w.total > 0) {
                const mid = await queryAlbum(null, 0);
                await openFromMap(mid.items[0]);
                const startIndex = previewPosition.index;
                const t0 = performance.now();
                // The album opens on its newest item, so walk back in time.
                for (let i = 0; i < 5; i++) await navigatePreview("prev");
                const stepMs = Math.round((performance.now() - t0) / 5);
                ok = previewPosition.index === startIndex - 5 && previewPosition.total === w.total;
                detail = { start: startIndex, end: previewPosition.index, total: previewPosition.total, msPerStep: stepMs };
                closePreview();
            }
            out.push({ name: "desktop:viewer-album", ok, detail });
        }

        // Photo details: the time is shown, the folder button lives in the
        // panel, and there is no way to delete the original file from here.
        {
            showLibrary = true;
            await until(() => libraryLoaded && libraryCounts.total > 0);
            const index = Math.floor(libraryCounts.total / 2);
            const item = await pager.item(index);
            if (item) await openFromLibrary(item, index);
            await until(() => !!document.querySelector("[data-show-in-finder]"), 3000);
            await sleep(400);
            const time = document.querySelector("[data-detail-time]")?.textContent?.replace(/\s+/g, " ").trim() ?? "";
            const folderButton = !!document.querySelector("[data-show-in-finder]");
            const deleteOriginal = [...document.querySelectorAll("button")].some((b) => b.textContent?.trim() === t.delete.deleteOriginal);
            await invoke("mac_map_command", {
                command: "mapDebugSnapshot",
                body: { path: dbPath.replace(/footos\.db$/, "window-detail.png") },
            }).catch(() => {});
            await sleep(600);
            closePreview();
            out.push({
                name: "desktop:photo-details",
                ok: time !== "" && folderButton && !deleteOriginal,
                detail: { time, folderButtonInPanel: folderButton, deleteOriginalButton: deleteOriginal },
            });
        }

        // Turning a photo in the viewer; a video's playback speed.
        {
            const click = (selector: string) => (document.querySelector(selector) as HTMLElement | null)?.click();
            const box = () => {
                const el = document.querySelector("[data-preview-image]") as HTMLElement | null;
                const r = el?.parentElement?.getBoundingClientRect();
                return { w: Math.round(r?.width ?? 0), h: Math.round(r?.height ?? 0), transform: el?.style.transform ?? "" };
            };
            showLibrary = true;
            mediaFilter = "photo";
            await until(() => libraryLoaded && libraryCounts.total > 0);
            await sleep(400);
            const photo = await pager.item(0);
            if (photo) await openFromLibrary(photo, 0);
            await until(() => box().w > 0, 4000);
            const upright = box();
            click('[data-preview-rotate="right"]');
            await sleep(300);
            const turned = box();
            await invoke("mac_map_command", {
                command: "mapDebugSnapshot",
                body: { path: dbPath.replace(/footos\.db$/, "window-rotated.png") },
            }).catch(() => {});
            await sleep(600);
            click('[data-preview-rotate="left"]');
            await sleep(200);
            const back = box();

            // A trackpad pinch over the large view zooms the photo, and the
            // list underneath keeps its cell size.
            const cellWidth = () => Math.round(
                (document.querySelector("[data-hover-text]") as HTMLElement | null)?.getBoundingClientRect().width ?? 0);
            const cellBefore = cellWidth();
            {
                const image = document.querySelector("[data-preview-image]") as HTMLElement | null;
                const r = image?.getBoundingClientRect();
                const x = (r?.left ?? 0) + (r?.width ?? 0) / 2;
                const y = (r?.top ?? 0) + (r?.height ?? 0) / 2;
                const pinch = (type: string, scale: number) => {
                    const e = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y });
                    Object.defineProperty(e, "scale", { value: scale });
                    (document.elementFromPoint(x, y) ?? image)?.dispatchEvent(e);
                };
                pinch("gesturestart", 1);
                pinch("gesturechange", 2);
                pinch("gestureend", 2);
            }
            await sleep(300);
            const pinched = box();
            const cellAfter = cellWidth();
            out.push({
                name: "desktop:preview-pinch-zooms-photo",
                ok: pinched.w > back.w * 1.5 && cellAfter === cellBefore,
                detail: { before: `${back.w}x${back.h}`, pinched: `${pinched.w}x${pinched.h}`, cellBefore, cellAfter },
            });

            // Two fingers sideways. The large view is one long sideways
            // scroller; the photos never change place in it and the code
            // never moves it during a swipe. Sliding along it, the middle of
            // the screen only ever shows the photos passed, in order, each
            // with a picture; where it comes to rest is the photo on
            // display. Slides started before the last one finished (fast
            // swiping) end on the right photo too. Zoomed in, the scroller
            // does not move. (What moves the scroller here is the page:
            // scripted wheel events do not scroll, and the app may not post
            // real ones.)
            {
                const pagerNow = () => document.querySelector("[data-preview-pager]") as HTMLElement | null;
                const zoomedLocked = getComputedStyle(pagerNow()!).overflowX === "hidden";
                closePreview();
                await sleep(200);
                if (photo) await openFromLibrary(photo, 0);
                await until(() => box().w > 0, 4000);
                const el = pagerNow()!;
                const pictureOf = (page: Element | null | undefined) =>
                    (page?.querySelector("img[alt]:not([alt=''])") ?? null) as HTMLImageElement | null;
                // Something to look at in the place: the picture itself, or
                // the thumbnail standing in while the picture loads.
                const drawn = (img: HTMLImageElement | null) => !!img && [...(img.closest("[data-preview-page]")?.querySelectorAll("img") ?? [])]
                    .some((i) => i.complete && i.naturalWidth > 0 && !i.classList.contains("opacity-0") && !i.classList.contains("hidden"));
                const pagesNow = () => [...el.querySelectorAll("[data-preview-page]")];
                const allDrawn = () => pagesNow().length === 9 && pagesNow().every((p) => drawn(pictureOf(p)));
                const neighboursLoaded = await until(allDrawn, 4000);
                const w = el.clientWidth;
                const r = el.getBoundingClientRect();
                const start = previewPosition.index;
                const home = el.scrollLeft;
                const snap = getComputedStyle(el).scrollSnapType;
                const names: string[] = [];
                for (let i = 0; i < 6; i++) names.push(((await pager.item(i))?.path ?? "").split("/").pop() ?? "");
                const middle = () => document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2)?.closest("[data-preview-page]");
                // Watches the middle of the screen frame by frame.
                const watch = async (ms: number) => {
                    const seen: string[] = [];
                    const trace: number[] = [];
                    let blank = 0;
                    let jumps = 0;
                    let last = el.scrollLeft;
                    const t0 = performance.now();
                    while (performance.now() - t0 < ms) {
                        await new Promise((f) => requestAnimationFrame(f));
                        const img = pictureOf(middle());
                        const name = img?.alt ?? "";
                        if (seen[seen.length - 1] !== name) seen.push(name);
                        if (!drawn(img)) blank++;
                        // The scroller only ever glides: a frame-to-frame
                        // move of half a page or more is a jump.
                        if (Math.abs(el.scrollLeft - last) > w / 2) jumps++;
                        if (trace.length < 40 && trace[trace.length - 1] !== Math.round(el.scrollLeft - home)) trace.push(Math.round(el.scrollLeft - home));
                        last = el.scrollLeft;
                    }
                    return { order: seen.join(">"), blank, jumps, index: previewPosition.index, left: el.scrollLeft - home, trace: trace.join(","), w, home, cw: el.clientWidth };
                };
                const to = (place: number) => el.scrollTo({ left: home + place * w, behavior: "smooth" });

                // One slide to the next photo, with a picture on the way.
                to(1);
                setTimeout(() => invoke("mac_map_command", {
                    command: "mapDebugSnapshot",
                    body: { path: dbPath.replace(/footos\.db$/, "window-swipe.png") },
                }).catch(() => {}), 130);
                const one = await watch(1200);
                // Fast: the next slide starts while the last still moves.
                const fastWatch = watch(1800);
                to(2);
                await sleep(120);
                to(3);
                await sleep(120);
                to(4);
                const fast = await fastWatch;
                // And back two.
                const backWatch = watch(1800);
                to(3);
                await sleep(100);
                to(2);
                const back = await backWatch;
                // The arrow key from here: straight to the next.
                await navigatePreview("next");
                await sleep(300);
                const key = { index: previewPosition.index, left: el.scrollLeft - home };

                const inOrder = (order: string, from: number, end: number) => {
                    const step = end > from ? 1 : -1;
                    const want: string[] = [];
                    for (let i = from; i !== end + step; i += step) want.push(names[i]);
                    return order === want.join(">");
                };
                const oneOk = inOrder(one.order, 0, 1) && one.blank === 0 && one.jumps === 0 && one.index === start + 1 && one.left === w;
                const fastOk = inOrder(fast.order, 1, 4) && fast.blank === 0 && fast.jumps === 0 && fast.index === start + 4 && fast.left === 4 * w;
                const backOk = inOrder(back.order, 4, 2) && back.blank === 0 && back.jumps === 0 && back.index === start + 2 && back.left === 2 * w;
                const keyOk = key.index === start + 3 && key.left === 3 * w;
                said(`swipe one ${JSON.stringify(one)}`);
                said(`swipe fast ${JSON.stringify(fast)}`);
                said(`swipe back ${JSON.stringify(back)}`);
                out.push({
                    name: "desktop:preview-swipe",
                    ok: zoomedLocked && neighboursLoaded && snap.includes("mandatory") && oneOk && fastOk && backOk && keyOk,
                    detail: { zoomedLocked, neighboursLoaded, width: w, oneOk, fastOk, backOk, keyOk, key: `${key.index}@${key.left}`, blankFrames: one.blank + fast.blank + back.blank, jumps: one.jumps + fast.jumps + back.jumps },
                });
            }
            closePreview();

            mediaFilter = "video";
            await sleep(600);
            await until(() => libraryLoaded && libraryCounts.total > 0);
            const video = await pager.item(0);
            if (video) await openFromLibrary(video, 0);
            await until(() => !!document.querySelector("[data-preview-video]"), 4000);
            // The seeded rows have no video file behind them: show a real
            // one (made by the test script) in the row's place.
            const sample = dbPath.replace(/footos\.db$/, "sample-video.mp4");
            if (previewPhoto) previewPhoto = { ...previewPhoto, path: sample };
            await sleep(300);
            const videoNow = () => document.querySelector("[data-preview-video]") as HTMLVideoElement | null;
            await until(() => (videoNow()?.readyState ?? 0) >= 2, 5000);
            click('[data-preview-speed="2"]');
            await sleep(200);
            const rate = videoNow()?.playbackRate ?? 0;
            click('[data-preview-rotate="right"]');
            await sleep(700);
            const videoTransform = videoNow()?.style.transform ?? "";
            const playable = (videoNow()?.readyState ?? 0) >= 2 && (videoNow()?.currentTime ?? 0) > 0;
            const bar = document.querySelector("[data-preview-controls]") as HTMLElement | null;
            const barRect = bar?.getBoundingClientRect();
            // The bar is a wide strip under the picture, not turned with it.
            const barUpright = !!barRect && barRect.width > barRect.height * 5
                && !videoNow()?.hasAttribute("controls");
            await invoke("mac_map_command", {
                command: "mapDebugSnapshot",
                body: { path: dbPath.replace(/footos\.db$/, "window-video.png") },
            }).catch(() => {});
            await sleep(600);
            click('[data-preview-speed="1"]');
            closePreview();
            mediaFilter = "all";
            const videos = libraryCounts.total;
            await until(() => libraryLoaded && libraryCounts.total !== videos, 4000);
            await sleep(300);
            out.push({
                name: "desktop:preview-rotate-speed",
                ok: turned.transform.includes("rotate(90deg)") && back.transform.includes("rotate(0deg)")
                    && back.w === upright.w && back.h === upright.h
                    && rate === 2 && videoTransform.includes("rotate(90deg)") && barUpright && playable,
                detail: { upright: `${upright.w}x${upright.h}`, turned: `${turned.w}x${turned.h}`, rate, videoTransform, barUpright, playable },
            });
        }

        // Position filter in the drawer: with, without, and back to everything.
        {
            closePreview();
            showLibrary = true;
            mediaFilter = "all";
            locationFilter = "all";
            await until(() => libraryLoaded && libraryCounts.total > 0);
            const all = libraryCounts.total;
            const settle = async (expectChangeFrom: number) => {
                await until(() => libraryLoaded && libraryCounts.total !== expectChangeFrom, 3000);
                await sleep(300);
                return libraryCounts.total;
            };
            cycleLocationFilter();
            const located = await settle(all);
            await invoke("mac_map_command", {
                command: "mapDebugSnapshot",
                body: { path: dbPath.replace(/footos\.db$/, "window-located.png") },
            }).catch(() => {});
            await sleep(600);
            cycleLocationFilter();
            const unlocated = await settle(located);
            cycleLocationFilter();
            const restored = await settle(unlocated);
            // The drawer follows the map: zoomed to the densest spot it lists
            // fewer entries than the whole library; with everything in view
            // again, all of them.
            let regionCount = -1;
            let backToAll = -1;
            let timelineWhileDragging = -1;
            let timelineRanges: string[] = [];
            let presetCount = -1;
            let presetRange = "";
            let yearRange = "";
            let timelineGestures = { width30d: 0, afterPinchOut: 0, movedBy: 0, widthAfterMove: 0, rangeChanged: false };
            let yearSelected = false;
            {
                const world = await queryMap({ west: -180, south: -85, east: 180, north: 85 }, 1, null, null);
                const densest = [...world.features].sort((x, y) => y.count - x.count)[0];
                const theMap = (window as any).__footosMap;
                if (densest && theMap) {
                    theMap.easeTo({ center: [densest.lon, densest.lat], zoom: 7, duration: 0 });
                    regionCount = await settle(all);
                    await invoke("mac_map_command", {
                        command: "mapDebugSnapshot",
                        body: { path: dbPath.replace(/footos\.db$/, "window-region.png") },
                    }).catch(() => {});
                    await sleep(600);
                    // The timeline on top of the list: it selects the span of
                    // what the map shows; dragging an edge filters the list
                    // while the mouse is still down.
                    const rangeBefore = document.querySelector("[data-library-timeline-range]")?.textContent ?? "";
                    const handle = document.querySelector<HTMLElement>("[data-library-timeline] [data-handle='start']");
                    const track = handle?.parentElement?.getBoundingClientRect();
                    if (handle && track) {
                        const at = handle.getBoundingClientRect();
                        const x0 = at.left + at.width / 2;
                        const y = at.top + at.height / 2;
                        handle.dispatchEvent(new MouseEvent("mousedown", { clientX: x0, clientY: y, bubbles: true }));
                        const target = x0 + (track.right - x0) * 0.5;
                        for (let i = 1; i <= 10; i++) {
                            window.dispatchEvent(new MouseEvent("mousemove", { clientX: x0 + ((target - x0) * i) / 10, clientY: y, bubbles: true }));
                            await sleep(30);
                        }
                        timelineWhileDragging = await settle(regionCount);
                        await invoke("mac_map_command", {
                            command: "mapDebugSnapshot",
                            body: { path: dbPath.replace(/footos\.db$/, "window-list-timeline.png") },
                        }).catch(() => {});
                        await sleep(600);
                        window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
                    }
                    timelineRanges = [rangeBefore, document.querySelector("[data-library-timeline-range]")?.textContent ?? ""];
                    // Two fingers on the timeline: a pinch changes the length
                    // of the selection, a sideways swipe moves it.
                    {
                        const strip = document.querySelector<HTMLElement>("[data-library-timeline]");
                        const rangeText = () => document.querySelector("[data-library-timeline-range]")?.textContent ?? "";
                        const width = () => {
                            const h = [...document.querySelectorAll<HTMLElement>("[data-library-timeline] [data-handle]")];
                            return h.length === 2 ? h[1].getBoundingClientRect().left - h[0].getBoundingClientRect().left : 0;
                        };
                        const left = () => document.querySelector<HTMLElement>("[data-library-timeline] [data-handle='start']")?.getBoundingClientRect().left ?? 0;
                        document.querySelector<HTMLElement>("[data-span-preset='30d']")?.click();
                        await sleep(300);
                        const w0 = width();
                        strip?.dispatchEvent(new Event("gesturestart", { cancelable: true }));
                        const grow = new Event("gesturechange", { cancelable: true }) as Event & { scale?: number };
                        grow.scale = 3;
                        strip?.dispatchEvent(grow);
                        strip?.dispatchEvent(new Event("gestureend", { cancelable: true }));
                        await sleep(300);
                        const w1 = width();
                        const x1 = left();
                        const r1 = rangeText();
                        strip?.dispatchEvent(new WheelEvent("wheel", { deltaX: 80, deltaY: 0, cancelable: true, bubbles: true }));
                        await sleep(300);
                        timelineGestures = { width30d: Math.round(w0), afterPinchOut: Math.round(w1), movedBy: Math.round(left() - x1), widthAfterMove: Math.round(width()), rangeChanged: rangeText() !== r1 };
                    }

                    // One year around the middle of the selection.
                    document.querySelector<HTMLElement>("[data-span-preset='1y']")?.click();
                    await sleep(500);
                    yearRange = document.querySelector("[data-library-timeline-range]")?.textContent ?? "";
                    yearSelected = !!document.querySelector("[data-span-preset='1y'].bg-\\[var\\(--accent\\)\\]");
                    await invoke("mac_map_command", {
                        command: "mapDebugSnapshot",
                        body: { path: dbPath.replace(/footos\.db$/, "window-list-year.png") },
                    }).catch(() => {});
                    await sleep(600);
                    // A quick span: thirty days around the middle of the selection.
                    document.querySelector<HTMLElement>("[data-span-preset='30d']")?.click();
                    presetCount = await settle(libraryCounts.total);
                    presetRange = document.querySelector("[data-library-timeline-range]")?.textContent ?? "";
                    await invoke("mac_map_command", {
                        command: "mapDebugSnapshot",
                        body: { path: dbPath.replace(/footos\.db$/, "window-list-preset.png") },
                    }).catch(() => {});
                    await sleep(600);
                    const narrowed = libraryCounts.total;
                    theMap.easeTo({ center: [0, 20], zoom: 0, duration: 0 });
                    backToAll = await settle(narrowed);
                }
            }
            out.push({
                name: "desktop:library-timeline",
                ok: timelineWhileDragging > 0 && timelineWhileDragging < regionCount
                    && timelineRanges.length === 2 && timelineRanges[0] !== "" && timelineRanges[0] !== timelineRanges[1],
                detail: { inMapArea: regionCount, whileDragging: timelineWhileDragging, selectedBefore: timelineRanges[0], selectedAfter: timelineRanges[1] },
            });
            out.push({
                name: "desktop:library-timeline-gestures",
                ok: timelineGestures.width30d > 0 && timelineGestures.afterPinchOut > timelineGestures.width30d * 2
                    && timelineGestures.movedBy > 40 && Math.abs(timelineGestures.widthAfterMove - timelineGestures.afterPinchOut) <= 2
                    && timelineGestures.rangeChanged,
                detail: timelineGestures,
            });
            out.push({
                name: "desktop:library-timeline-preset",
                // How many photos a span holds depends on where the map
                // happens to be; what must hold is that the selection changed
                // and the one-year button took.
                ok: presetCount > 0 && presetRange !== "" && presetRange !== timelineRanges[1] && yearSelected && yearRange !== presetRange,
                detail: { after30d: presetCount, selected: presetRange, oneYear: yearRange, oneYearHighlighted: yearSelected },
            });
            out.push({
                name: "desktop:library-follows-map",
                ok: regionCount > 0 && regionCount < all && backToAll >= regionCount,
                detail: { all, zoomedIn: regionCount, zoomedOut: backToAll },
            });
            out.push({
                name: "desktop:location-filter",
                ok: located > 0 && unlocated > 0 && located + unlocated === all && restored === all,
                detail: { all, located, unlocated },
            });
        }

        showLibrary = false;
        restoreFilter();

        // System map (macOS): the native map lies under the web view, so
        // nothing of the page may paint over the middle of the map area.
        if (document.documentElement.classList.contains("native-map")) {
            // The viewer and the drawer from the steps above are legitimately on top.
            closePreview();
            await until(() => !document.querySelector(".fixed.inset-0.z-50"), 2000);
            await sleep(400);
            const frame = (window as any).__footosMapStats ? document.querySelector(".controls")?.parentElement : null;
            const r = frame?.getBoundingClientRect();
            const painted: string[] = [];
            if (r) {
                for (const el of document.elementsFromPoint(r.left + r.width / 2, r.top + r.height / 2)) {
                    const style = getComputedStyle(el);
                    const bg = style.backgroundColor;
                    const opaque = bg && bg !== "transparent" && !/rgba\(.*,\s*0\)$/.test(bg);
                    if (opaque || style.backgroundImage !== "none") painted.push(`${el.tagName}.${(el as HTMLElement).className}`.slice(0, 80));
                }
            }
            // Move the pointer across the map and the controls. Things that
            // only happen when a pointer is over the window (tracking areas,
            // hover) have crashed the app before.
            {
                const area = document.querySelector(".controls")?.parentElement?.getBoundingClientRect();
                if (area) {
                    for (let i = 0; i <= 24; i++) {
                        await invoke("mac_map_command", {
                            command: "mapDebugMouseMove",
                            body: { x: area.left + (area.width * i) / 24, y: area.top + area.height * (0.2 + 0.6 * ((i % 5) / 4)) },
                        }).catch(() => {});
                        await sleep(40);
                    }
                }
                await invoke("mac_map_command", { command: "mapDebugProbeViews", body: {} }).catch(() => {});
                await sleep(300);
                out.push({ name: "desktop:pointer-over-map", ok: !!area, detail: { moves: 25 } });
            }

            // Hovering a marker previews a single photo, never a cluster.
            {
                // The busiest spot at street level: clusters and single photos side by side.
                const world = await queryMap({ west: -180, south: -85, east: 180, north: 85 }, 14.5, null, null);
                const densest = [...world.features].sort((x, y) => y.count - x.count)[0];
                const theMap = (window as any).__footosMap;
                const native = (window as any).__footosNativeMap;
                let onCluster: boolean | null = null;
                let onSingle: boolean | null = null;
                if (densest && theMap && native) {
                    theMap.easeTo({ center: [densest.lon, densest.lat], zoom: 14.5, duration: 0 });
                    await sleep(1500);
                    // Well inside what the map has loaded at this zoom.
                    const d = 0.012;
                    const near = await queryMap(
                        { west: densest.lon - d, south: densest.lat - d, east: densest.lon + d, north: densest.lat + d },
                        14.5, null, null,
                    );
                    const cluster = near.features.find((f) => f.count > 1);
                    const single = near.features.find((f) => f.count === 1);
                    const shown = () => !!document.querySelector(".hover");
                    if (cluster) {
                        native.onHover({ key: cluster.key, x: 400, y: 300 });
                        await sleep(150);
                        onCluster = shown();
                        native.onHoverEnd({ key: cluster.key });
                    }
                    if (single) {
                        native.onHover({ key: single.key, x: 400, y: 300 });
                        await sleep(150);
                        onSingle = shown();
                        native.onHoverEnd({ key: single.key });
                        await sleep(100);
                    }
                }
                out.push({
                    name: "desktop:hover-preview",
                    ok: onCluster === false && onSingle === true,
                    detail: { previewOnCluster: onCluster, previewOnSingle: onSingle },
                });
            }

            // The import menu drops out of the narrow button column: a click
            // on an item's text (not only its icon) must stay with the page.
            {
                const plus = document.querySelector<HTMLElement>("button[title='" + t.import.importPhotos + "']");
                plus?.click();
                await sleep(700);
                const item = [...document.querySelectorAll<HTMLElement>("button")].find((b) => b.textContent?.trim() === t.import.importFile);
                const r = item?.getBoundingClientRect();
                if (r) {
                    await invoke("mac_map_command", { command: "mapDebugHit", body: { label: "import menu text", x: r.right - 12, y: r.top + r.height / 2 } }).catch(() => {});
                    await invoke("mac_map_command", { command: "mapDebugHit", body: { label: "import menu icon", x: r.left + 14, y: r.top + r.height / 2 } }).catch(() => {});
                }
                await sleep(300);
                // Close it again (the click-catcher behind the menu).
                document.querySelector<HTMLElement>(".fixed.inset-0.z-40")?.click();
                await sleep(300);
                out.push({ name: "desktop:import-menu-open", ok: !!r, detail: { width: Math.round(r?.width ?? 0) } });
            }

            // The filter menu, open.
            (document.querySelector("[data-media-filter]") as HTMLElement | null)?.click();
            await sleep(700);
            await invoke("mac_map_command", {
                command: "mapDebugSnapshot",
                body: { path: dbPath.replace(/footos\.db$/, "window-filter.png") },
            }).catch(() => {});
            await sleep(600);
            (document.querySelector("[data-media-filter]") as HTMLElement | null)?.click();
            await sleep(400);
            // A picture of the window, so whoever runs the test can look at it.
            await invoke("mac_map_command", {
                command: "mapDebugSnapshot",
                body: { path: dbPath.replace(/footos\.db$/, "window.png") },
            }).catch(() => {});
            await sleep(800);
            out.push({
                name: "desktop:native-map-uncovered",
                ok: !!r && r.width > 200 && painted.length === 0,
                detail: { width: Math.round(r?.width ?? 0), height: Math.round(r?.height ?? 0), painted },
            });
        }

        // SCENARIO=clear: clear everything while the background thumbnail pass
        // is busy. That used to fail half way and leave the app without a
        // database. Wipes the test library: run with FRESH=1 afterwards.
        if ((await invoke("agent_test_enabled")) === "clear") {
            const before = await photoCount();
            await invoke("sharpen_thumbnails");
            await sleep(1500);
            let failed = "";
            const t0 = performance.now();
            await invoke("clear_app_data").catch((e) => { failed = String(e); });
            const clearMs = Math.round(performance.now() - t0);
            let after = -1;
            let usable = false;
            try {
                after = await photoCount();
                await queryLibrary("date", true, "all", "all", null, 0, 10);
                usable = true;
            } catch (e) {
                failed = failed || String(e);
            }
            out.push({
                name: "desktop:clear-while-busy",
                ok: !failed && usable && before > 0 && after === 0,
                detail: { before, after, clearMs, failed },
            });
        }

        // Import a folder of real files, rescan it, see it in Settings, remove it.
        const importDir = (await invoke("agent_test_import_dir")) as string | null;
        if (importDir) {
            const before = await photoCount();
            let progressEvents = 0;
            const off = platformService.onImportProgress(() => { progressEvents++; });
            const t0 = performance.now();
            const first: any = await invoke("import_photos", { rootPath: importDir });
            const importMs = Math.round(performance.now() - t0);
            off();
            const afterImport = await photoCount();
            const again: any = await invoke("import_photos", { rootPath: importDir });
            const roots: any[] = await invoke("list_library_roots");
            const root = roots.find((r) => r.photos === first.success);
            out.push({
                name: "desktop:import-folder",
                ok: first.success > 0 && first.failure === 0 && afterImport - before === first.success
                    && again.success === 0 && !!root && root.available && progressEvents > 0,
                detail: { importMs, success: first.success, duplicates: first.duplicates, failure: first.failure,
                    added: afterImport - before, secondImport: again.success, roots: roots.length, progressEvents },
            });

            const rescan: any = await invoke("rescan_library");
            out.push({
                name: "desktop:rescan-unchanged",
                ok: rescan.roots >= 1 && rescan.added + rescan.updated + rescan.moved + rescan.removed === 0
                    && (await photoCount()) === afterImport,
                detail: rescan,
            });

            showSettings = true;
            const listed = await until(() => !!root && document.body.innerText.includes(root.path));
            out.push({ name: "desktop:settings-folders", ok: listed, detail: { path: root?.path } });
            showSettings = false;

            const removed = root ? ((await invoke("remove_library_root", { path: root.path, removePhotos: true })) as number) : 0;
            const afterRemove = await photoCount();
            out.push({
                name: "desktop:remove-folder",
                ok: removed === first.success && afterRemove === before
                    && ((await invoke("list_library_roots")) as any[]).length === roots.length - 1,
                detail: { removed, before, afterRemove },
            });
        }
        return out;
    }

    // Delete state
    let deleteConfirmOpen = $state(false);
    let deleteMode = $state<'app' | 'complete'>('app');
    let deleteIncludeRaw = $state(true);
    let isDeleting = $state(false);

    async function handleDeletePhoto(photo: LibraryItem, mode: 'app' | 'complete', includeRaw: boolean = true) {
        if (!photo.id) return;
        const src = previewSource;

        isDeleting = true;
        try {
            const ids = [photo.id.id];
            if (includeRaw && photo.hasRaw && photo.rawId != null) {
                ids.push(photo.rawId);
            }

            // Where to go afterwards, decided before the rows disappear.
            let neighbour: { id: number; path: string } | null = null;
            if (src?.kind === 'album') {
                const gone = new Set(ids);
                const { items } = src.window;
                for (let d = 1; d < items.length && !neighbour; d++) {
                    for (const candidate of [items[src.pos + d], items[src.pos - d]]) {
                        if (candidate && !gone.has(candidate.id)) {
                            neighbour = { id: candidate.id, path: candidate.path };
                            break;
                        }
                    }
                }
            }

            const command = mode === 'complete' ? 'delete_photos_completely' : 'delete_photos_from_app';
            const result = await invoke(command, { ids });
            console.log('Delete result:', result);

            refreshKey++;
            photoCount().then((n) => { libraryCount = n; }).catch(() => {});

            if (src?.kind === 'library') {
                // The list closes up, so the same index now holds the next photo.
                pager.reset();
                const seq = ++previewSeq;
                const first = await pager.item(0);
                const total = pager.counts.total;
                if (seq !== previewSeq) return;
                if (total === 0 || !first) {
                    closePreview();
                } else {
                    const index = Math.min(src.index, total - 1);
                    const item = await pager.item(index);
                    if (seq !== previewSeq) return;
                    if (item) {
                        previewPhoto = item;
                        previewSource = { kind: 'library', index };
                    } else {
                        closePreview();
                    }
                }
            } else {
                if (showLibrary) pager.reset();
                if (neighbour) await openFromMap(neighbour);
                else closePreview();
            }
        } catch (e) {
            console.error('Delete failed:', e);
            error = t.errors.deleteFailed + ': ' + e;
        } finally {
            isDeleting = false;
            deleteConfirmOpen = false;
        }
    }

    function openDeleteConfirm(mode: 'app' | 'complete') {
        deleteMode = mode;
        deleteIncludeRaw = true;
        deleteConfirmOpen = true;
    }

    function handleKeydown(e: KeyboardEvent) {
        if (e.key === "Escape") {
            if (previewPhoto) {
                closePreview();
                return;
            }
            if (showSettings) {
                showSettings = false;
                return;
            }
            if (showLibrary) {
                showLibrary = false;
                return;
            }
        }

        if (previewPhoto) {
            switch (e.key) {
                case "ArrowLeft":
                    navigatePreview("prev");
                    break;
                case "ArrowRight":
                    navigatePreview("next");
                    break;
            }
            return;
        }

        switch (e.key) {
            case "i":
                if (e.metaKey || e.ctrlKey) {
                    e.preventDefault();
                    handleScan("folder");
                }
                break;
        }
    }
</script>

<svelte:window onkeydown={handleKeydown} />

<main class="fixed inset-0 flex flex-col theme-bg-primary theme-text-primary overflow-hidden">
    <!-- Fullscreen Map -->
    <div class="flex-1 relative">
        <MapView {refreshKey} {libraryCount} onOpenPreview={openFromMap} onOpenLibrary={handleLibrary} onViewChange={handleMapViewChange} theme={effectiveTheme} {t} {locale} />

        <!-- Floating action buttons (top-left, vertical) -->
        <div class="absolute top-4 left-4 z-[1001] flex flex-col gap-2">
            <!-- Import button with dropdown -->
            <div class="relative">
                <button
                    onclick={() => floatingImportMenuOpen = !floatingImportMenuOpen}
                    disabled={isScanning}
                    class="w-12 h-12 rounded-full theme-bg-card border theme-border theme-text-secondary hover:theme-text-primary disabled:opacity-50 flex items-center justify-center transition-all"
                    title={t.import.importPhotos}
                >
                    <i class="fa-solid {isScanning ? 'fa-spinner fa-spin' : 'fa-plus'} text-lg"></i>
                </button>
                {#if floatingImportMenuOpen}
                    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                    <div
                        class="fixed inset-0 z-40"
                        onclick={() => floatingImportMenuOpen = false}
                    ></div>
                    <div class="absolute left-0 top-full mt-2 py-1 theme-bg-secondary border theme-border rounded-lg shadow-lg z-50 min-w-[140px]">
                        <button
                            onclick={() => { floatingImportMenuOpen = false; handleScan("folder"); }}
                            disabled={isScanning}
                            class="w-full px-3 py-2 text-left text-sm theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary flex items-center gap-2"
                        >
                            <i class="fa-solid fa-folder w-4"></i>
                            {t.import.importFolder}
                        </button>
                        <button
                            onclick={() => { floatingImportMenuOpen = false; handleScan("file"); }}
                            disabled={isScanning}
                            class="w-full px-3 py-2 text-left text-sm theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary flex items-center gap-2"
                        >
                            <i class="fa-solid fa-file-image w-4"></i>
                            {t.import.importFile}
                        </button>
                        <button
                            onclick={() => { floatingImportMenuOpen = false; handleRescan(); }}
                            disabled={isScanning}
                            class="w-full px-3 py-2 text-left text-sm theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary flex items-center gap-2"
                        >
                            <i class="fa-solid fa-rotate w-4"></i>
                            {t.import.rescan}
                        </button>
                    </div>
                {/if}
            </div>

            <!-- Same order as on the phone: add, settings, filter. -->
            <!-- Settings button -->
            <button
                onclick={() => showSettings = true}
                class="w-12 h-12 rounded-full theme-bg-card border theme-border theme-text-secondary hover:theme-text-primary flex items-center justify-center transition-all"
                title={t.common.settings}
            >
                <i class="fa-solid fa-gear text-lg"></i>
            </button>

            <MediaFilterButton value={mapMediaFilter} onChange={setMapMediaFilter} {t} />

            <!-- Library button: with the system map it sits at the bottom right
                 of the map, where the phone has its album button. -->
            {#if !platformService.capabilities.nativeMap}
                <button
                    onclick={() => handleLibrary()}
                    class="w-12 h-12 rounded-full theme-bg-card border theme-border theme-text-secondary hover:theme-text-primary flex items-center justify-center transition-all {showLibrary ? 'theme-bg-secondary' : ''}"
                    title={t.nav.library}
                >
                    <i class="fa-solid fa-images text-lg"></i>
                </button>
            {/if}
        </div>

        <!-- Library Drawer (right side) -->
        {#if showLibrary}
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            <div
                class="fixed inset-0 bg-black/30 z-[1002]"
                onclick={() => showLibrary = false}
            ></div>
            <div class="absolute top-0 right-0 bottom-0 {libraryWide ? 'w-full' : 'w-1/2 min-w-[500px]'} theme-bg-overlay backdrop-blur-md border-l theme-border z-[1003] flex flex-col" data-library-drawer>
                <!-- Header -->
                <div class="flex items-center justify-between px-4 py-3 border-b theme-border">
                    <div class="flex items-center gap-2">
                        <i class="fa-solid fa-images theme-text-muted text-sm"></i>
                        <span class="theme-text-primary font-medium">
                            {libraryCounts.total} {t.nav.photos}
                            {#if libraryCounts.total !== libraryCounts.groupedTotal}
                                <span class="theme-text-muted text-xs">/ {libraryCounts.groupedTotal}</span>
                            {:else if libraryCounts.total !== libraryCounts.filesTotal}
                                <span class="theme-text-muted text-xs">({libraryCounts.filesTotal} {t.nav.files})</span>
                            {/if}
                        </span>
                    </div>
                    <div class="flex items-center gap-1">
                        <!-- Half of the window, or all of it -->
                        <button
                            onclick={() => libraryWide = !libraryWide}
                            data-library-wide={libraryWide}
                            class="p-1.5 rounded-lg hover:theme-bg-secondary theme-text-muted hover:theme-text-primary transition-colors"
                            title={libraryWide ? t.library.halfScreen : t.library.fullScreen}
                            aria-label={libraryWide ? t.library.halfScreen : t.library.fullScreen}
                        >
                            <i class="fa-solid {libraryWide ? 'fa-compress' : 'fa-expand'}"></i>
                        </button>
                        <button
                            onclick={() => showLibrary = false}
                            class="p-1.5 rounded-lg hover:theme-bg-secondary theme-text-muted hover:theme-text-primary transition-colors"
                            title={t.common.close}
                        >
                            <i class="fa-solid fa-xmark"></i>
                        </button>
                    </div>
                </div>

                <!-- Toolbar -->
                <div class="flex items-center gap-1.5 px-3 py-2 border-b theme-border">
                    <!-- Media type filter pills -->
                    <div class="flex items-center gap-0.5 px-0.5 py-0.5 rounded theme-bg-primary">
                        <button
                            onclick={() => mediaFilter = 'all'}
                            class="h-[22px] px-2 rounded text-[11px] transition-all {mediaFilter === 'all'
                                ? 'theme-bg-tertiary theme-text-primary font-medium shadow-sm'
                                : 'theme-text-muted hover:theme-text-secondary'}"
                        >
                            {t.library.filterAll}
                        </button>
                        <button
                            onclick={() => mediaFilter = 'photo'}
                            class="h-[22px] px-2 rounded text-[11px] transition-all {mediaFilter === 'photo'
                                ? 'theme-bg-tertiary theme-text-primary font-medium shadow-sm'
                                : 'theme-text-muted hover:theme-text-secondary'}"
                        >
                            <i class="fa-solid fa-image text-[9px] mr-1"></i>{t.library.filterPhotos}
                        </button>
                        <button
                            onclick={() => mediaFilter = 'video'}
                            class="h-[22px] px-2 rounded text-[11px] transition-all {mediaFilter === 'video'
                                ? 'theme-bg-tertiary theme-text-primary font-medium shadow-sm'
                                : 'theme-text-muted hover:theme-text-secondary'}"
                        >
                            <i class="fa-solid fa-video text-[9px] mr-1"></i>{t.library.filterVideos}
                        </button>
                        <button
                            onclick={() => mediaFilter = 'live_photo'}
                            class="h-[22px] px-2 rounded text-[11px] transition-all {mediaFilter === 'live_photo'
                                ? 'theme-bg-tertiary theme-text-primary font-medium shadow-sm'
                                : 'theme-text-muted hover:theme-text-secondary'}"
                        >
                            <i class="fa-solid fa-bolt text-[9px] mr-1"></i>{t.library.filterLivePhotos}
                        </button>
                    </div>

                    <!-- Position filter: click to go through all / with / without -->
                    <button
                        onclick={cycleLocationFilter}
                        data-location-filter={locationFilter}
                        title={t.library.locationFilter}
                        class="h-[26px] ml-1.5 px-2 rounded text-[11px] whitespace-nowrap transition-all {locationFilter === 'all'
                            ? 'theme-bg-primary theme-text-muted hover:theme-text-secondary'
                            : 'bg-[var(--accent)] text-black font-medium'}"
                    >
                        <i class="fa-solid fa-location-dot text-[10px] mr-1"></i>{locationFilter === 'located'
                            ? t.library.locationWith
                            : locationFilter === 'unlocated' ? t.library.locationWithout : t.library.locationAny}
                    </button>

                    <div class="flex-1"></div>

                    <div class="relative">
                        <button
                            onclick={() => libraryImportMenuOpen = !libraryImportMenuOpen}
                            disabled={isScanning}
                            class="w-7 h-7 rounded theme-bg-tertiary theme-text-secondary hover:theme-text-primary disabled:opacity-50 flex items-center justify-center"
                            title={t.common.import}
                        >
                            <i class="fa-solid {isScanning ? 'fa-spinner fa-spin' : 'fa-plus'} text-xs"></i>
                        </button>
                        {#if libraryImportMenuOpen}
                            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                            <div
                                class="fixed inset-0 z-40"
                                onclick={() => libraryImportMenuOpen = false}
                            ></div>
                            <div class="absolute right-0 top-full mt-1 py-1 theme-bg-secondary border theme-border rounded shadow-lg z-50 min-w-[130px]">
                                <button
                                    onclick={() => { libraryImportMenuOpen = false; handleScan("folder"); }}
                                    disabled={isScanning}
                                    class="w-full px-3 py-1.5 text-left text-xs theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary flex items-center gap-2"
                                >
                                    <i class="fa-solid fa-folder w-3"></i>
                                    {t.import.importFolder}
                                </button>
                                <button
                                    onclick={() => { libraryImportMenuOpen = false; handleScan("file"); }}
                                    disabled={isScanning}
                                    class="w-full px-3 py-1.5 text-left text-xs theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary flex items-center gap-2"
                                >
                                    <i class="fa-solid fa-file-image w-3"></i>
                                    {t.import.importFile}
                                </button>
                        <button
                            onclick={() => { libraryImportMenuOpen = false; handleRescan(); }}
                            disabled={isScanning}
                            class="w-full px-3 py-1.5 text-left text-xs theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary flex items-center gap-2"
                        >
                            <i class="fa-solid fa-rotate w-3"></i>
                            {t.import.rescan}
                        </button>
                            </div>
                        {/if}
                    </div>
                </div>

                <!-- Timeline of the whole library; the selection filters the list -->
                <LibraryTimeline
                    stamps={libraryStamps}
                    start={listSpan?.start ?? libraryCounts.regionSpan?.[0] ?? null}
                    end={listSpan?.end ?? libraryCounts.regionSpan?.[1] ?? null}
                    onChange={(start, end) => { listSpan = { start, end }; }}
                    {t}
                    {locale}
                />

                <!-- Photo Grid (Virtual Scrolling) -->
                {#if libraryCounts.total > 0}
                    <VirtualPhotoGrid
                        total={libraryCounts.total}
                        getItem={(index) => pager.peek(index)}
                        version={libraryVersion}
                        onPhotoClick={openFromLibrary}
                    />
                {:else if !isScanning && libraryLoaded}
                    <div class="flex-1 flex flex-col items-center justify-center text-white/50 py-12">
                        <i class="fa-solid fa-images text-2xl mb-3"></i>
                        <p class="text-sm">{t.library.noPhotos}</p>
                        <button
                            onclick={() => handleScan("folder")}
                            class="mt-3 px-3 py-1.5 rounded bg-white/10 text-white/80 text-xs hover:bg-white/20"
                        >
                            {t.import.importPhotos}
                        </button>
                    </div>
                {:else}
                    <div class="flex-1 flex flex-col items-center justify-center text-white/50 py-12">
                        <i class="fa-solid fa-spinner fa-spin text-xl mb-2"></i>
                        <p class="text-sm">
                            {#if importStatus.total > 0}
                                {importStatus.current} / {importStatus.total}
                            {:else}
                                {t.common.scanning}...
                            {/if}
                        </p>
                    </div>
                {/if}
            </div>
        {/if}

        <!-- Import progress indicator -->
        {#if isScanning}
            <div class="absolute top-4 left-16 z-[1001] px-3 py-2 rounded-full bg-black/70 backdrop-blur-sm border border-white/20 text-sm text-white/80 flex items-center gap-2">
                <i class="fa-solid fa-spinner fa-spin text-xs"></i>
                {#if importStatus.total > 0}
                    <span>{importStatus.current} / {importStatus.total}</span>
                    {#if importStatus.duplicates > 0}
                        <span class="text-yellow-400" title={t.import.duplicates}>({importStatus.duplicates} {t.import.duplicates})</span>
                    {/if}
                {:else}
                    <span>{t.common.scanning}...</span>
                {/if}
                <button
                    onclick={handleCancelImport}
                    class="ml-1 w-5 h-5 rounded-full bg-white/10 hover:bg-red-500/50 flex items-center justify-center transition-colors"
                    title={t.common.cancel}
                >
                    <i class="fa-solid fa-xmark text-[10px]"></i>
                </button>
            </div>
        {/if}

        <!-- Error message -->
        {#if error}
            <div class="absolute top-4 left-1/2 -translate-x-1/2 z-[1001] px-4 py-2 rounded-lg bg-red-900/80 backdrop-blur-sm border border-red-700/50 text-red-200 text-sm">
                {error}
            </div>
        {/if}
    </div>
</main>

<!-- Settings Modal -->
{#if showSettings}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div
        class="fixed inset-0 z-[2000] bg-black/60 backdrop-blur-sm flex items-center justify-center"
        onclick={() => showSettings = false}
    >
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
            class="theme-bg-secondary border theme-border rounded-2xl shadow-2xl w-full max-w-lg max-h-[80vh] overflow-hidden"
            onclick={(e) => e.stopPropagation()}
        >
            <div class="flex items-center justify-between px-6 py-4 border-b theme-border">
                <h2 class="text-lg font-medium theme-text-primary">{t.settings.title}</h2>
                <button
                    onclick={() => showSettings = false}
                    class="p-2 rounded-lg hover:theme-bg-tertiary theme-text-muted hover:theme-text-primary transition-colors"
                >
                    <i class="fa-solid fa-xmark"></i>
                </button>
            </div>
            <div class="p-6 overflow-y-auto max-h-[calc(80vh-60px)]">
                <Settings {dbPath} {thumbDir} {version} {theme} onThemeChange={(newTheme) => theme = newTheme} {t} {locale} onLocaleChange={(newLocale) => i18n.setLocale(newLocale)} />
            </div>
        </div>
    </div>
{/if}

{#if previewPhoto}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div
        class="fixed inset-0 z-50 bg-black/90"
        onclick={closePreview}
    >
        <!-- Top bar -->
        <div class="absolute top-0 left-0 right-64 h-12 flex items-center justify-between px-4 z-20 theme-bg-overlay">
            <div class="flex items-center gap-3">
                <span class="text-xs theme-text-muted">{previewPosition.index + 1} / {previewPosition.total}</span>
                <span class="text-sm theme-text-secondary truncate">
                    {previewPhoto.path.split("/").pop()}
                </span>
            </div>
            <div class="flex items-center gap-2">
                <button
                    onclick={closePreview}
                    class="p-2 rounded hover:theme-bg-secondary theme-text-muted hover:theme-text-primary"
                >
                    <i class="fa-solid fa-xmark text-sm"></i>
                </button>
            </div>
        </div>

        <!-- Navigation buttons -->
        <button
            onclick={(e) => { e.stopPropagation(); navigatePreview("prev"); }}
            class="absolute left-4 top-1/2 -translate-y-1/2 p-3 rounded-full theme-bg-card hover:theme-bg-secondary theme-text-secondary hover:theme-text-primary z-20"
            title={t.preview.previous}
        >
            <i class="fa-solid fa-chevron-left"></i>
        </button>
        <button
            onclick={(e) => { e.stopPropagation(); navigatePreview("next"); }}
            class="absolute right-72 top-1/2 -translate-y-1/2 p-3 rounded-full theme-bg-card hover:theme-bg-secondary theme-text-secondary hover:theme-text-primary z-20"
            title={t.preview.next}
        >
            <i class="fa-solid fa-chevron-right"></i>
        </button>

        <!-- Image area -->
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
            class="absolute top-12 left-0 right-64 bottom-0"
            onclick={(e) => e.stopPropagation()}
            data-preview-area
        >
            <div
                bind:this={pagerEl}
                class="preview-pager absolute inset-0"
                style="overflow-x: {previewZoomed ? 'hidden' : 'auto'};"
                onscroll={handlePagerScroll}
                data-preview-pager
            >
                {#each stripStops as i (i)}
                    <div class="preview-stop" style="left: {i * 100}%;"></div>
                {/each}
                {#each stripPages as page (page.k)}
                    <div
                        class="preview-page"
                        style="left: {(STRIP_BASE + page.k) * 100}%;"
                        data-preview-page={page.k - stripAt}
                    >
                        {#if page.photo && page.full}
                            <ImagePreview
                                src={page.photo.path}
                                alt={page.photo.path.split("/").pop() || "Preview"}
                                thumbPath={page.photo.thumb_path || ""}
                                {t}
                                mediaType={page.photo.media_type || 'photo'}
                                videoPath={page.photo.video_path || ''}
                                active={page.k === stripAt}
                                onZoomed={(zoomed: boolean) => (previewZoomed = zoomed)}
                            />
                        {:else if page.photo?.thumb_path}
                            <img
                                src={convertFileSrc(page.photo.thumb_path)}
                                alt={page.photo.path.split("/").pop() || "Preview"}
                                class="absolute inset-0 w-full h-full object-contain"
                                draggable="false"
                            />
                        {/if}
                    </div>
                {/each}
            </div>
        </div>

        <!-- Info Panel -->
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
            class="absolute top-0 right-0 bottom-0 w-64 theme-bg-secondary border-l theme-border pt-12 px-4 py-4 overflow-y-auto text-sm"
            onclick={(e) => e.stopPropagation()}
        >
            <div class="space-y-3">
                {#if previewPhoto.metadata.date_taken}
                    <div data-detail-time>
                        <p class="theme-text-muted text-xs">{t.photoInfo.date}</p>
                        <p class="theme-text-primary font-mono text-xs">{previewPhoto.metadata.date_taken}</p>
                    </div>
                {:else if previewPhoto.takenMs}
                    <!-- No capture time in the file: its own date stands in. -->
                    <div data-detail-time>
                        <p class="theme-text-muted text-xs">{t.photoInfo.fileDate}</p>
                        <p class="theme-text-primary font-mono text-xs">{formatFileTime(previewPhoto.takenMs)}</p>
                    </div>
                {/if}

                <div>
                    <p class="theme-text-muted text-xs">{t.photoInfo.dimensions}</p>
                    <p class="theme-text-primary">{previewPhoto.metadata.width} × {previewPhoto.metadata.height}</p>
                </div>

                <div>
                    <p class="theme-text-muted text-xs">{t.photoInfo.fileSize}</p>
                    <p class="theme-text-primary">
                        {formatFileSize(previewPhoto.file_size)}
                        {#if previewPhoto.hasRaw}
                            {#if previewPhoto.rawSize}
                                <span class="text-amber-500 ml-2">+ {formatFileSize(previewPhoto.rawSize)} RAW</span>
                            {/if}
                        {/if}
                    </p>
                </div>

                {#if previewPhoto.metadata.make || previewPhoto.metadata.model}
                    <div>
                        <p class="theme-text-muted text-xs">{t.photoInfo.camera}</p>
                        <p class="theme-text-primary">{previewPhoto.metadata.make || ""} {previewPhoto.metadata.model || ""}</p>
                    </div>
                {/if}

                {#if previewPhoto.metadata.iso || previewPhoto.metadata.f_number || previewPhoto.metadata.exposure_time}
                    <div>
                        <p class="theme-text-muted text-xs">{t.photoInfo.exposure}</p>
                        <p class="theme-text-primary">
                            {#if previewPhoto.metadata.iso}ISO {previewPhoto.metadata.iso}{/if}
                            {#if previewPhoto.metadata.f_number} f/{previewPhoto.metadata.f_number.toFixed(1)}{/if}
                            {#if previewPhoto.metadata.exposure_time} {previewPhoto.metadata.exposure_time}{/if}
                        </p>
                    </div>
                {/if}

                {#if previewPhoto.metadata.lat && previewPhoto.metadata.lon}
                    <div>
                        <p class="theme-text-muted text-xs">{t.photoInfo.gps}</p>
                        <p class="theme-text-primary font-mono text-xs">
                            {previewPhoto.metadata.lat.toFixed(6)}, {previewPhoto.metadata.lon.toFixed(6)}
                        </p>
                    </div>
                {/if}

                {#if previewPhoto.media_type === 'video' && previewPhoto.metadata?.duration_ms}
                    <div>
                        <p class="theme-text-muted text-xs">Duration</p>
                        <p class="theme-text-primary">
                            {(() => {
                                const ms = previewPhoto.metadata.duration_ms;
                                const totalSec = Math.floor(ms / 1000);
                                const min = Math.floor(totalSec / 60);
                                const sec = totalSec % 60;
                                if (min >= 60) {
                                    const hr = Math.floor(min / 60);
                                    const remainMin = min % 60;
                                    return `${hr}:${remainMin.toString().padStart(2, '0')}:${sec.toString().padStart(2, '0')}`;
                                }
                                return `${min}:${sec.toString().padStart(2, '0')}`;
                            })()}
                        </p>
                    </div>
                {/if}

                {#if previewPhoto.metadata?.video_codec}
                    <div>
                        <p class="theme-text-muted text-xs">Codec</p>
                        <p class="theme-text-primary">
                            {previewPhoto.metadata.video_codec}
                            {#if previewPhoto.metadata.audio_codec}
                                / {previewPhoto.metadata.audio_codec}
                            {/if}
                        </p>
                    </div>
                {/if}

                {#if previewPhoto.media_type === 'live_photo'}
                    <div>
                        <p class="theme-text-muted text-xs">Type</p>
                        <p class="text-cyan-400 text-xs font-medium">Live Photo</p>
                        {#if previewPhoto.video_path}
                            <p class="theme-text-muted text-[10px] font-mono break-all mt-0.5">{previewPhoto.video_path}</p>
                        {/if}
                    </div>
                {/if}

                {#if previewPhoto.hasRaw || previewPhoto.isRawOnly}
                    <div>
                        <p class="theme-text-muted text-xs">{t.photoInfo.raw}</p>
                        {#if previewPhoto.hasRaw}
                            <p class="text-amber-500 text-xs font-medium">{t.photoInfo.jpegPlusRaw}</p>
                            <p class="theme-text-muted text-[10px] font-mono break-all mt-0.5">{previewPhoto.rawPath}</p>
                        {:else if previewPhoto.isRawOnly}
                            <p class="text-rose-500 text-xs font-medium">{t.photoInfo.rawOnly}</p>
                        {/if}
                    </div>
                {/if}

                <div>
                    <p class="theme-text-muted text-xs">{t.photoInfo.path}</p>
                    <p class="theme-text-primary text-xs font-mono break-all">{previewPhoto.path}</p>
                    <button
                        onclick={(e) => handleShowInFinder(previewPhoto!.path, e)}
                        data-show-in-finder
                        class="mt-2 w-full px-3 py-2 rounded text-xs theme-bg-tertiary theme-text-secondary hover:theme-text-primary flex items-center gap-2"
                    >
                        <i class="fa-solid fa-folder-open"></i>
                        {t.preview.showInFinder}
                    </button>
                </div>

                <!-- Delete actions -->
                <div class="pt-3 mt-3 border-t theme-border">
                    <p class="theme-text-muted text-xs mb-2">{t.delete.title}</p>
                    <div class="flex flex-col gap-2">
                        <button
                            onclick={(e) => { e.stopPropagation(); openDeleteConfirm('app'); }}
                            disabled={isDeleting}
                            class="w-full px-3 py-2 rounded text-xs bg-amber-600/20 text-amber-400 hover:bg-amber-600/30 disabled:opacity-50 flex items-center gap-2"
                        >
                            <i class="fa-solid fa-database"></i>
                            {t.delete.removeFromLibrary}
                        </button>
                    </div>
                </div>
            </div>
        </div>
    </div>

    <!-- Delete Confirmation Modal -->
    {#if deleteConfirmOpen}
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
            class="fixed inset-0 z-[100] bg-black/60 backdrop-blur-sm flex items-center justify-center"
            onclick={() => deleteConfirmOpen = false}
        >
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            <div
                class="theme-bg-secondary border theme-border rounded-xl shadow-2xl w-full max-w-md p-6"
                onclick={(e) => e.stopPropagation()}
            >
                <div class="flex items-center gap-3 mb-4">
                    <div class="w-10 h-10 rounded-full {deleteMode === 'complete' ? 'bg-red-500/20' : 'bg-amber-500/20'} flex items-center justify-center">
                        <i class="fa-solid {deleteMode === 'complete' ? 'fa-trash text-red-400' : 'fa-database text-amber-400'}"></i>
                    </div>
                    <div>
                        <h3 class="text-lg font-medium theme-text-primary">
                            {deleteMode === 'complete' ? t.delete.deleteOriginal : t.delete.removeFromLibrary}
                        </h3>
                        <p class="text-sm theme-text-muted">
                            {deleteMode === 'complete' ? t.delete.deleteOriginalDesc : t.delete.removeFromLibraryDesc}
                        </p>
                    </div>
                </div>

                <div class="theme-bg-primary rounded-lg p-3 mb-4">
                    <p class="text-xs theme-text-muted mb-1">{t.delete.file}</p>
                    <p class="text-sm theme-text-primary font-mono break-all">{previewPhoto.path.split('/').pop()}</p>
                    {#if previewPhoto.hasRaw && deleteMode === 'complete'}
                        <div class="mt-3 pt-3 border-t border-white/10">
                            <p class="text-xs theme-text-muted mb-2">{t.delete.associatedRaw}:</p>
                            <p class="text-xs text-amber-400 font-mono break-all mb-3">{previewPhoto.rawPath?.split('/').pop()}</p>
                            <label class="flex items-center gap-2 cursor-pointer">
                                <input
                                    type="checkbox"
                                    bind:checked={deleteIncludeRaw}
                                    class="w-4 h-4 rounded border-2 border-amber-500 bg-transparent checked:bg-amber-500"
                                />
                                <span class="text-sm text-amber-400">{t.delete.alsoDeleteRaw}</span>
                            </label>
                        </div>
                    {:else if previewPhoto.hasRaw}
                        <p class="text-xs text-amber-400 mt-1">+ {t.delete.associatedRaw}</p>
                    {/if}
                </div>

                {#if deleteMode === 'complete'}
                    <div class="bg-red-500/10 border border-red-500/30 rounded-lg p-3 mb-4">
                        <p class="text-xs text-red-400">
                            <i class="fa-solid fa-triangle-exclamation mr-1"></i>
                            {t.delete.warningPermanent}
                        </p>
                    </div>
                {/if}

                <div class="flex gap-3">
                    <button
                        onclick={() => deleteConfirmOpen = false}
                        disabled={isDeleting}
                        class="flex-1 px-4 py-2 rounded-lg theme-bg-tertiary theme-text-secondary hover:theme-text-primary disabled:opacity-50"
                    >
                        {t.common.cancel}
                    </button>
                    <button
                        onclick={() => handleDeletePhoto(previewPhoto!, deleteMode, deleteIncludeRaw)}
                        disabled={isDeleting}
                        class="flex-1 px-4 py-2 rounded-lg {deleteMode === 'complete' ? 'bg-red-600 hover:bg-red-700' : 'bg-amber-600 hover:bg-amber-700'} text-white disabled:opacity-50 flex items-center justify-center gap-2"
                    >
                        {#if isDeleting}
                            <i class="fa-solid fa-spinner fa-spin"></i>
                        {/if}
                        {deleteMode === 'complete' ? t.common.delete : t.common.remove}
                    </button>
                </div>
            </div>
        </div>
    {/if}
{/if}

<style>
    :global(body) {
        overflow: hidden;
    }
    /* The large view's sideways scroller: one stop per photo. */
    .preview-pager {
        overflow-y: hidden;
        scroll-snap-type: x mandatory;
        scrollbar-width: none;
    }
    .preview-pager::-webkit-scrollbar {
        display: none;
    }
    .preview-pager > :global(.preview-page) {
        position: absolute;
        top: 0;
        width: 100%;
        height: 100%;
        overflow: hidden;
    }
    .preview-pager > :global(.preview-stop) {
        position: absolute;
        top: 0;
        width: 100%;
        height: 1px;
        scroll-snap-align: start;
        scroll-snap-stop: always;
    }
</style>
