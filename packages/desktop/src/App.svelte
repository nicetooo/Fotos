<script lang="ts">
    import { onMount } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
    import { open } from "@tauri-apps/plugin-dialog";
    import { revealItemInDir } from "@tauri-apps/plugin-opener";
    import { appDataDir, join } from "@tauri-apps/api/path";
    import { listen } from "@tauri-apps/api/event";
    import Settings from "./components/Settings.svelte";
    import VirtualPhotoGrid from "./components/VirtualPhotoGrid.svelte";
    import ImagePreview from "./components/ImagePreview.svelte";
    import Map from "./components/Map.svelte";
    import type { PhotoInfo } from "./types";

    let version = $state("...");
    let currentView = $state("library");
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
    let sortBy = $state<"name" | "date" | "size" | "dimensions">("date");
    let sortOrder = $state<"asc" | "desc">("desc");
    let importMenuOpen = $state(false);

    // Grid thumbnail size (pinch to zoom)
    let thumbSize = $state(200);

    let sortedPhotos = $derived.by(() => {
        const photosCopy = [...photos];
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

    function openPreview(photo: PhotoInfo) {
        previewPhoto = photo;
    }

    function closePreview() {
        previewPhoto = null;
    }

    function handleGridWheel(e: WheelEvent) {
        if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            const delta = e.deltaY > 0 ? -20 : 20;
            thumbSize = Math.max(100, Math.min(400, thumbSize + delta));
        }
    }

    function navigatePreview(direction: "prev" | "next") {
        if (!previewPhoto) return;
        const currentIndex = sortedPhotos.findIndex(p => p.path === previewPhoto!.path);
        if (currentIndex === -1) return;

        let newIndex: number;
        if (direction === "prev") {
            newIndex = currentIndex > 0 ? currentIndex - 1 : sortedPhotos.length - 1;
        } else {
            newIndex = currentIndex < sortedPhotos.length - 1 ? currentIndex + 1 : 0;
        }
        previewPhoto = sortedPhotos[newIndex];
    }

    function handleKeydown(e: KeyboardEvent) {
        // Preview navigation (zoom handled by ImagePreview component)
        if (previewPhoto) {
            switch (e.key) {
                case "Escape":
                    closePreview();
                    break;
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

<main class="fixed inset-0 flex bg-neutral-900 text-neutral-200 overflow-hidden">
    <!-- Sidebar -->
    <aside class="w-48 shrink-0 border-r border-neutral-800 bg-neutral-900 flex flex-col py-3 h-full">
        <div class="px-4 py-2 mb-2">
            <h1 class="text-sm font-semibold text-neutral-300">Fotos</h1>
        </div>

        <nav class="flex-1 px-2 space-y-0.5">
            <button
                onclick={() => (currentView = "library")}
                class="w-full flex items-center gap-2 px-3 py-1.5 rounded text-sm {currentView === 'library'
                    ? 'bg-neutral-800 text-white'
                    : 'text-neutral-400 hover:bg-neutral-800/50 hover:text-neutral-200'}"
            >
                <i class="fa-solid fa-images w-4 text-center text-xs"></i>
                <span>Library</span>
            </button>
            <button
                onclick={() => (currentView = "map")}
                class="w-full flex items-center gap-2 px-3 py-1.5 rounded text-sm {currentView === 'map'
                    ? 'bg-neutral-800 text-white'
                    : 'text-neutral-400 hover:bg-neutral-800/50 hover:text-neutral-200'}"
            >
                <i class="fa-solid fa-map w-4 text-center text-xs"></i>
                <span>Map</span>
            </button>
            <button
                onclick={() => (currentView = "settings")}
                class="w-full flex items-center gap-2 px-3 py-1.5 rounded text-sm {currentView === 'settings'
                    ? 'bg-neutral-800 text-white'
                    : 'text-neutral-400 hover:bg-neutral-800/50 hover:text-neutral-200'}"
            >
                <i class="fa-solid fa-gear w-4 text-center text-xs"></i>
                <span>Settings</span>
            </button>
        </nav>

        <div class="px-4 py-2 text-xs text-neutral-600">
            v{version}
        </div>
    </aside>

    <!-- Main Content -->
    <section class="flex-1 flex flex-col min-w-0 h-full overflow-hidden {currentView === 'map' ? '' : 'p-4'}">
        {#if currentView === "library"}
            <!-- Toolbar -->
            <header class="flex justify-between items-center mb-4 shrink-0">
                <div class="flex items-center gap-4">
                    <h2 class="text-lg font-medium text-white">{photos.length} Photos</h2>
                </div>

                <div class="flex items-center gap-2">
                    <select
                        bind:value={sortBy}
                        class="bg-neutral-800 text-neutral-300 text-sm px-2 py-1 rounded border border-neutral-700 focus:outline-none focus:border-neutral-600"
                    >
                        <option value="date">Date</option>
                        <option value="name">Name</option>
                        <option value="dimensions">Size</option>
                    </select>

                    <button
                        onclick={() => (sortOrder = sortOrder === "asc" ? "desc" : "asc")}
                        class="p-1.5 rounded bg-neutral-800 border border-neutral-700 text-neutral-400 hover:text-white"
                        title={sortOrder === "asc" ? "Ascending" : "Descending"}
                    >
                        <i class="fa-solid {sortOrder === 'asc' ? 'fa-arrow-up' : 'fa-arrow-down'} text-xs"></i>
                    </button>

                    <div class="relative">
                        <button
                            onclick={() => importMenuOpen = !importMenuOpen}
                            disabled={isScanning}
                            class="flex items-center gap-2 px-3 py-1.5 rounded bg-neutral-800 border border-neutral-700 text-sm text-neutral-300 hover:bg-neutral-700 hover:text-white disabled:opacity-50"
                        >
                            <i class="fa-solid {isScanning ? 'fa-spinner fa-spin' : 'fa-plus'} text-xs"></i>
                            <span>{isScanning ? "Importing..." : "Import"}</span>
                            <i class="fa-solid fa-caret-down text-xs text-neutral-500"></i>
                        </button>
                        {#if importMenuOpen}
                            <!-- svelte-ignore a11y_no_static_element_interactions -->
                            <div
                                class="fixed inset-0 z-40"
                                onclick={() => importMenuOpen = false}
                            ></div>
                            <div class="absolute right-0 top-full mt-1 py-1 bg-neutral-800 border border-neutral-700 rounded shadow-lg z-50 min-w-[140px]">
                                <button
                                    onclick={() => { importMenuOpen = false; handleScan("folder"); }}
                                    disabled={isScanning}
                                    class="w-full px-3 py-1.5 text-left text-sm text-neutral-300 hover:bg-neutral-700 flex items-center gap-2"
                                >
                                    <i class="fa-solid fa-folder text-xs"></i>
                                    Folder
                                </button>
                                <button
                                    onclick={() => { importMenuOpen = false; handleScan("file"); }}
                                    disabled={isScanning}
                                    class="w-full px-3 py-1.5 text-left text-sm text-neutral-300 hover:bg-neutral-700 flex items-center gap-2"
                                >
                                    <i class="fa-solid fa-file-image text-xs"></i>
                                    Single File
                                </button>
                            </div>
                        {/if}
                    </div>
                </div>
            </header>

            {#if error}
                <div class="mb-3 px-3 py-2 rounded bg-red-900/30 border border-red-800/50 text-red-400 text-sm shrink-0">
                    {error}
                </div>
            {/if}

            <!-- Photo Grid -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="flex-1 min-h-0 flex flex-col" onwheel={handleGridWheel}>
                {#if photos.length > 0}
                    <VirtualPhotoGrid
                        photos={sortedPhotos}
                        {uniqueTs}
                        {thumbSize}
                        onPhotoClick={openPreview}
                    />
                {:else if !isScanning}
                    <div class="h-full flex flex-col items-center justify-center text-neutral-500">
                        <i class="fa-solid fa-images text-3xl mb-3"></i>
                        <p class="text-sm">No photos. Click Import to add.</p>
                    </div>
                {:else}
                    <div class="h-full flex flex-col items-center justify-center text-neutral-500">
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

            <!-- Import Progress -->
            {#if isScanning && importStatus.success > 0}
                <div class="absolute bottom-4 right-4 px-3 py-2 rounded bg-neutral-800 border border-neutral-700 text-sm">
                    <span class="text-green-400">{importStatus.success}</span>
                    {#if importStatus.failure > 0}
                        / <span class="text-red-400">{importStatus.failure} failed</span>
                    {/if}
                </div>
            {/if}
        {:else if currentView === "settings"}
            <Settings {dbPath} {thumbDir} {version} />
        {:else if currentView === "map"}
            <Map {photos} />
        {/if}
    </section>
</main>

{#if previewPhoto}
    {@const currentIndex = sortedPhotos.findIndex(p => p.path === previewPhoto.path)}
    <div
        class="fixed inset-0 z-50 bg-black/95"
        onclick={closePreview}
    >
        <!-- Top bar -->
        <div class="absolute top-0 left-0 right-64 h-12 flex items-center justify-between px-4 z-20">
            <div class="flex items-center gap-3">
                <span class="text-xs text-neutral-500">{currentIndex + 1} / {sortedPhotos.length}</span>
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
