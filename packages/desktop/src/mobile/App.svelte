<script lang="ts">
    import { onMount } from "svelte";
    import { invoke, convertFileSrc } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { appDataDir, join } from "@tauri-apps/api/path";

    // Platform service
    import {
        setPlatformService,
        isIOSPlatform,
        isAndroidPlatform,
        type PlatformService,
        type IOSPlatformService,
        type AndroidPlatformService,
        type ImportProgress,
    } from "../lib/platform";

    // Shared utilities and types
    import {
        type PhotoInfo,
        type PhotoPreview,
        type Theme,
        type ImportStatus,
        groupPhotos,
    } from "../shared";

    // i18n
    import { createI18nState, type Locale } from "../shared/i18n";

    // Components
    import Settings from "./components/Settings.svelte";
    import PhotoViewer from "./components/PhotoViewer.svelte";
    import MapView from "./components/MapView.svelte";

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

    // Two-phase photo loading:
    // Phase 1 (instant): lightweight previews for map markers + timeline
    // Phase 2 (lazy): full PhotoInfo loaded on demand for preview modal
    let previews = $state.raw<any[]>([]);
    let photos = $state.raw<PhotoInfo[]>([]);
    let fullDataLoaded = $state(false);

    let dbPath = $state("");
    let thumbDir = $state("");
    let previewPhoto = $state<PhotoInfo | null>(null);
    let previewPhotoList = $state.raw<PhotoInfo[] | null>(null);

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

    // Group RAW+JPEG pairs (only when full data is loaded)
    let groupedPhotos = $derived(groupPhotos(photos));
    let photosByPath = $derived.by(() => {
        const map = new Map<string, PhotoInfo>();
        for (const photo of photos) {
            map.set(photo.path, photo);
        }
        return map;
    });

    // Map-compatible photos: uses lightweight previews for instant render,
    // upgrades to full grouped data when available.
    let mapPhotos = $derived.by(() => {
        if (fullDataLoaded && groupedPhotos.length > 0) {
            return groupedPhotos;
        }
        if (previews.length > 0) {
            return previews;
        }
        return groupedPhotos;
    });

    onMount(async () => {
        // Back button support: close modals on browser history navigation
        window.addEventListener('popstate', handlePopState);

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
                if (progress.current <= 5 || progress.current % 400 === 0) {
                    loadPreviews();
                }
            });

            platformService.onImportComplete((result) => {
                console.log("[App] Import complete callback triggered:", result);
                loadPreviews().then(() => {
                    iosPlatformService?.setKnownPhotoIdentifiers(iosIdentifiersFrom(previews));
                });
                loadFullPhotos();
            });

            // Listen for reload-photos event (e.g., after clear cache)
            await listen("reload-photos", () => {
                fullDataLoaded = false;
                loadPreviews();
                loadFullPhotos();
            });

            // Phase 1: geotagged previews only — enough for map + timeline.
            await loadPreviews();
            mark("previews");

            // iOS: incremental PhotoKit sync after first paint. Known IDs come
            // from a cheap path-only query so we do not miss non-geotagged rows.
            if (iosPlatformService) {
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
            if (androidPlatformService) {
                androidPlatformService.onPermissionChange((hasAccess: boolean) => {
                    hasFullAccess = hasAccess;
                });

                androidPlatformService.syncPhotosIfHasAccess();
            }
            (window as any).__footosOpenPreview = (mediaType?: string) => {
                const list = mapPhotos.filter((p: any) => p.metadata?.lat && p.metadata?.lon);
                if (list.length === 0) return false;
                const pick = mediaType
                    ? list.find((p: any) => (p.media_type || p.metadata?.media_type) === mediaType) || list[0]
                    : list[0];
                openPreview(pick, list);
                return { ok: true, media_type: pick.media_type || pick.metadata?.media_type || "photo", path: pick.path };
            };
            (window as any).__footosClosePreview = () => closePreview();
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

    /**
     * Phase 1: Fast preview loading.
     * Fetches lightweight PhotoPreview data (id, path, thumb_path, lat, lon, date_taken).
     * Zero file I/O — completes in <50ms even for 10K+ photos.
     * Used for map markers and timeline rendering.
     */
    async function loadPreviews() {
        console.log("[App] loadPreviews called, dbPath:", dbPath);
        if (!dbPath) {
            console.log("[App] loadPreviews: dbPath is empty, skipping");
            return;
        }
        try {
            const startTime = performance.now();
            const file = await invoke("list_map_previews") as {
                path: string;
                rows: number;
                bytes: number;
                rust_ms: number;
            };
            const boot = (window as any).__footosBoot;
            if (boot?.marks) boot.marks.preview_rust = file.rust_ms;
            const res = await fetch(convertFileSrc(file.path));
            const pack = await res.json() as {
                id: number[];
                path: string[];
                thumb: string[];
                lat: number[];
                lon: number[];
                date: string[];
                media: string[];
                dur: number[];
            };
            const previewList: any[] = new Array(pack.path.length);
            for (let i = 0; i < pack.path.length; i++) {
                previewList[i] = {
                    id: { id: pack.id[i] },
                    path: pack.path[i],
                    thumb_path: resolveThumbPath(pack.thumb[i]) || pack.thumb[i],
                    media_type: pack.media[i] || "photo",
                    metadata: {
                        lat: pack.lat[i],
                        lon: pack.lon[i],
                        date_taken: pack.date[i] || null,
                        width: 0,
                        height: 0,
                        duration_ms: pack.dur[i] || null,
                    },
                    hash: "",
                    file_size: 0,
                };
            }
            const elapsed = (performance.now() - startTime).toFixed(1);
            console.log(`[App] loadPreviews: ${previewList.length} rows rust=${file.rust_ms}ms ipc=${elapsed}ms bytes=${file.bytes}`);
            previews = previewList;
        } catch (e) {
            console.error("[App] Failed to list previews", e);
            // Fallback: try full load
            loadFullPhotos();
        }
    }

    /**
     * Phase 2: Full data loading.
     * Fetches complete PhotoInfo with metadata, hash, etc.
     * Used for preview modal (dimensions, camera info, file size).
     * Runs in background after initial render.
     */
    async function loadFullPhotos() {
        if (!dbPath) return;
        try {
            const startTime = performance.now();
            const result = await invoke("list_photos");
            const elapsed = (performance.now() - startTime).toFixed(1);
            const photoList = result as PhotoInfo[];
            console.log(`[App] loadFullPhotos: got ${photoList.length} photos in ${elapsed}ms`);
            photos = photoList.map((p) => ({
                ...p,
                thumb_path: resolveThumbPath(p.thumb_path) || p.thumb_path,
            }));
            fullDataLoaded = true;
        } catch (e) {
            console.error("[App] Failed to load full photos", e);
        }
    }

    function iosIdentifiersFrom(list: PhotoPreview[]): string[] {
        return list
            .filter((p) => p.path.startsWith("ios-photo://"))
            .map((p) => p.path.slice("ios-photo://".length));
    }

    /** Reinstalls change the iOS container UUID; DB may still hold the old absolute path. */
    function resolveThumbPath(stored: string | null | undefined): string {
        if (!stored) return "";
        const name = stored.split("/").pop();
        if (!name || !thumbDir) return stored;
        return `${thumbDir.replace(/\/$/, "")}/${name}`;
    }

    async function handleImport() {
        try {
            error = "";
            importStatus = { success: 0, failure: 0, duplicates: 0, current: 0, total: 0, lastPath: "" };
            iosPlatformService?.setKnownPhotoIdentifiers(iosIdentifiersFrom(previews));
            await platformService.requestImport();
        } catch (e) {
            error = String(e);
        }
    }

    function openPreview(photo: any, customList?: any[]) {
        previewPhoto = photosByPath.get(photo.path) || photo;
        previewPhotoList = customList || null;
        history.pushState({ modal: "preview" }, "");
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
        previewPhoto = null;
        previewPhotoList = null;
        isClosingManually = true;
        history.back();
    }

    let navigationPhotos = $derived(previewPhotoList || groupedPhotos);

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
            previewPhoto = null;
            previewPhotoList = null;
        } else if (showSettings) {
            showSettings = false;
        }
    }
