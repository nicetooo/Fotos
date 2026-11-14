<script lang="ts">
    import { onMount } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
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
        type SortBy,
        type SortOrder,
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
        type MediaFilter,
    } from "../shared/query";
    import { LibraryPager, type LibraryCounts } from "./library-pager";

    // i18n
    import { createI18nState, type Locale } from "../shared/i18n";

    // Components
    import Settings from "./components/Settings.svelte";
    import ImagePreview from "./components/ImagePreview.svelte";
    import MapView from "./components/MapView.svelte";
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
    let libraryCounts = $state<LibraryCounts>({ total: 0, groupedTotal: 0, filesTotal: 0 });
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

    // Sort state with localStorage persistence
    const SORT_BY_KEY = "footos-sort-by";
    const SORT_ORDER_KEY = "footos-sort-order";
    let sortBy = $state<SortBy>((() => {
        if (typeof localStorage !== "undefined") {
            const saved = localStorage.getItem(SORT_BY_KEY);
            if (saved === "name" || saved === "date" || saved === "size" || saved === "dimensions") {
                return saved;
            }
        }
        return "date";
    })());
    let sortOrder = $state<SortOrder>((() => {
        if (typeof localStorage !== "undefined") {
            const saved = localStorage.getItem(SORT_ORDER_KEY);
            if (saved === "asc" || saved === "desc") {
                return saved;
            }
        }
        return "desc";
    })());

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

    // Save sort options and filter when changed
    $effect(() => {
        if (typeof localStorage !== "undefined") {
            localStorage.setItem(SORT_BY_KEY, sortBy);
        }
    });
    $effect(() => {
        if (typeof localStorage !== "undefined") {
            localStorage.setItem(SORT_ORDER_KEY, sortOrder);
        }
    });
    $effect(() => {
        if (typeof localStorage !== "undefined") {
            localStorage.setItem(MEDIA_FILTER_KEY, mediaFilter);
        }
    });

    let libraryImportMenuOpen = $state(false);
    let floatingImportMenuOpen = $state(false);
    let sortMenuOpen = $state(false);

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
        pager.setParams(sortBy, sortOrder === "desc", mediaFilter);
    });

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

    async function navigatePreview(direction: "prev" | "next") {
        const src = previewSource;
        if (!previewPhoto || !src) return;
        const step = direction === "prev" ? -1 : 1;
        const seq = ++previewSeq;

        if (src.kind === 'library') {
            const total = libraryCounts.total;
            if (total === 0) return;
            const index = (src.index + step + total) % total;
            const item = await pager.item(index);
            if (seq !== previewSeq || !item) return;
            previewPhoto = item;
            previewSource = { kind: 'library', index };
            return;
        }

        let { window, pos } = src;
        const shownId = previewPhoto.id.id;
        // A RAW next to its JPEG is two rows in the album but one photo: step over it.
        for (;;) {
            pos += step;
            const item = window.items[pos];
            if (!item) return;
            const detail = await photoDetail(item.id).catch(() => null);
            if (seq !== previewSeq) return;
            const photo = detail ?? albumItemToPhoto(item);
            if (photo.id.id === shownId) continue;
            previewPhoto = photo;
            previewSource = { kind: 'album', window, pos };
            break;
        }

        // Near the edge of the window: fetch a new one centred on this photo.
        const end = window.start + window.items.length;
        const nearStart = pos < ALBUM_EDGE && window.start > 0;
        const nearEnd = pos >= window.items.length - ALBUM_EDGE && end < window.total;
        if (nearStart || nearEnd) {
            const centerId = window.items[pos].id;
            const next = await queryAlbum(centerId, ALBUM_RADIUS).catch(() => null);
            if (!next || seq !== previewSeq) return;
            const nextPos = next.items.findIndex((p) => p.id === centerId);
            if (nextPos >= 0) previewSource = { kind: 'album', window: next, pos: nextPos };
        }
    }

    /** Desktop-only checks for the agent self-test: timeline, drawer paging, viewer. */
    async function agentDesktopSteps() {
        const out: { name: string; ok: boolean; detail?: unknown }[] = [];
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
            // Saved drawer settings must not decide what the check sees.
            const savedFilter = mediaFilter;
            mediaFilter = "all";
            const t0 = performance.now();
            showLibrary = true;
            const ready = await until(() => libraryLoaded && libraryCounts.total > 0);
            const firstMs = Math.round(performance.now() - t0);
            await sleep(200);
            const cells = document.querySelectorAll("button[title] img").length;
            const total = libraryCounts.total;
            const t1 = performance.now();
            const last = await pager.item(total - 1);
            const farMs = Math.round(performance.now() - t1);
            const before = sortBy;
            const t2 = performance.now();
            sortBy = "name";
            await sleep(0);
            const a = await pager.item(0);
            const b = await pager.item(1);
            const sortMs = Math.round(performance.now() - t2);
            const name = (p?: PhotoInfo) => (p?.path.split("/").pop() || "").toLowerCase();
            const sorted = sortOrder === "desc" ? name(a) >= name(b) : name(a) <= name(b);
            sortBy = before;
            await sleep(0);
            await pager.item(0);
            restoreFilter = () => { mediaFilter = savedFilter; };
            out.push({
                name: "desktop:library",
                ok: ready && cells > 0 && !!last && sorted && total === libraryCounts.groupedTotal,
                detail: { total, files: libraryCounts.filesTotal, firstPageMs: firstMs, farPageMs: farMs, resortMs: sortMs, cellsRendered: cells },
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

        showLibrary = false;
        restoreFilter();

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
        <MapView {refreshKey} {libraryCount} onOpenPreview={openFromMap} theme={effectiveTheme} {t} {locale} />

        <!-- Floating action buttons (top-left, vertical) -->
        <div class="absolute top-4 left-4 z-[1001] flex flex-col gap-2">
            <!-- Import button with dropdown -->
            <div class="relative">
                <button
                    onclick={() => floatingImportMenuOpen = !floatingImportMenuOpen}
                    disabled={isScanning}
                    class="w-10 h-10 rounded-full theme-bg-card backdrop-blur-sm border theme-border theme-text-secondary hover:theme-text-primary disabled:opacity-50 flex items-center justify-center shadow-lg transition-all"
                    title={t.import.importPhotos}
                >
                    <i class="fa-solid {isScanning ? 'fa-spinner fa-spin' : 'fa-plus'}"></i>
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

            <!-- Library button -->
            <button
                onclick={() => handleLibrary()}
                class="w-10 h-10 rounded-full theme-bg-card backdrop-blur-sm border theme-border theme-text-secondary hover:theme-text-primary flex items-center justify-center shadow-lg transition-all {showLibrary ? 'theme-bg-secondary' : ''}"
                title={t.nav.library}
            >
                <i class="fa-solid fa-images"></i>
            </button>

            <!-- Settings button -->
            <button
                onclick={() => showSettings = true}
                class="w-10 h-10 rounded-full theme-bg-card backdrop-blur-sm border theme-border theme-text-secondary hover:theme-text-primary flex items-center justify-center shadow-lg transition-all"
                title={t.common.settings}
            >
                <i class="fa-solid fa-gear"></i>
            </button>
        </div>

        <!-- Library Drawer (right side) -->
        {#if showLibrary}
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            <div
                class="fixed inset-0 bg-black/30 backdrop-blur-sm z-[1002]"
                onclick={() => showLibrary = false}
            ></div>
            <div class="absolute top-0 right-0 bottom-0 w-1/2 min-w-[500px] theme-bg-overlay backdrop-blur-md border-l theme-border z-[1003] flex flex-col">
                <!-- Header -->
                <div class="flex items-center justify-between px-4 py-3 border-b theme-border">
                    <div class="flex items-center gap-2">
                        <i class="fa-solid fa-images theme-text-muted text-sm"></i>
                        <span class="theme-text-primary font-medium">
                            {libraryCounts.total} {t.nav.photos}
                            {#if mediaFilter !== 'all' && libraryCounts.total !== libraryCounts.groupedTotal}
                                <span class="theme-text-muted text-xs">/ {libraryCounts.groupedTotal}</span>
                            {:else if libraryCounts.total !== libraryCounts.filesTotal}
                                <span class="theme-text-muted text-xs">({libraryCounts.filesTotal} {t.nav.files})</span>
                            {/if}
                        </span>
                    </div>
                    <button
                        onclick={() => showLibrary = false}
                        class="p-1.5 rounded-lg hover:theme-bg-secondary theme-text-muted hover:theme-text-primary transition-colors"
                    title={t.common.close}
                >
                    <i class="fa-solid fa-xmark"></i>
                </button>
                </div>

                <!-- Toolbar -->
                <div class="flex items-center gap-1.5 px-3 py-2 border-b theme-border">
                    <!-- Sort dropdown -->
                    <div class="relative">
                        <button
                            onclick={() => sortMenuOpen = !sortMenuOpen}
                            class="h-7 px-2.5 rounded theme-bg-tertiary theme-text-primary text-xs flex items-center gap-1.5 hover:theme-bg-secondary"
                        >
                            <span>{sortBy === 'date' ? t.library.sortDate : sortBy === 'name' ? t.library.sortName : t.library.sortSize}</span>
                            <i class="fa-solid fa-chevron-down text-[10px] theme-text-muted"></i>
                        </button>
                        {#if sortMenuOpen}
                            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                            <div
                                class="fixed inset-0 z-40"
                                onclick={() => sortMenuOpen = false}
                            ></div>
                            <div class="absolute left-0 top-full mt-1 py-1 theme-bg-secondary border theme-border rounded shadow-lg z-50 min-w-[80px]">
                                <button
                                    onclick={() => { sortBy = 'date'; sortMenuOpen = false; }}
                                    class="w-full px-3 py-1.5 text-left text-xs flex items-center gap-2 {sortBy === 'date' ? 'theme-text-primary bg-[var(--accent)]/20' : 'theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary'}"
                                >
                                    {t.library.sortDate}
                                </button>
                                <button
                                    onclick={() => { sortBy = 'name'; sortMenuOpen = false; }}
                                    class="w-full px-3 py-1.5 text-left text-xs flex items-center gap-2 {sortBy === 'name' ? 'theme-text-primary bg-[var(--accent)]/20' : 'theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary'}"
                                >
                                    {t.library.sortName}
                                </button>
                                <button
                                    onclick={() => { sortBy = 'dimensions'; sortMenuOpen = false; }}
                                    class="w-full px-3 py-1.5 text-left text-xs flex items-center gap-2 {sortBy === 'dimensions' ? 'theme-text-primary bg-[var(--accent)]/20' : 'theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary'}"
                                >
                                    {t.library.sortSize}
                                </button>
                            </div>
                        {/if}
                    </div>

                    <button
                        onclick={() => (sortOrder = sortOrder === "asc" ? "desc" : "asc")}
                        class="w-7 h-7 rounded theme-bg-tertiary theme-text-secondary hover:theme-text-primary flex items-center justify-center"
                        title={sortOrder === "asc" ? t.library.ascending : t.library.descending}
                    >
                        <i class="fa-solid {sortOrder === 'asc' ? 'fa-arrow-up' : 'fa-arrow-down'} text-xs"></i>
                    </button>

                    <!-- Media type filter pills -->
                    <div class="flex items-center gap-0.5 ml-1.5 px-0.5 py-0.5 rounded theme-bg-primary">
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
                    onclick={(e) => handleShowInFinder(previewPhoto!.path, e)}
                    class="p-2 rounded hover:theme-bg-secondary theme-text-muted hover:theme-text-primary"
                    title={t.preview.showInFinder}
                >
                    <i class="fa-solid fa-folder-open text-sm"></i>
                </button>
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
        >
            {#key previewPhoto.path}
                <ImagePreview
                    src={previewPhoto.path}
                    alt={previewPhoto.path.split("/").pop() || "Preview"}
                    thumbPath={previewPhoto.thumb_path || ""}
                    {t}
                    mediaType={previewPhoto.media_type || 'photo'}
                    videoPath={previewPhoto.video_path || ''}
                />
            {/key}
        </div>

        <!-- Info Panel -->
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
            class="absolute top-0 right-0 bottom-0 w-64 theme-bg-secondary border-l theme-border pt-12 px-4 py-4 overflow-y-auto text-sm"
            onclick={(e) => e.stopPropagation()}
        >
            <div class="space-y-3">
                {#if previewPhoto.metadata.date_taken}
                    <div>
                        <p class="theme-text-muted text-xs">{t.photoInfo.date}</p>
                        <p class="theme-text-primary font-mono text-xs">{previewPhoto.metadata.date_taken}</p>
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
                        <button
                            onclick={(e) => { e.stopPropagation(); openDeleteConfirm('complete'); }}
                            disabled={isDeleting}
                            class="w-full px-3 py-2 rounded text-xs bg-red-600/20 text-red-400 hover:bg-red-600/30 disabled:opacity-50 flex items-center gap-2"
                        >
                            <i class="fa-solid fa-trash"></i>
                            {t.delete.deleteOriginal}
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
</style>
