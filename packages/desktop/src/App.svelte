<script lang="ts">
    import { onMount } from "svelte";
    import { invoke, convertFileSrc } from "@tauri-apps/api/core";
    import { open } from "@tauri-apps/plugin-dialog";
    import { revealItemInDir } from "@tauri-apps/plugin-opener";
    import { appDataDir, join } from "@tauri-apps/api/path";
    import { listen } from "@tauri-apps/api/event";
    import { platform } from "@tauri-apps/plugin-os";

    // Platform detection
    let currentPlatform = $state<string>("unknown");
    let isMobile = $derived(currentPlatform === "ios" || currentPlatform === "android");
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

    // Sort state with localStorage persistence
    const SORT_BY_KEY = "fotos-sort-by";
    const SORT_ORDER_KEY = "fotos-sort-order";
    let sortBy = $state<"name" | "date" | "size" | "dimensions">((() => {
        if (typeof localStorage !== "undefined") {
            const saved = localStorage.getItem(SORT_BY_KEY);
            if (saved === "name" || saved === "date" || saved === "size" || saved === "dimensions") {
                return saved;
            }
        }
        return "date";
    })());
    let sortOrder = $state<"asc" | "desc">((() => {
        if (typeof localStorage !== "undefined") {
            const saved = localStorage.getItem(SORT_ORDER_KEY);
            if (saved === "asc" || saved === "desc") {
                return saved;
            }
        }
        return "desc";
    })());

    // Save sort options when changed
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

    let importMenuOpen = $state(false);
    let libraryImportMenuOpen = $state(false);
    let sortMenuOpen = $state(false);

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

    function formatFileSize(bytes: number): string {
        if (bytes === 0) return "0 B";
        const units = ["B", "KB", "MB", "GB"];
        const k = 1024;
        const i = Math.floor(Math.log(bytes) / Math.log(k));
        const size = bytes / Math.pow(k, i);
        return `${size.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
    }

    // Get RAW file size for a photo that has an associated RAW
    function getRawFileSize(photo: PhotoInfo): number {
        if (!photo.hasRaw || !photo.rawPath) return 0;
        const rawPhoto = photos.find(p => p.path === photo.rawPath);
        return rawPhoto?.file_size || 0;
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
                // Only RAW - mark as standalone RAW
                for (const raw of group.raws) {
                    result.push({
                        ...raw,
                        isRawOnly: true,
                    });
                }
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
            // Detect platform
            currentPlatform = await platform();
            console.log("Platform:", currentPlatform);

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

            await listen("import-cancelled", async () => {
                isScanning = false;
                await loadPhotos();
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

    async function handleCancelImport() {
        try {
            await invoke("cancel_import");
        } catch (e) {
            console.error("Failed to cancel import:", e);
        }
    }

    async function handlePhotoLibraryAccess() {
        try {
            // On iOS, this will trigger the native photo picker
            // The actual implementation needs Swift code via Tauri plugin
            isScanning = true;
            error = "";

            const result = await invoke("request_photo_library_access", {
                dbPath,
                thumbDir,
            });

            await loadPhotos();
        } catch (e) {
            error = "Photo library access: " + String(e);
            console.log("Photo library not yet implemented - needs native iOS code");
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

    // Delete state
    let deleteConfirmOpen = $state(false);
    let deleteMode = $state<'app' | 'complete'>('app');
    let deleteIncludeRaw = $state(true); // Whether to include RAW when deleting JPEG+RAW pair
    let isDeleting = $state(false);

    async function handleDeletePhoto(photo: PhotoInfo, mode: 'app' | 'complete', includeRaw: boolean = true) {
        if (!photo.id) return;

        isDeleting = true;
        try {
            const ids = [photo.id.id];
            // Also include RAW file if present and user chose to delete both
            if (includeRaw && photo.hasRaw && photo.rawPath) {
                const rawPhoto = photos.find(p => p.path === photo.rawPath);
                if (rawPhoto?.id) {
                    ids.push(rawPhoto.id.id);
                }
            }

            const command = mode === 'complete' ? 'delete_photos_completely' : 'delete_photos_from_app';
            const result = await invoke(command, { ids, dbPath, thumbDir });
            console.log('Delete result:', result);

            // Remember the "next" photo's path before reload
            // This works for both library view and map view
            const currentList = navigationPhotos;
            const currentIndex = currentList.findIndex(p => p.path === photo.path);
            const nextPhoto = currentIndex < currentList.length - 1
                ? currentList[currentIndex + 1]
                : (currentIndex > 0 ? currentList[currentIndex - 1] : null);
            const nextPhotoPath = nextPhoto?.path;

            // Reload photos
            await loadPhotos();

            // Clear the custom list (map view list) since it's now stale
            previewPhotoList = null;

            // Find and show the next photo by path
            if (nextPhotoPath) {
                const foundPhoto = sortedPhotos.find(p => p.path === nextPhotoPath);
                if (foundPhoto) {
                    previewPhoto = foundPhoto;
                } else if (sortedPhotos.length > 0) {
                    // Photo not found (might have been filtered out), show first available
                    previewPhoto = sortedPhotos[0];
                } else {
                    closePreview();
                }
            } else if (sortedPhotos.length > 0) {
                previewPhoto = sortedPhotos[0];
            } else {
                closePreview();
            }
        } catch (e) {
            console.error('Delete failed:', e);
            error = 'Delete failed: ' + e;
        } finally {
            isDeleting = false;
            deleteConfirmOpen = false;
        }
    }

    function openDeleteConfirm(mode: 'app' | 'complete') {
        deleteMode = mode;
        deleteIncludeRaw = true; // Reset to default
        deleteConfirmOpen = true;
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
        <MapView photos={groupedPhotos} onOpenPreview={openPreview} theme={effectiveTheme} />

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
                    <div class="absolute left-0 top-full mt-2 py-1 theme-bg-overlay backdrop-blur-sm border theme-border rounded-lg shadow-lg z-50 min-w-[160px]">
                        {#if isMobile}
                            <button
                                onclick={() => { importMenuOpen = false; handlePhotoLibraryAccess(); }}
                                disabled={isScanning}
                                class="w-full px-3 py-2 text-left text-sm theme-text-secondary hover:theme-bg-secondary flex items-center gap-2"
                            >
                                <i class="fa-solid fa-images text-xs"></i>
                                Photo Library
                            </button>
                        {:else}
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
                        {/if}
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
            <!-- Overlay to close drawer when clicking outside -->
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
                <div class="flex items-center gap-1.5 px-3 py-2 border-b theme-border">
                    <!-- Sort dropdown -->
                    <div class="relative">
                        <button
                            onclick={() => sortMenuOpen = !sortMenuOpen}
                            class="h-7 px-2.5 rounded theme-bg-tertiary theme-text-primary text-xs flex items-center gap-1.5 hover:theme-bg-secondary"
                        >
                            <span>{sortBy === 'date' ? 'Date' : sortBy === 'name' ? 'Name' : 'Size'}</span>
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
                                    Date
                                </button>
                                <button
                                    onclick={() => { sortBy = 'name'; sortMenuOpen = false; }}
                                    class="w-full px-3 py-1.5 text-left text-xs flex items-center gap-2 {sortBy === 'name' ? 'theme-text-primary bg-[var(--accent)]/20' : 'theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary'}"
                                >
                                    Name
                                </button>
                                <button
                                    onclick={() => { sortBy = 'dimensions'; sortMenuOpen = false; }}
                                    class="w-full px-3 py-1.5 text-left text-xs flex items-center gap-2 {sortBy === 'dimensions' ? 'theme-text-primary bg-[var(--accent)]/20' : 'theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary'}"
                                >
                                    Size
                                </button>
                            </div>
                        {/if}
                    </div>

                    <button
                        onclick={() => (sortOrder = sortOrder === "asc" ? "desc" : "asc")}
                        class="w-7 h-7 rounded theme-bg-tertiary theme-text-secondary hover:theme-text-primary flex items-center justify-center"
                        title={sortOrder === "asc" ? "Ascending" : "Descending"}
                    >
                        <i class="fa-solid {sortOrder === 'asc' ? 'fa-arrow-up' : 'fa-arrow-down'} text-xs"></i>
                    </button>

                    <div class="flex-1"></div>

                    <div class="relative">
                        <button
                            onclick={() => libraryImportMenuOpen = !libraryImportMenuOpen}
                            disabled={isScanning}
                            class="w-7 h-7 rounded theme-bg-tertiary theme-text-secondary hover:theme-text-primary disabled:opacity-50 flex items-center justify-center"
                            title="Import"
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
                                    Import Folder
                                </button>
                                <button
                                    onclick={() => { libraryImportMenuOpen = false; handleScan("file"); }}
                                    disabled={isScanning}
                                    class="w-full px-3 py-1.5 text-left text-xs theme-text-secondary hover:theme-text-primary hover:theme-bg-tertiary flex items-center gap-2"
                                >
                                    <i class="fa-solid fa-file-image w-3"></i>
                                    Import File
                                </button>
                            </div>
                        {/if}
                    </div>
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
                                    {:else if photo.isRawOnly}
                                        <div class="absolute top-1 right-1 bg-rose-600 text-white text-[8px] font-bold px-1 rounded">RAW</div>
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
                <button
                    onclick={handleCancelImport}
                    class="ml-1 w-5 h-5 rounded-full bg-white/10 hover:bg-red-500/50 flex items-center justify-center transition-colors"
                    title="Cancel import"
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
                <ImagePreview
                    src={previewPhoto.path}
                    alt={previewPhoto.path.split("/").pop() || "Preview"}
                    thumbPath={previewPhoto.thumb_path || ""}
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
                        <p class="theme-text-muted text-xs">Date</p>
                        <p class="theme-text-primary font-mono text-xs">{previewPhoto.metadata.date_taken}</p>
                    </div>
                {/if}

                <div>
                    <p class="theme-text-muted text-xs">Dimensions</p>
                    <p class="theme-text-primary">{previewPhoto.metadata.width} × {previewPhoto.metadata.height}</p>
                </div>

                <div>
                    <p class="theme-text-muted text-xs">File Size</p>
                    <p class="theme-text-primary">
                        {formatFileSize(previewPhoto.file_size)}
                        {#if previewPhoto.hasRaw}
                            {@const rawSize = getRawFileSize(previewPhoto)}
                            {#if rawSize > 0}
                                <span class="text-amber-500 ml-2">+ {formatFileSize(rawSize)} RAW</span>
                            {/if}
                        {/if}
                    </p>
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

                {#if previewPhoto.hasRaw || previewPhoto.isRawOnly}
                    <div>
                        <p class="theme-text-muted text-xs">RAW</p>
                        {#if previewPhoto.hasRaw}
                            <p class="text-amber-500 text-xs font-medium">JPEG + RAW</p>
                            <p class="theme-text-muted text-[10px] font-mono break-all mt-0.5">{previewPhoto.rawPath}</p>
                        {:else if previewPhoto.isRawOnly}
                            <p class="text-rose-500 text-xs font-medium">RAW Only</p>
                        {/if}
                    </div>
                {/if}

                <div>
                    <p class="theme-text-muted text-xs">Path</p>
                    <p class="theme-text-primary text-xs font-mono break-all">{previewPhoto.path}</p>
                </div>

                <!-- Delete actions -->
                <div class="pt-3 mt-3 border-t theme-border">
                    <p class="theme-text-muted text-xs mb-2">Delete</p>
                    <div class="flex flex-col gap-2">
                        <button
                            onclick={(e) => { e.stopPropagation(); openDeleteConfirm('app'); }}
                            disabled={isDeleting}
                            class="w-full px-3 py-2 rounded text-xs bg-amber-600/20 text-amber-400 hover:bg-amber-600/30 disabled:opacity-50 flex items-center gap-2"
                        >
                            <i class="fa-solid fa-database"></i>
                            Remove from Library
                        </button>
                        <button
                            onclick={(e) => { e.stopPropagation(); openDeleteConfirm('complete'); }}
                            disabled={isDeleting}
                            class="w-full px-3 py-2 rounded text-xs bg-red-600/20 text-red-400 hover:bg-red-600/30 disabled:opacity-50 flex items-center gap-2"
                        >
                            <i class="fa-solid fa-trash"></i>
                            Delete Original File
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
                            {deleteMode === 'complete' ? 'Delete Photo' : 'Remove from Library'}
                        </h3>
                        <p class="text-sm theme-text-muted">
                            {deleteMode === 'complete' ? 'This will permanently delete the original file' : 'Original file will be kept'}
                        </p>
                    </div>
                </div>

                <div class="theme-bg-primary rounded-lg p-3 mb-4">
                    <p class="text-xs theme-text-muted mb-1">File</p>
                    <p class="text-sm theme-text-primary font-mono break-all">{previewPhoto.path.split('/').pop()}</p>
                    {#if previewPhoto.hasRaw && deleteMode === 'complete'}
                        <div class="mt-3 pt-3 border-t border-white/10">
                            <p class="text-xs theme-text-muted mb-2">This photo has an associated RAW file:</p>
                            <p class="text-xs text-amber-400 font-mono break-all mb-3">{previewPhoto.rawPath?.split('/').pop()}</p>
                            <label class="flex items-center gap-2 cursor-pointer">
                                <input
                                    type="checkbox"
                                    bind:checked={deleteIncludeRaw}
                                    class="w-4 h-4 rounded border-2 border-amber-500 bg-transparent checked:bg-amber-500"
                                />
                                <span class="text-sm text-amber-400">Also delete RAW file</span>
                            </label>
                        </div>
                    {:else if previewPhoto.hasRaw}
                        <p class="text-xs text-amber-400 mt-1">+ Associated RAW file will also be removed</p>
                    {/if}
                </div>

                {#if deleteMode === 'complete'}
                    <div class="bg-red-500/10 border border-red-500/30 rounded-lg p-3 mb-4">
                        <p class="text-xs text-red-400">
                            <i class="fa-solid fa-triangle-exclamation mr-1"></i>
                            Warning: This action cannot be undone. The original file will be permanently deleted from disk.
                        </p>
                    </div>
                {/if}

                <div class="flex gap-3">
                    <button
                        onclick={() => deleteConfirmOpen = false}
                        disabled={isDeleting}
                        class="flex-1 px-4 py-2 rounded-lg theme-bg-tertiary theme-text-secondary hover:theme-text-primary disabled:opacity-50"
                    >
                        Cancel
                    </button>
                    <button
                        onclick={() => handleDeletePhoto(previewPhoto!, deleteMode, deleteIncludeRaw)}
                        disabled={isDeleting}
                        class="flex-1 px-4 py-2 rounded-lg {deleteMode === 'complete' ? 'bg-red-600 hover:bg-red-700' : 'bg-amber-600 hover:bg-amber-700'} text-white disabled:opacity-50 flex items-center justify-center gap-2"
                    >
                        {#if isDeleting}
                            <i class="fa-solid fa-spinner fa-spin"></i>
                        {/if}
                        {deleteMode === 'complete' ? 'Delete' : 'Remove'}
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