</script>

<main class="fixed inset-0 flex flex-col theme-bg-primary theme-text-primary overflow-hidden">
    <!-- Fullscreen Map -->
    <div class="flex-1 relative">
        <MapView photos={mapPhotos} onOpenPreview={openPreview} theme={effectiveTheme} {t} {locale} isSyncing={isScanning} />

        <!-- Floating action buttons (top-left, vertical) -->
        <div class="absolute left-4 z-[1001] flex flex-col gap-2" style="top: calc(max(47px, env(safe-area-inset-top, 0px)) + 12px)">
            <button
                onclick={() => handleImport()}
                disabled={isScanning}
                class="w-12 h-12 rounded-full theme-bg-card backdrop-blur-sm border theme-border theme-text-secondary hover:theme-text-primary disabled:opacity-50 flex items-center justify-center shadow-lg transition-all"
                title={t.import.importPhotos}
            >
                <i class="fa-solid {isScanning ? 'fa-spinner fa-spin' : 'fa-plus'} text-lg"></i>
            </button>

            <!-- Settings button -->
            <button
                onclick={openSettings}
                class="w-12 h-12 rounded-full theme-bg-card backdrop-blur-sm border theme-border theme-text-secondary hover:theme-text-primary flex items-center justify-center shadow-lg transition-all"
                title={t.common.settings}
            >
                <i class="fa-solid fa-gear text-lg"></i>
            </button>
        </div>

        <!-- Import progress indicator -->
        {#if isScanning}
            <div class="absolute left-20 z-[1001] px-3 py-2 rounded-full bg-black/70 backdrop-blur-sm border border-white/20 text-sm text-white/80 flex items-center gap-2" style="top: calc(max(47px, env(safe-area-inset-top, 0px)) + 12px)">
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
        class="fixed inset-0 z-[2000] bg-black/60 backdrop-blur-sm flex items-end justify-center"
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

{#if previewPhoto}
    <PhotoViewer
        photos={navigationPhotos}
        currentPath={previewPhoto.path}
        {t}
        {locale}
        onClose={closePreview}
        onNavigate={(photo) => {
            if (!photo) return;
            previewPhoto = photo;
        }}
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
