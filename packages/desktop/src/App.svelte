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

<main class="fixed inset-0 flex flex-col bg-neutral-900 text-neutral-200 overflow-hidden">
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
                    class="w-10 h-10 rounded-full bg-black/70 backdrop-blur-sm border border-white/20 text-white/80 hover:bg-black/90 hover:text-white disabled:opacity-50 flex items-center justify-center shadow-lg transition-all"
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
                    <div class="absolute left-0 top-full mt-2 py-1 bg-black/90 backdrop-blur-sm border border-white/20 rounded-lg shadow-lg z-50 min-w-[140px]">
                        <button
                            onclick={() => { importMenuOpen = false; handleScan("folder"); }}
                            disabled={isScanning}
                            class="w-full px-3 py-2 text-left text-sm text-white/80 hover:bg-white/10 flex items-center gap-2"
                        >
                            <i class="fa-solid fa-folder text-xs"></i>
                            Import Folder
                        </button>
                        <button
                            onclick={() => { importMenuOpen = false; handleScan("file"); }}
                            disabled={isScanning}
                            class="w-full px-3 py-2 text-left text-sm text-white/80 hover:bg-white/10 flex items-center gap-2"
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
                class="w-10 h-10 rounded-full bg-black/70 backdrop-blur-sm border border-white/20 text-white/80 hover:bg-black/90 hover:text-white flex items-center justify-center shadow-lg transition-all {showLibrary ? 'bg-white/20 text-white' : ''}"
                title="Library"
            >
                <i class="fa-solid fa-images"></i>
            </button>

            <!-- Settings button -->
            <button
                onclick={() => showSettings = true}
                class="w-10 h-10 rounded-full bg-black/70 backdrop-blur-sm border border-white/20 text-white/80 hover:bg-black/90 hover:text-white flex items-center justify-center shadow-lg transition-all"
                title="Settings"
            >
                <i class="fa-solid fa-gear"></i>
            </button>
        </div>

        <!-- Library Drawer (right side) -->
        {#if showLibrary}
            <div class="absolute top-0 right-0 bottom-0 w-80 bg-black/95 backdrop-blur-md border-l border-white/10 z-[1001] flex flex-col">
                <!-- Header -->
                <div class="flex items-center justify-between px-4 py-3 border-b border-white/10">
                    <div class="flex items-center gap-2">
                        <i class="fa-solid fa-images text-white/60 text-sm"></i>
                        <span class="text-white font-medium">
                            {sortedPhotos.length} Photos
                            {#if sortedPhotos.length !== photos.length}
                                <span class="text-white/50 text-xs">({photos.length} files)</span>
                            {/if}
                        </span>
                    </div>
                    <button
                        onclick={() => showLibrary = false}
                        class="p-1.5 rounded-lg hover:bg-white/10 text-white/60 hover:text-white transition-colors"
                        title="Close"
                    >
                        <i class="fa-solid fa-xmark"></i>
                    </button>
                </div>

                <!-- Toolbar -->
                <div class="flex items-center gap-2 px-3 py-2 border-b border-white/10">
                    <select
                        bind:value={sortBy}
                        class="flex-1 bg-white/10 text-white/80 text-xs px-2 py-1.5 rounded border border-white/10 focus:outline-none focus:border-white/30"
                    >
                        <option value="date">Date</option>
                        <option value="name">Name</option>
                        <option value="dimensions">Size</option>
                    </select>

                    <button
                        onclick={() => (sortOrder = sortOrder === "asc" ? "desc" : "asc")}
                        class="p-1.5 rounded bg-white/10 border border-white/10 text-white/60 hover:text-white hover:bg-white/20"
                        title={sortOrder === "asc" ? "Ascending" : "Descending"}
                    >
                        <i class="fa-solid {sortOrder === 'asc' ? 'fa-arrow-up' : 'fa-arrow-down'} text-xs"></i>
                    </button>

                    <button
                        onclick={() => { showLibrary = false; handleScan("folder"); }}
                        disabled={isScanning}
                        class="p-1.5 rounded bg-white/10 border border-white/10 text-white/60 hover:text-white hover:bg-white/20 disabled:opacity-50"
                        title="Import folder"
                    >
                        <i class="fa-solid {isScanning ? 'fa-spinner fa-spin' : 'fa-folder-plus'} text-xs"></i>
                    </button>
                </div>

                <!-- Photo Grid -->
                <div class="flex-1 overflow-y-auto p-2">
                    {#if sortedPhotos.length > 0}
                        <div class="grid grid-cols-3 gap-1">
                            {#each sortedPhotos as photo (photo.path)}
                                <button
                                    class="aspect-square relative overflow-hidden rounded bg-neutral-800 hover:ring-2 hover:ring-white/50 transition-all group"
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
        class="fixed inset-0 z-[2000] bg-black/80 backdrop-blur-sm flex items-center justify-center"
        onclick={() => showSettings = false}
    >
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
            class="bg-neutral-900 border border-neutral-700 rounded-2xl shadow-2xl w-full max-w-lg max-h-[80vh] overflow-hidden"
            onclick={(e) => e.stopPropagation()}
        >
            <div class="flex items-center justify-between px-6 py-4 border-b border-neutral-800">
                <h2 class="text-lg font-medium text-white">Settings</h2>
                <button
                    onclick={() => showSettings = false}
                    class="p-2 rounded-lg hover:bg-neutral-800 text-neutral-400 hover:text-white transition-colors"
                >
                    <i class="fa-solid fa-xmark"></i>
                </button>
            </div>
            <div class="p-6 overflow-y-auto max-h-[calc(80vh-60px)]">
                <Settings {dbPath} {thumbDir} {version} />
            </div>
        </div>
    </div>
{/if}

{#if previewPhoto}
    {@const currentIndex = navigationPhotos.findIndex(p => p.path === previewPhoto.path)}
    <div
        class="fixed inset-0 z-50 bg-black/95"
        onclick={closePreview}
    >
        <!-- Top bar -->
        <div class="absolute top-0 left-0 right-64 h-12 flex items-center justify-between px-4 z-20">
            <div class="flex items-center gap-3">
                <span class="text-xs text-neutral-500">{currentIndex + 1} / {navigationPhotos.length}</span>
                <span class="text-sm text-neutral-400 truncate">
                    {previewPhoto.path.split("/").pop()}
                </span>
            </div>
            <div class="flex items-center gap-2">
                <button
                    onclick={(e) => handleShowInFinder(previewPhoto!.path, e)}
                    class="p-2 rounded hover:bg-white/10 text-neutral-400 hover:text-white"
                    title="Show in Finder"
                >
                    <i class="fa-solid fa-folder-open text-sm"></i>
                </button>
                <button
                    onclick={closePreview}
                    class="p-2 rounded hover:bg-white/10 text-neutral-400 hover:text-white"
                >
                    <i class="fa-solid fa-xmark text-sm"></i>
                </button>
            </div>
        </div>

        <!-- Navigation buttons -->
        <button
            onclick={(e) => { e.stopPropagation(); navigatePreview("prev"); }}
            class="absolute left-4 top-1/2 -translate-y-1/2 p-3 rounded-full bg-black/50 hover:bg-black/70 text-white/70 hover:text-white z-20"
            title="Previous (←)"
        >
            <i class="fa-solid fa-chevron-left"></i>
        </button>
        <button
            onclick={(e) => { e.stopPropagation(); navigatePreview("next"); }}
            class="absolute right-72 top-1/2 -translate-y-1/2 p-3 rounded-full bg-black/50 hover:bg-black/70 text-white/70 hover:text-white z-20"
            title="Next (→)"
        >
            <i class="fa-solid fa-chevron-right"></i>
        </button>

        <!-- Image area -->
        <div
            class="absolute top-12 left-0 right-64 bottom-0"
            onclick={(e) => e.stopPropagation()}
        >
            {#key previewPhoto.path}
                <ImagePreview src={previewPhoto.path} alt={previewPhoto.path.split("/").pop() || "Preview"} />
            {/key}
        </div>

        <!-- Info Panel -->
        <div
            class="absolute top-0 right-0 bottom-0 w-64 bg-neutral-900 border-l border-neutral-800 pt-12 px-4 py-4 overflow-y-auto text-sm"
            onclick={(e) => e.stopPropagation()}
        >
            <div class="space-y-3">
                {#if previewPhoto.metadata.date_taken}
                    <div>
                        <p class="text-neutral-500 text-xs">Date</p>
                        <p class="text-neutral-200 font-mono text-xs">{previewPhoto.metadata.date_taken}</p>
                    </div>
                {/if}

                <div>
                    <p class="text-neutral-500 text-xs">Dimensions</p>
                    <p class="text-neutral-200">{previewPhoto.metadata.width} × {previewPhoto.metadata.height}</p>
                </div>

                {#if previewPhoto.metadata.make || previewPhoto.metadata.model}
                    <div>
                        <p class="text-neutral-500 text-xs">Camera</p>
                        <p class="text-neutral-200">{previewPhoto.metadata.make || ""} {previewPhoto.metadata.model || ""}</p>
                    </div>
                {/if}

                {#if previewPhoto.metadata.iso || previewPhoto.metadata.f_number || previewPhoto.metadata.exposure_time}
                    <div>
                        <p class="text-neutral-500 text-xs">Exposure</p>
                        <p class="text-neutral-200">
                            {#if previewPhoto.metadata.iso}ISO {previewPhoto.metadata.iso}{/if}
                            {#if previewPhoto.metadata.f_number} f/{previewPhoto.metadata.f_number.toFixed(1)}{/if}
                            {#if previewPhoto.metadata.exposure_time} {previewPhoto.metadata.exposure_time}{/if}
                        </p>
                    </div>
                {/if}

                {#if previewPhoto.metadata.lat && previewPhoto.metadata.lon}
                    <div>
                        <p class="text-neutral-500 text-xs">GPS</p>
                        <p class="text-neutral-200 font-mono text-xs">
                            {previewPhoto.metadata.lat.toFixed(6)}, {previewPhoto.metadata.lon.toFixed(6)}
                        </p>
                    </div>
                {/if}

                <div>
                    <p class="text-neutral-500 text-xs">Path</p>
                    <p class="text-neutral-200 text-xs font-mono break-all">{previewPhoto.path}</p>
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
