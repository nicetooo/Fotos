<script lang="ts">
    import { onMount } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { appDataDir, join } from "@tauri-apps/api/path";

    // Platform service
    import {
        setPlatformService,
        isIOSPlatform,
        isAndroidPlatform,
        nativeViewerOf,
        type PlatformService,
        type IOSPlatformService,
        type AndroidPlatformService,
        type ImportProgress,
    } from "../lib/platform";

    // Shared utilities and types
    import {
        type Theme,
        type ImportStatus,
    } from "../shared";
    import { queryAlbum, queryAlbumIn, photoCount, albumItemToPhoto, type AlbumItem, type Bbox, type MediaFilter } from "../shared/query";

    // i18n
    import { createI18nState, type Locale } from "../shared/i18n";

    // Components
    import Settings from "./components/Settings.svelte";
    import PhotoViewer from "./components/PhotoViewer.svelte";
    import AlbumGrid from "./components/AlbumGrid.svelte";
    import MapView from "./components/MapView.svelte";
    import MediaFilterButton from "./components/MediaFilterButton.svelte";

    // Platform service received as prop from main.ts
    let { platformService } = $props<{ platformService: PlatformService }>();

    // Set context synchronously during component initialization
    setPlatformService(platformService);

    // i18n state
    const i18n = createI18nState();
    let locale = $derived(i18n.locale);
    let t = $derived(i18n.t);

    // iOS-specific service (set during initialization)
    let iosPlatformService: IOSPlatformService | null = $state(null);

    // Android-specific service (set during initialization)
    let androidPlatformService: AndroidPlatformService | null = $state(null);

    let version = $state("...");
    let showSettings = $state(false);
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

    // The UI holds no library data. The map queries what is in view; this key
    // tells it the library changed and it should ask again.
    let refreshKey = $state(0);
    let libraryCount = $state(0);

    let lastProgressRefresh = 0;
    function bumpRefresh() {
        refreshKey++;
        photoCount().then((c) => { libraryCount = c; }).catch(() => {});
    }

    /** Self-test: each media filter narrows the map, and together they add up. */
    async function agentFilterSteps() {
        const { geoSummary } = await import("../shared/query");
        const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
        const counts: Record<string, number> = {};
        const drawn: Record<string, number> = {};
        for (const kind of ["all", "photo", "video", "live_photo", "all"] as MediaFilter[]) {
            await setMediaFilter(kind);
            await sleep(700);
            counts[kind] = (await geoSummary()).total;
            drawn[kind] = (window as any).__footosMapStats?.().total ?? -1;
        }
        const button = document.querySelector("[data-media-filter]");
        const parts = counts.photo + counts.video + counts.live_photo;
        return [{
            name: "mobile:media-filter",
            ok: !!button && counts.photo > 0 && counts.video > 0 && counts.live_photo > 0 && parts === counts.all
                && mediaFilter === "all",
            detail: { ...counts, mapTotals: drawn },
        }];
    }

    // Photos / videos / Live Photos. Rust narrows the query index, so the
    // map, the timeline and the album all follow without knowing about it.
    let mediaFilter = $state<MediaFilter>("all");
    async function setMediaFilter(next: MediaFilter) {
        try {
            await invoke("set_media_filter", { filter: next === "all" ? null : next });
            mediaFilter = next;
            albumReturn = null;
            bumpRefresh();
        } catch (e) {
            console.error("[filter] set_media_filter failed", e);
        }
    }

    let dbPath = $state("");
    let thumbDir = $state("");
    // Viewer: a window of the time-ordered album around the current photo.
    const ALBUM_RADIUS = 300;
    const ALBUM_EDGE = 60;
    let previewPhoto = $state.raw<any | null>(null);
    let viewerPhotos = $state.raw<any[]>([]);
    let viewerStart = 0;
    let viewerTotal = 0;
    let albumRefetchTimer: ReturnType<typeof setTimeout> | null = null;

    // iOS specific state
    let hasFullAccess = $state(false);

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

    onMount(async () => {
        // Back button support: close modals on browser history navigation
        window.addEventListener('popstate', handlePopState);
        // The native viewer reports where it was closed.
        // A choice made in the system context menu of an album cell.
        (window as any).__handleAlbumMenu = (action: string, index: number) => {
            if (action === "locate") locateAlbumItem(index);
            else if (action === "open") openAlbumItem(index);
        };
        const onNativeViewerClosed = (index: number) => albumGridRef?.reveal(index);
        (window as any).__handleIOSViewerClosed = onNativeViewerClosed;
        (window as any).__handleNativeViewerClosed = onNativeViewerClosed;

        try {
            // Check if iOS platform for specific features
            if (isIOSPlatform(platformService)) {
                iosPlatformService = platformService;
            }

            // Check if Android platform for specific features
            if (isAndroidPlatform(platformService)) {
                androidPlatformService = platformService;
            }

            version = await invoke("get_core_version");
            const appData = await appDataDir();
            dbPath = await join(appData, "footos.db");
            thumbDir = await join(appData, "thumbnails");

            const boot: { t0: number; marks: Record<string, number> } = {
                t0: performance.now(),
                marks: {},
            };
            const mark = (key: string) => {
                boot.marks[key] = Math.round(performance.now() - boot.t0);
                console.log(`[boot] ${key} +${boot.marks[key]}ms`);
            };
            (window as any).__footosBoot = boot;

            // Initialize shared DB connection (single connection for all commands)
            await invoke("init_app", { dbPath, thumbDir });
            mark("init_app");

            // Initialize platform service
            await platformService.initialize(dbPath, thumbDir);
            mark("platform");

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
                // At most once a second: batches arrive at any count.
                const now = performance.now();
                if (now - lastProgressRefresh >= 1000) {
                    lastProgressRefresh = now;
                    bumpRefresh();
                }
            });

            iosPlatformService?.onLibraryChanged(() => bumpRefresh());

            platformService.onImportComplete((result) => {
                console.log("[App] Import complete callback triggered:", result);
                bumpRefresh();
                refreshKnownIosIds();
            });

            // Listen for reload-photos event (e.g., after clear cache)
            await listen("reload-photos", () => {
                bumpRefresh();
                // The library was wiped. What the app shows is a cache of the
                // photo library, so fill it again right away where access allows.
                iosPlatformService?.resetAfterClear();
                if (iosPlatformService) {
                    void iosPlatformService.syncPhotosIfFullAccess();
                } else {
                    void androidPlatformService?.syncPhotosIfHasAccess();
                }
            });

            // DB is ready: let the map run its first viewport query.
            bumpRefresh();
            mark("first-query");

            // Agent self-test (scripts/agent-test): seeded data only, so no
            // PhotoKit / MediaStore sync and no permission dialogs over the UI.
            const agentScenario = (await invoke("agent_test_enabled")) as string | null;

            // iOS: incremental PhotoKit sync after first paint. Known IDs come
            // from a cheap path-only query so we do not miss non-geotagged rows.
            if (iosPlatformService && !agentScenario) {
                iosPlatformService.onPermissionChange((fullAccess) => {
                    hasFullAccess = fullAccess;
                });
                setTimeout(async () => {
                    try {
                        const ids = (await invoke("list_ios_known_ids")) as string[];
                        iosPlatformService?.setKnownPhotoIdentifiers(ids);
                        const status = await platformService.checkPermissionStatus();
                        hasFullAccess = status === "granted";
                        await iosPlatformService?.syncPhotosIfFullAccess();
                        mark("photosync");
                    } catch (e) {
                        console.warn("[App] Deferred PhotoKit sync failed", e);
                    }
                }, 1500);
            }

            // Android: Auto-sync photos from MediaStore if has access
            if (androidPlatformService && !agentScenario) {
                androidPlatformService.onPermissionChange((hasAccess: boolean) => {
                    hasFullAccess = hasAccess;
                });

                androidPlatformService.syncPhotosIfHasAccess();
            }
            // Photos taken while the app was in the background: pick them up
            // when it comes back, since there is no button to ask for it.
            if (!agentScenario) {
                let lastForegroundSync = Date.now();
                document.addEventListener("visibilitychange", () => {
                    if (document.visibilityState !== "visible" || !hasFullAccess || isScanning) return;
                    if (Date.now() - lastForegroundSync < 30_000) return;
                    lastForegroundSync = Date.now();
                    if (iosPlatformService) {
                        void refreshKnownIosIds().then(() => iosPlatformService?.syncPhotosIfFullAccess());
                    } else {
                        void androidPlatformService?.syncPhotosIfHasAccess();
                    }
                });
            }

            (window as any).__footosOpenPreview = async () => {
                const w = await queryAlbum(null, 0);
                if (w.items.length === 0) return false;
                const opened = await openPreview(w.items[0]);
                return { ok: true, path: w.items[0].path, ...opened };
            };
            (window as any).__footosClosePreview = () => closePreview();

            // Agent self-test: only when launched with FOOTOS_AGENT_TEST (scripts/agent-test).
            const scenario = agentScenario;
            if (scenario) {
                const { runAgentTest } = await import("../shared/agent-test");
                void runAgentTest(scenario, {
                    getMap: () => (window as any).__footosMap ?? null,
                    getMapStats: () => (window as any).__footosMapStats?.() ?? {},
                    openPreview: () => (window as any).__footosOpenPreview(),
                    closePreview: () => closePreview(),
                    bootMarks: () => boot.marks,
                    extraSteps: agentFilterSteps,
                });
            }
        } catch (e) {
            error = t.errors.initFailed + ": " + e;
        }

        return () => {
            delete (window as any).__footosOpenPreview;
            delete (window as any).__footosClosePreview;
            window.removeEventListener('popstate', handlePopState);
            platformService.cleanup();
        };
    });

    // A sync with nothing new finishes at once and shows nothing; keep the
    // icon turning for a moment so the tap visibly did something.
    let refreshing = $state(false);

    /** Full access: look for photos added since the last sync. */
    async function handleRefresh() {
        if (refreshing || isScanning) return;
        refreshing = true;
        const minimum = new Promise((r) => setTimeout(r, 900));
        try {
            if (iosPlatformService) {
                await refreshKnownIosIds();
                await iosPlatformService.syncPhotosIfFullAccess();
            } else {
                await androidPlatformService?.syncPhotosIfHasAccess();
            }
        } catch (e) {
            console.warn("[App] Refresh failed", e);
        }
        await minimum;
        refreshing = false;
    }

    async function refreshKnownIosIds() {
        if (!iosPlatformService) return;
        try {
            const ids = (await invoke("list_ios_known_ids")) as string[];
            iosPlatformService.setKnownPhotoIdentifiers(ids);
        } catch (e) {
            console.warn("[App] list_ios_known_ids failed", e);
        }
    }

    async function handleImport() {
        try {
            error = "";
            importStatus = { success: 0, failure: 0, duplicates: 0, current: 0, total: 0, lastPath: "" };
            await refreshKnownIosIds();
            iosPlatformService?.setAccessDeniedTexts({
                title: t.permissions.deniedAccessTitle,
                message: t.permissions.deniedAccessMessage,
                settings: t.permissions.goToSettings,
                cancel: t.common.cancel,
            });
            await platformService.requestImport();
        } catch (e) {
            error = String(e);
        }
    }

    async function loadAlbumWindow(centerId: number | null) {
        const w = await queryAlbum(centerId, ALBUM_RADIUS);
        viewerStart = w.start;
        viewerTotal = w.total;
        viewerPhotos = w.items.map(albumItemToPhoto);
        return viewerPhotos;
    }

    // Native viewer gets a bigger window: it cannot ask for more once open.
    const NATIVE_ALBUM_RADIUS = 500;

    async function openPreview(item: { id: number; path: string }) {
        try {
            const nativeViewer = nativeViewerOf(platformService);
            if (nativeViewer) {
                const w = await queryAlbum(item.id, NATIVE_ALBUM_RADIUS);
                const index = w.items.findIndex((p) => p.id === item.id);
                const opened = nativeViewer.openNativeViewer(
                    w.items.map((p) => ({
                        path: p.path,
                        thumb: p.thumb,
                        media_type: p.media_type,
                        date_taken: p.date_taken,
                    })),
                    Math.max(0, index),
                );
                if (opened) return { native: true, count: w.items.length };
            }
            const list = await loadAlbumWindow(item.id);
            const photo = list.find((p) => p.path === item.path);
            if (!photo) return;
            previewPhoto = photo;
            history.pushState({ modal: "preview" }, "");
            return { native: false, count: list.length };
        } catch (e) {
            console.error("[App] Failed to open preview", e);
        }
    }

    // What the map shows can be a lot; the grid holds the newest this many.
    const VIEWPORT_ALBUM_LIMIT = 1000;

    // The album grid (what is on the map), shown before any single photo.
    let albumGrid = $state.raw<{ items: AlbumItem[]; total: number; initialIndex?: number } | null>(null);
    // The list the user left to look at one photo on the map, and which photo.
    // The map button that would reopen "what is in view" now shows only the
    // few photos around that spot, so this is the way back to the same list.
    let albumReturn = $state.raw<{ items: AlbumItem[]; total: number; index: number } | null>(null);
    let albumGridRef = $state<{ reveal: (index: number) => void } | null>(null);

    /** Open the album of what is on the map now as a grid. */
    async function openViewportAlbum(bbox: Bbox, startMs: number | null, endMs: number | null) {
        try {
            const w = await queryAlbumIn(bbox, startMs, endMs, VIEWPORT_ALBUM_LIMIT);
            if (w.items.length === 0) return;
            // A new list replaces the one to go back to.
            albumReturn = null;
            showAlbum({ items: w.items, total: w.total });
        } catch (e) {
            console.error("[App] Failed to open viewport album", e);
        }
    }

    function showAlbum(grid: { items: AlbumItem[]; total: number; initialIndex?: number }) {
        albumGrid = grid;
        if (platformService.capabilities.nativeContextMenu) {
            iosPlatformService?.setAlbumMenuContext(
                grid.items.map((p) => ({ path: p.path, thumb: p.thumb, media_type: p.media_type, date_taken: p.date_taken })),
                { copy: t.map.copyPhoto, share: t.map.sharePhoto, locate: t.map.locateOnMap },
            );
        }
        history.pushState({ modal: "album" }, "");
    }

    /** Back to the list the user located a photo from, at that photo. */
    function returnToAlbum() {
        const saved = albumReturn;
        if (!saved) return;
        albumReturn = null;
        showAlbum({ items: saved.items, total: saved.total, initialIndex: saved.index });
    }

    /** A cell of the grid was tapped: open the viewer on it, over the grid. */
    function openAlbumItem(index: number) {
        const grid = albumGrid;
        if (!grid) return;
        const nativeViewer = nativeViewerOf(platformService);
        if (nativeViewer) {
            const opened = nativeViewer.openNativeViewer(
                grid.items.map((p) => ({
                    path: p.path,
                    thumb: p.thumb,
                    media_type: p.media_type,
                    date_taken: p.date_taken,
                })),
                index,
            );
            if (opened) return;
        }
        // A fixed list: start/total say there is nothing more to fetch, so
        // browsing never swaps it for a window of the whole album.
        viewerPhotos = grid.items.map(albumItemToPhoto);
        viewerStart = 0;
        viewerTotal = viewerPhotos.length;
        previewPhoto = viewerPhotos[index];
        history.pushState({ modal: "preview" }, "");
    }

    // Where the map should go next (an item picked in the grid).
    let mapFocus = $state.raw<{ lon: number; lat: number; seq: number } | null>(null);

    /** Leave the grid and show this item's place on the map. */
    function locateAlbumItem(index: number) {
        const grid = albumGrid;
        const item = grid?.items[index];
        if (!grid || !item) return;
        albumReturn = { items: grid.items, total: grid.total, index };
        closeAlbum();
        mapFocus = { lon: item.lon, lat: item.lat, seq: (mapFocus?.seq ?? 0) + 1 };
    }

    function closeAlbum() {
        if (!albumGrid) return;
        albumGrid = null;
        isClosingManually = true;
        history.back();
    }

    /** Back in the grid, keep the photo that was on display in sight. */
    function revealInAlbum() {
        const grid = albumGrid;
        const photo = previewPhoto;
        if (!grid || !photo) return;
        const index = grid.items.findIndex((p) => p.path === photo.path);
        if (index >= 0) albumGridRef?.reveal(index);
    }

    /** Near the edge of the loaded window: fetch the next one around the current photo. */
    function handleViewerNavigate(photo: any) {
        if (!photo) return;
        previewPhoto = photo;
        const idx = viewerPhotos.indexOf(photo);
        if (idx < 0) return;
        const moreBefore = viewerStart > 0 && idx < ALBUM_EDGE;
        const moreAfter = viewerStart + viewerPhotos.length < viewerTotal
            && idx >= viewerPhotos.length - ALBUM_EDGE;
        if (albumRefetchTimer) clearTimeout(albumRefetchTimer);
        if (!moreBefore && !moreAfter) return;
        // Wait for the swipe / scrub to settle before swapping the list.
        albumRefetchTimer = setTimeout(() => {
            albumRefetchTimer = null;
            const current = previewPhoto;
            if (!current) return;
            loadAlbumWindow(current.id.id)
                .then((list) => {
                    const same = list.find((p) => p.path === current.path);
                    if (same && previewPhoto === current) previewPhoto = same;
                })
                .catch((e) => console.warn("[App] album window refetch failed", e));
        }, 500);
    }

    async function resolveOriginalForViewer(photo: any): Promise<string | null> {
        if (!iosPlatformService || !photo?.path?.startsWith("ios-photo://")) {
            return null;
        }
        try {
            return await iosPlatformService.ensureOriginal(photo.path);
        } catch (e) {
            console.warn("[App] On-demand original export failed, showing thumbnail", e);
            return null;
        }
    }

    async function resolveStillForViewer(photo: any): Promise<string | null> {
        if (!iosPlatformService || !photo?.path?.startsWith("ios-photo://")) {
            return null;
        }
        try {
            return await iosPlatformService.ensureViewerStill(photo.path);
        } catch (e) {
            console.warn("[App] Viewer still export failed", e);
            return null;
        }
    }

    async function resolveLiveVideoForViewer(photo: any): Promise<string | null> {
        if (photo?.video_path) {
            return photo.video_path;
        }
        if (!iosPlatformService || !photo?.path?.startsWith("ios-photo://")) {
            return null;
        }
        try {
            return await iosPlatformService.ensureLiveVideo(photo.path);
        } catch (e) {
            console.warn("[App] Live video export failed", e);
            return null;
        }
    }

    function closePreview() {
        if (!previewPhoto) return;
        revealInAlbum();
        previewPhoto = null;
        viewerPhotos = [];
        isClosingManually = true;
        history.back();
    }

    // Back button support: manage browser history for modal states
    // When a modal opens, we pushState. When user presses hardware back button,
    // the browser navigates back (popstate), and we close the topmost modal.
    // When user closes manually (X button/backdrop), we also go back in history.
    let isClosingManually = false;

    function openSettings() {
        showSettings = true;
        history.pushState({ modal: 'settings' }, '');
    }

    function closeSettings() {
        if (!showSettings) return;
        showSettings = false;
        isClosingManually = true;
        history.back();
    }

    function handlePopState() {
        if (isClosingManually) {
            isClosingManually = false;
            return;
        }
        // Hardware/system back button pressed: close topmost modal
        if (previewPhoto) {
            revealInAlbum();
            previewPhoto = null;
            viewerPhotos = [];
        } else if (albumGrid) {
            albumGrid = null;
        } else if (showSettings) {
            showSettings = false;
        }
    }
