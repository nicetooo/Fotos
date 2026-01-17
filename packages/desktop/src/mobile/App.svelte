<script lang="ts">
    import { onMount } from "svelte";
    import { invoke, convertFileSrc } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import { appDataDir, join } from "@tauri-apps/api/path";

    // Platform service
    import {
        setPlatformService,
        isIOSPlatform,
        type PlatformService,
        type IOSPlatformService,
        type ImportProgress,
    } from "../lib/platform";

    // Shared utilities and types
    import {
        type PhotoInfo,
        type Theme,
        type ImportStatus,
        formatFileSize,
        groupPhotos,
    } from "../shared";

    // i18n
    import { createI18nState, type Locale } from "../shared/i18n";

    // Components
    import Settings from "./components/Settings.svelte";
    import ImagePreview from "./components/ImagePreview.svelte";
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
    let photos = $state<PhotoInfo[]>([]);
    let dbPath = $state("");
    let thumbDir = $state("");
    let previewPhoto = $state<PhotoInfo | null>(null);
    let previewPhotoList = $state<PhotoInfo[] | null>(null);

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

    // Group RAW+JPEG pairs
    let groupedPhotos = $derived(groupPhotos(photos));

    onMount(async () => {
        try {
            // Check if iOS platform for specific features
            if (isIOSPlatform(platformService)) {
                iosPlatformService = platformService;
            }

            version = await invoke("get_core_version");
            const appData = await appDataDir();
            dbPath = await join(appData, "footos.db");
            thumbDir = await join(appData, "thumbnails");

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
                if (progress.current <= 5 || progress.current % 50 === 0) {
                    loadPhotos();
                }
            });

            platformService.onImportComplete(() => {
                loadPhotos();
            });

            // Listen for reload-photos event (e.g., after clear cache)
            await listen("reload-photos", () => loadPhotos());

            await loadPhotos();

            // iOS: Auto-sync photos if user has full access
            if (iosPlatformService) {
                // Subscribe to permission changes
                iosPlatformService.onPermissionChange((fullAccess) => {
                    hasFullAccess = fullAccess;
                });

                iosPlatformService.syncPhotosIfFullAccess();
                // Check initial permission status
                const status = await platformService.checkPermissionStatus();
                hasFullAccess = status === 'granted';
            }
        } catch (e) {
            error = t.errors.initFailed + ": " + e;
        }

        return () => {
            platformService.cleanup();
        };
    });

    async function loadPhotos() {
        if (!dbPath) return;
        try {
            photos = await invoke("list_photos", { dbPath, thumbDir });
        } catch (e) {
            console.error("Failed to list photos", e);
        }
    }

    async function handleImport() {
        try {
            error = "";
            importStatus = { success: 0, failure: 0, duplicates: 0, current: 0, total: 0, lastPath: "" };
            await platformService.requestImport();
        } catch (e) {
            error = String(e);
        }
    }

    function openPreview(photo: PhotoInfo, customList?: PhotoInfo[]) {
        previewPhoto = photo;
        previewPhotoList = customList || null;
    }

    function closePreview() {
        previewPhoto = null;
        previewPhotoList = null;
    }

    // Get the photo list to use for navigation
    let navigationPhotos = $derived(previewPhotoList || groupedPhotos);

    function navigatePreview(direction: "prev" | "next") {
        if (!previewPhoto) return;
        const photoList = navigationPhotos;
        const currentIndex = photoList.findIndex(p => p.path === previewPhoto!.path);
        if (currentIndex === -1) return;

        let newIndex: number;
        if (direction === "prev") {
            newIndex = currentIndex > 0 ? currentIndex - 1 : photoList.length - 1;
        } else {
            newIndex = currentIndex < photoList.length - 1 ? currentIndex + 1 : 0;
        }
        previewPhoto = photoList[newIndex];
    }
</script>

