<script lang="ts">
    import { onMount } from "svelte";
    import { invoke, convertFileSrc } from "@tauri-apps/api/core";
    import { open } from "@tauri-apps/plugin-dialog";
    import { revealItemInDir } from "@tauri-apps/plugin-opener";
    import { appDataDir, join } from "@tauri-apps/api/path";
    import { listen } from "@tauri-apps/api/event";
    import Settings from "./components/Settings.svelte";
    import ImagePreview from "./components/ImagePreview.svelte";
    import MapView from "./components/Map.svelte";
    import type { PhotoInfo } from "./types";

    let version = $state("...");
    let showSettings = $state(false);
    let showLibrary = $state(false);
    let importStatus = $state({
        success: 0,
        failure: 0,
        current: 0,
        total: 0,
        lastPath: "",
    });
    let isScanning = $state(false);
    let error = $state("");
    let photos = $state<PhotoInfo[]>([]);
    let dbPath = $state("");
    let thumbDir = $state("");
    let uniqueTs = $state(Date.now());
    let previewPhoto = $state<PhotoInfo | null>(null);
    let previewPhotoList = $state<PhotoInfo[] | null>(null); // Custom list for map view
    let sortBy = $state<"name" | "date" | "size" | "dimensions">("date");
    let sortOrder = $state<"asc" | "desc">("desc");
    let importMenuOpen = $state(false);

    // Theme state
    type Theme = "dark" | "light" | "system";
    const THEME_KEY = "fotos-theme";
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

    // Listen for system theme changes
    $effect(() => {
        if (theme !== "system") return;
        const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
        const handler = () => applyTheme("system");
        mediaQuery.addEventListener("change", handler);
        return () => mediaQuery.removeEventListener("change", handler);
    });

    // RAW file extensions
    const RAW_EXTENSIONS = new Set(["cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "dng", "raf", "orf", "rw2", "pef", "raw"]);
    const JPEG_EXTENSIONS = new Set(["jpg", "jpeg"]);

    function isRawFile(path: string): boolean {
        const ext = path.split(".").pop()?.toLowerCase() || "";
        return RAW_EXTENSIONS.has(ext);
    }

    function isJpegFile(path: string): boolean {
        const ext = path.split(".").pop()?.toLowerCase() || "";
        return JPEG_EXTENSIONS.has(ext);
    }

    function getBaseName(path: string): string {
        const fileName = path.split("/").pop() || "";
        const lastDot = fileName.lastIndexOf(".");
        return lastDot > 0 ? fileName.substring(0, lastDot).toLowerCase() : fileName.toLowerCase();
    }

    // Group RAW+JPEG pairs: show JPEG with RAW badge, hide standalone RAW
    let groupedPhotos = $derived.by(() => {
        // Build a map of base name -> photos
        const byBaseName = new Map<string, { jpegs: PhotoInfo[]; raws: PhotoInfo[] }>();

        for (const photo of photos) {
            const baseName = getBaseName(photo.path);
            if (!byBaseName.has(baseName)) {
                byBaseName.set(baseName, { jpegs: [], raws: [] });
            }
            const group = byBaseName.get(baseName)!;
            if (isRawFile(photo.path)) {
                group.raws.push(photo);
            } else if (isJpegFile(photo.path)) {
                group.jpegs.push(photo);
            } else {
                // Other formats (PNG, etc.) - treat as JPEG for grouping
                group.jpegs.push(photo);
            }
        }

        // Build result: for each group, show JPEGs with RAW badge if RAW exists
        const result: PhotoInfo[] = [];
        for (const [_, group] of byBaseName) {
            if (group.jpegs.length > 0) {
                // Has JPEG - show JPEG(s) with RAW badge if RAW exists
                for (const jpeg of group.jpegs) {
                    if (group.raws.length > 0) {
                        result.push({
                            ...jpeg,
                            hasRaw: true,
                            rawPath: group.raws[0].path,
                        });
                    } else {
                        result.push(jpeg);
                    }
                }
            } else {
                // Only RAW - show RAW files
                result.push(...group.raws);
            }
        }

        return result;
    });

    let sortedPhotos = $derived.by(() => {
        const photosCopy = [...groupedPhotos];
        photosCopy.sort((a, b) => {
            let comparison = 0;
            switch (sortBy) {
                case "name":
                    const nameA = a.path.split("/").pop() || "";
                    const nameB = b.path.split("/").pop() || "";
                    comparison = nameA.localeCompare(nameB);
                    break;
                case "date":
                    const dateA = a.metadata.date_taken || "";
                    const dateB = b.metadata.date_taken || "";
                    comparison = dateA.localeCompare(dateB);
                    break;
                case "size":
                case "dimensions":
                    const pixelsA = a.metadata.width * a.metadata.height;
                    const pixelsB = b.metadata.width * b.metadata.height;
                    comparison = pixelsA - pixelsB;
                    break;
            }
            return sortOrder === "asc" ? comparison : -comparison;
        });
        return photosCopy;
    });

    onMount(async () => {
        try {
            version = await invoke("get_core_version");
            const appData = await appDataDir();
            dbPath = await join(appData, "fotos.db");
            thumbDir = await join(appData, "thumbnails");

            await listen("import-progress", (event: any) => {
                const payload = event.payload;
                importStatus = {
                    ...importStatus,
                    success: payload.success,
                    failure: payload.failure,
                    current: payload.current,
                    total: payload.total,
                    lastPath: payload.last_path,
                };
                if (payload.current <= 5 || payload.current % 50 === 0) {
                    loadPhotos();
                }
            });

            await listen("reload-photos", () => loadPhotos());
            await loadPhotos();
        } catch (e) {
            error = "Failed to initialize: " + e;
        }
    });

    async function loadPhotos() {
        if (!dbPath) return;
        try {
            photos = await invoke("list_photos", { dbPath, thumbDir });
            uniqueTs = Date.now();
        } catch (e) {
            console.error("Failed to list photos", e);
        }
    }

    async function handleScan(mode: "folder" | "file" = "folder") {
        try {
            const selected = await open({
                directory: mode === "folder",
                multiple: false,
                filters: mode === "file" ? [{
                    name: "Images",
                    extensions: ["jpg", "jpeg", "png", "webp", "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "dng", "raf", "orf", "rw2", "pef", "raw"]
                }] : undefined
            });
            if (!selected) return;
            const rootPath = Array.isArray(selected) ? selected[0] : selected;

            isScanning = true;
            error = "";

            const result = await invoke("import_photos", {
                rootPath,
                dbPath,
                thumbDir,
            });
            importStatus = result as any;
            await loadPhotos();
        } catch (e) {
            error = String(e);
        } finally {
            isScanning = false;
        }
    }

    async function handleShowInFinder(path: string, e: MouseEvent) {
        e.stopPropagation();
        try {
            await revealItemInDir(path);
        } catch (e) {
            alert("Failed to open location: " + e);
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
    let navigationPhotos = $derived(previewPhotoList || sortedPhotos);

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

    function handleKeydown(e: KeyboardEvent) {
        // Close modals with Escape
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

        // Preview navigation (zoom handled by ImagePreview component)
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

        // Global shortcuts
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
        <MapView photos={groupedPhotos} onOpenPreview={openPreview} />

        <!-- Floating action buttons (top-left) -->
        <div class="absolute top-4 left-4 z-[1001] flex flex-col gap-2">
            <!-- Import button -->
            <div class="relative">
                <button
                    onclick={() => importMenuOpen = !importMenuOpen}
                    disabled={isScanning}
                    class="w-10 h-10 rounded-full theme-bg-card backdrop-blur-sm border theme-border theme-text-secondary hover:theme-text-primary disabled:opacity-50 flex items-center justify-center shadow-lg transition-all"
                    title="Import photos"
                >
                    <i class="fa-solid {isScanning ? 'fa-spinner fa-spin' : 'fa-plus'}"></i>
                </button>
                {#if importMenuOpen}
                    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                    <div
                        class="fixed inset-0 z-40"
                        onclick={() => importMenuOpen = false}
                    ></div>
                    <div class="absolute left-0 top-full mt-2 py-1 theme-bg-overlay backdrop-blur-sm border theme-border rounded-lg shadow-lg z-50 min-w-[140px]">
                        <button
                            onclick={() => { importMenuOpen = false; handleScan("folder"); }}
                            disabled={isScanning}
                            class="w-full px-3 py-2 text-left text-sm theme-text-secondary hover:theme-bg-secondary flex items-center gap-2"
                        >
                            <i class="fa-solid fa-folder text-xs"></i>
                            Import Folder
                        </button>
                        <button
                            onclick={() => { importMenuOpen = false; handleScan("file"); }}
                            disabled={isScanning}
                            class="w-full px-3 py-2 text-left text-sm theme-text-secondary hover:theme-bg-secondary flex items-center gap-2"
                        >
                            <i class="fa-solid fa-file-image text-xs"></i>
                            Import File
                        </button>
                    </div>
                {/if}
            </div>

            <!-- Library button -->
            <button
                onclick={() => showLibrary = !showLibrary}
                class="w-10 h-10 rounded-full theme-bg-card backdrop-blur-sm border theme-border theme-text-secondary hover:theme-text-primary flex items-center justify-center shadow-lg transition-all {showLibrary ? 'theme-bg-secondary' : ''}"
                title="Library"
            >
                <i class="fa-solid fa-images"></i>
            </button>

            <!-- Settings button -->
            <button
                onclick={() => showSettings = true}
                class="w-10 h-10 rounded-full theme-bg-card backdrop-blur-sm border theme-border theme-text-secondary hover:theme-text-primary flex items-center justify-center shadow-lg transition-all"
                title="Settings"
            >
                <i class="fa-solid fa-gear"></i>
            </button>
        </div>

        <!-- Library Drawer (right side) -->
        {#if showLibrary}
            <div class="absolute top-0 right-0 bottom-0 min-w-[500px] w-1/2 max-w-[800px] theme-bg-overlay backdrop-blur-md border-l theme-border z-[1001] flex flex-col">
                <!-- Header -->
                <div class="flex items-center justify-between px-4 py-3 border-b theme-border">
                    <div class="flex items-center gap-2">
                        <i class="fa-solid fa-images theme-text-muted text-sm"></i>
                        <span class="theme-text-primary font-medium">
                            {sortedPhotos.length} Photos
                            {#if sortedPhotos.length !== photos.length}
                                <span class="theme-text-muted text-xs">({photos.length} files)</span>
                            {/if}
                        </span>
                    </div>
                    <button
                        onclick={() => showLibrary = false}
                        class="p-1.5 rounded-lg hover:theme-bg-secondary theme-text-muted hover:theme-text-primary transition-colors"
                        title="Close"
                    >
                        <i class="fa-solid fa-xmark"></i>
                    </button>
                </div>

                <!-- Toolbar -->
                <div class="flex items-center gap-2 px-3 py-2 border-b theme-border">
                    <select
                        bind:value={sortBy}
                        class="flex-1 theme-bg-secondary theme-text-secondary text-xs px-2 py-1.5 rounded border theme-border focus:outline-none"
                    >
                        <option value="date">Date</option>
                        <option value="name">Name</option>
                        <option value="dimensions">Size</option>
                    </select>

                    <button
                        onclick={() => (sortOrder = sortOrder === "asc" ? "desc" : "asc")}
                        class="p-1.5 rounded theme-bg-secondary border theme-border theme-text-muted hover:theme-text-primary"
                        title={sortOrder === "asc" ? "Ascending" : "Descending"}
                    >
                        <i class="fa-solid {sortOrder === 'asc' ? 'fa-arrow-up' : 'fa-arrow-down'} text-xs"></i>
                    </button>

                    <button
                        onclick={() => { showLibrary = false; handleScan("folder"); }}
                        disabled={isScanning}
                        class="p-1.5 rounded theme-bg-secondary border theme-border theme-text-muted hover:theme-text-primary disabled:opacity-50"
                        title="Import folder"
                    >
                        <i class="fa-solid {isScanning ? 'fa-spinner fa-spin' : 'fa-folder-plus'} text-xs"></i>
                    </button>
                </div>

                <!-- Photo Grid -->
                <div class="flex-1 overflow-y-auto p-2">
                    {#if sortedPhotos.length > 0}
                        <div class="grid grid-cols-4 sm:grid-cols-5 md:grid-cols-6 gap-1.5">
                            {#each sortedPhotos as photo (photo.path)}
                                <button
                                    class="aspect-square relative overflow-hidden rounded theme-bg-secondary hover:ring-2 hover:ring-[var(--accent)] transition-all group"
                                    onclick={() => openPreview(photo)}
                                    title={photo.path.split("/").pop()}
                                >
                                    <img
                                        src={convertFileSrc(photo.thumb_path || photo.path)}
                                        alt=""
                                        class="w-full h-full object-cover"
                                        loading="lazy"
                                    />
                                    {#if photo.hasRaw}
                                        <div class="absolute top-1 right-1 bg-amber-600 text-white text-[8px] font-bold px-1 rounded">R</div>
                                    {/if}
                                    {#if photo.metadata?.lat && photo.metadata?.lon}
                                        <div class="absolute bottom-1 left-1 text-white/70 text-[10px]">
                                            <i class="fa-solid fa-location-dot"></i>
                                        </div>
                                    {/if}
                                </button>
                            {/each}
                        </div>
                    {:else if !isScanning}
                        <div class="h-full flex flex-col items-center justify-center text-white/50 py-12">
                            <i class="fa-solid fa-images text-2xl mb-3"></i>
                            <p class="text-sm">No photos</p>
                            <button
                                onclick={() => handleScan("folder")}
                                class="mt-3 px-3 py-1.5 rounded bg-white/10 text-white/80 text-xs hover:bg-white/20"
                            >
                                Import Photos
                            </button>
                        </div>
                    {:else}
                        <div class="h-full flex flex-col items-center justify-center text-white/50 py-12">
                            <i class="fa-solid fa-spinner fa-spin text-xl mb-2"></i>
                            <p class="text-sm">
                                {#if importStatus.total > 0}
                                    {importStatus.current} / {importStatus.total}
                                {:else}
                                    Scanning...
                                {/if}
                            </p>
                        </div>
                    {/if}
                </div>
            </div>
        {/if}

        <!-- Import progress indicator -->
        {#if isScanning}
            <div class="absolute top-4 left-16 z-[1001] px-3 py-2 rounded-full bg-black/70 backdrop-blur-sm border border-white/20 text-sm text-white/80 flex items-center gap-2">
                <i class="fa-solid fa-spinner fa-spin text-xs"></i>
                {#if importStatus.total > 0}
                    <span>{importStatus.current} / {importStatus.total}</span>
                {:else}
                    <span>Scanning...</span>
                {/if}
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
                <h2 class="text-lg font-medium theme-text-primary">Settings</h2>
                <button
                    onclick={() => showSettings = false}
                    class="p-2 rounded-lg hover:theme-bg-tertiary theme-text-muted hover:theme-text-primary transition-colors"
                >
                    <i class="fa-solid fa-xmark"></i>
                </button>
            </div>
            <div class="p-6 overflow-y-auto max-h-[calc(80vh-60px)]">
                <Settings {dbPath} {thumbDir} {version} {theme} onThemeChange={(t) => theme = t} />
            </div>
        </div>
    </div>
{/if}

{#if previewPhoto}
    {@const currentIndex = navigationPhotos.findIndex(p => p.path === previewPhoto.path)}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div
        class="fixed inset-0 z-50 bg-black/90"
        onclick={closePreview}
    >
        <!-- Top bar -->
        <div class="absolute top-0 left-0 right-64 h-12 flex items-center justify-between px-4 z-20 theme-bg-overlay">
            <div class="flex items-center gap-3">
                <span class="text-xs theme-text-muted">{currentIndex + 1} / {navigationPhotos.length}</span>
                <span class="text-sm theme-text-secondary truncate">
                    {previewPhoto.path.split("/").pop()}
                </span>
            </div>
            <div class="flex items-center gap-2">
                <button
                    onclick={(e) => handleShowInFinder(previewPhoto!.path, e)}
                    class="p-2 rounded hover:theme-bg-secondary theme-text-muted hover:theme-text-primary"
                    title="Show in Finder"
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
            title="Previous (←)"
        >
            <i class="fa-solid fa-chevron-left"></i>
        </button>
        <button
            onclick={(e) => { e.stopPropagation(); navigatePreview("next"); }}
            class="absolute right-72 top-1/2 -translate-y-1/2 p-3 rounded-full theme-bg-card hover:theme-bg-secondary theme-text-secondary hover:theme-text-primary z-20"
            title="Next (→)"
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
                <ImagePreview src={previewPhoto.path} alt={previewPhoto.path.split("/").pop() || "Preview"} />
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
                        <p class="theme-text-muted text-xs">Date</p>
                        <p class="theme-text-primary font-mono text-xs">{previewPhoto.metadata.date_taken}</p>
                    </div>
                {/if}

                <div>
                    <p class="theme-text-muted text-xs">Dimensions</p>
                    <p class="theme-text-primary">{previewPhoto.metadata.width} × {previewPhoto.metadata.height}</p>
                </div>

                {#if previewPhoto.metadata.make || previewPhoto.metadata.model}
                    <div>
                        <p class="theme-text-muted text-xs">Camera</p>
                        <p class="theme-text-primary">{previewPhoto.metadata.make || ""} {previewPhoto.metadata.model || ""}</p>
                    </div>
                {/if}

                {#if previewPhoto.metadata.iso || previewPhoto.metadata.f_number || previewPhoto.metadata.exposure_time}
                    <div>
                        <p class="theme-text-muted text-xs">Exposure</p>
                        <p class="theme-text-primary">
                            {#if previewPhoto.metadata.iso}ISO {previewPhoto.metadata.iso}{/if}
                            {#if previewPhoto.metadata.f_number} f/{previewPhoto.metadata.f_number.toFixed(1)}{/if}
                            {#if previewPhoto.metadata.exposure_time} {previewPhoto.metadata.exposure_time}{/if}
                        </p>
                    </div>
                {/if}

                {#if previewPhoto.metadata.lat && previewPhoto.metadata.lon}
                    <div>
                        <p class="theme-text-muted text-xs">GPS</p>
                        <p class="theme-text-primary font-mono text-xs">
                            {previewPhoto.metadata.lat.toFixed(6)}, {previewPhoto.metadata.lon.toFixed(6)}
                        </p>
                    </div>
                {/if}

                <div>
                    <p class="theme-text-muted text-xs">Path</p>
                    <p class="theme-text-primary text-xs font-mono break-all">{previewPhoto.path}</p>
                </div>
            </div>
        </div>
    </div>
{/if}

<style>
    :global(body) {
        overflow: hidden;
    }
</style>