</script>

<main class="fixed inset-0 flex flex-col theme-bg-primary theme-text-primary overflow-hidden">
    <!-- Fullscreen Map -->
    <div class="flex-1 relative">
        <MapView {refreshKey} {libraryCount} onOpenPreview={openPreview} onOpenAlbum={openViewportAlbum} focusRequest={mapFocus} theme={effectiveTheme} {t} {locale} isSyncing={isScanning} />

        <!-- Floating action buttons (top-left, vertical) -->
        <div data-map-overlay class="absolute left-4 z-[1001] flex flex-col gap-2" style="top: calc(max(47px, env(safe-area-inset-top, 0px)) + 12px)">
            <!-- Add photos while access is partial; with access to the whole
                 library there is nothing to pick, so the same button re-syncs. -->
            <button
                onclick={() => (hasFullAccess ? handleRefresh() : handleImport())}
                disabled={isScanning || refreshing}
                data-import-button
                class="w-12 h-12 rounded-full theme-bg-card border theme-border theme-text-secondary hover:theme-text-primary disabled:opacity-50 flex items-center justify-center transition-all"
                title={hasFullAccess ? t.import.refresh : t.import.importPhotos}
                aria-label={hasFullAccess ? t.import.refresh : t.import.importPhotos}
            >
                {#if isScanning || refreshing}
                    <i class="fa-solid {hasFullAccess ? 'fa-rotate' : 'fa-spinner'} fa-spin text-lg"></i>
                {:else}
                    <i class="fa-solid {hasFullAccess ? 'fa-rotate' : 'fa-plus'} text-lg"></i>
                {/if}
            </button>

            <!-- Settings button -->
            <button
                onclick={openSettings}
                class="w-12 h-12 rounded-full theme-bg-card border theme-border theme-text-secondary hover:theme-text-primary flex items-center justify-center transition-all"
                title={t.common.settings}
            >
                <i class="fa-solid fa-gear text-lg"></i>
            </button>

            <MediaFilterButton value={mediaFilter} onChange={setMediaFilter} {t} />

            <!-- Back to the list a photo was located from -->
            {#if albumReturn && !albumGrid}
                <button
                    onclick={returnToAlbum}
                    data-album-return
                    class="h-12 pl-3 pr-4 rounded-full bg-[var(--accent)] text-black font-medium text-sm flex items-center gap-2 transition-all whitespace-nowrap self-start"
                    aria-label={t.map.backToList}
                >
                    <i class="fa-solid fa-chevron-left"></i>
                    <span>{t.map.backToList}</span>
                </button>
            {/if}
        </div>

        <!-- Import progress indicator -->
        {#if isScanning}
            <div data-map-overlay class="absolute left-20 z-[1001] px-3 py-2 rounded-full bg-black/70 backdrop-blur-sm border border-white/20 text-sm text-white/80 flex items-center gap-2" style="top: calc(max(47px, env(safe-area-inset-top, 0px)) + 12px)">
                <i class="fa-solid fa-spinner fa-spin text-xs"></i>
                {#if importStatus.total > 0}
                    <span>{importStatus.current} / {importStatus.total}</span>
                {:else}
                    <span>{t.common.syncing}...</span>
                {/if}
            </div>
        {/if}

        <!-- Error message -->
        {#if error}
            <div class="absolute left-1/2 -translate-x-1/2 z-[1001] px-4 py-2 rounded-lg bg-red-900/80 backdrop-blur-sm border border-red-700/50 text-red-200 text-sm max-w-[90%]" style="top: calc(max(47px, env(safe-area-inset-top, 0px)) + 12px)">
                {error}
            </div>
        {/if}
    </div>
</main>

<!-- Settings Modal -->
{#if showSettings}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div
        data-map-blocker
        class="fixed inset-0 z-[2000] bg-black/50 flex items-end justify-center"
        onclick={closeSettings}
    >
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
            class="theme-bg-secondary border-t theme-border rounded-t-2xl shadow-2xl w-full max-h-[80vh] overflow-hidden"
            onclick={(e) => e.stopPropagation()}
        >
            <div class="flex items-center justify-between px-6 py-4 border-b theme-border">
                <h2 class="text-lg font-medium theme-text-primary">{t.settings.title}</h2>
                <button
                    onclick={closeSettings}
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

{#if albumGrid}
    <AlbumGrid
        bind:this={albumGridRef}
        items={albumGrid.items}
        total={albumGrid.total}
        initialIndex={albumGrid.initialIndex ?? null}
        onOpen={openAlbumItem}
        onLocate={locateAlbumItem}
        nativeMenu={platformService.capabilities.nativeContextMenu}
        requestThumbs={platformService.requestGridThumbs ? (paths, onReady) => platformService.requestGridThumbs!(paths, onReady) : undefined}
        onClose={closeAlbum}
        {t}
        {locale}
    />
{/if}

{#if previewPhoto}
    <PhotoViewer
        photos={viewerPhotos}
        currentPath={previewPhoto.path}
        {t}
        {locale}
        onClose={closePreview}
        onNavigate={handleViewerNavigate}
        resolveOriginal={resolveOriginalForViewer}
        resolveStill={resolveStillForViewer}
        resolveLiveVideo={resolveLiveVideoForViewer}
    />
{/if}

<style>
    :global(body) {
        overflow: hidden;
    }
</style>