<main class="fixed inset-0 flex flex-col theme-bg-primary theme-text-primary overflow-hidden">
    <!-- Fullscreen Map -->
    <div class="flex-1 relative">
        <MapView photos={groupedPhotos} onOpenPreview={openPreview} theme={effectiveTheme} />

        <!-- Floating action buttons (top-left, vertical) -->
        <div class="absolute top-4 left-4 z-[1001] flex flex-col gap-2">
            <!-- Import button (hidden when iOS has full access since auto-sync handles it) -->
            {#if !hasFullAccess}
                <button
                    onclick={() => handleImport()}
                    disabled={isScanning}
                    class="w-12 h-12 rounded-full theme-bg-card backdrop-blur-sm border theme-border theme-text-secondary hover:theme-text-primary disabled:opacity-50 flex items-center justify-center shadow-lg transition-all"
                    title="Import photos"
                >
                    <i class="fa-solid {isScanning ? 'fa-spinner fa-spin' : 'fa-plus'} text-lg"></i>
                </button>
            {/if}

            <!-- Settings button -->
            <button
                onclick={() => showSettings = true}
                class="w-12 h-12 rounded-full theme-bg-card backdrop-blur-sm border theme-border theme-text-secondary hover:theme-text-primary flex items-center justify-center shadow-lg transition-all"
                title="Settings"
            >
                <i class="fa-solid fa-gear text-lg"></i>
            </button>
        </div>

        <!-- Import progress indicator -->
        {#if isScanning}
            <div class="absolute top-4 left-20 z-[1001] px-3 py-2 rounded-full bg-black/70 backdrop-blur-sm border border-white/20 text-sm text-white/80 flex items-center gap-2">
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
            <div class="absolute top-4 left-1/2 -translate-x-1/2 z-[1001] px-4 py-2 rounded-lg bg-red-900/80 backdrop-blur-sm border border-red-700/50 text-red-200 text-sm max-w-[90%]">
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
        onclick={() => showSettings = false}
    >
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
            class="theme-bg-secondary border-t theme-border rounded-t-2xl shadow-2xl w-full max-h-[80vh] overflow-hidden"
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
    {@const currentIndex = navigationPhotos.findIndex(p => p.path === previewPhoto.path)}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div
        class="fixed inset-0 z-50 bg-black"
        onclick={closePreview}
    >
        <!-- Top bar -->
        <div class="absolute top-0 left-0 right-0 h-14 flex items-center justify-between px-4 z-20 bg-black/80">
            <button
                onclick={closePreview}
                class="p-2 rounded hover:bg-white/10 text-white"
            >
                <i class="fa-solid fa-xmark text-lg"></i>
            </button>
            <span class="text-sm text-white/70">{currentIndex + 1} / {navigationPhotos.length}</span>
            <div class="w-10"></div>
        </div>

        <!-- Navigation buttons -->
        <button
            onclick={(e) => { e.stopPropagation(); navigatePreview("prev"); }}
            class="absolute left-2 top-1/2 -translate-y-1/2 p-4 rounded-full bg-black/50 text-white z-20"
        >
            <i class="fa-solid fa-chevron-left text-xl"></i>
        </button>
        <button
            onclick={(e) => { e.stopPropagation(); navigatePreview("next"); }}
            class="absolute right-2 top-1/2 -translate-y-1/2 p-4 rounded-full bg-black/50 text-white z-20"
        >
            <i class="fa-solid fa-chevron-right text-xl"></i>
        </button>

        <!-- Image area -->
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
            class="absolute top-14 left-0 right-0 bottom-0"
            onclick={(e) => e.stopPropagation()}
        >
            {#key previewPhoto.path}
                <ImagePreview
                    src={previewPhoto.path}
                    alt={previewPhoto.path.split("/").pop() || "Preview"}
                    thumbPath={previewPhoto.thumb_path || ""}
                />
            {/key}
        </div>

        <!-- Photo info (bottom sheet style) -->
        <div class="absolute bottom-0 left-0 right-0 bg-black/80 backdrop-blur px-4 py-3 z-20">
            <p class="text-white text-sm truncate">{previewPhoto.path.split("/").pop()}</p>
            <p class="text-white/60 text-xs mt-1">
                {previewPhoto.metadata.width} × {previewPhoto.metadata.height}
                · {formatFileSize(previewPhoto.file_size)}
            </p>
        </div>
    </div>
{/if}

<style>
    :global(body) {
        overflow: hidden;
    }
</style>
