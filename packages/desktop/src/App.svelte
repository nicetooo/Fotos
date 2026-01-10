<script lang="ts">
    import { onMount } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
    import { open } from "@tauri-apps/plugin-dialog";
    import { revealItemInDir } from "@tauri-apps/plugin-opener";
    import { appDataDir, join } from "@tauri-apps/api/path";
    import { listen } from "@tauri-apps/api/event";
    import Settings from "./components/Settings.svelte";
    import ThumbnailImage from "./components/ThumbnailImage.svelte";
    import Map from "./components/Map.svelte";

    interface PhotoId {
        id: number;
    }
    interface PhotoMetadata {
        width: number;
        height: number;
        date_taken?: string;
        iso?: number;
        f_number?: number;
        exposure_time?: string;
        make?: string;
        model?: string;
        lat?: number;
        lon?: number;
    }
    interface PhotoInfo {
        id: PhotoId;
        path: string;
        hash: string;
        metadata: PhotoMetadata;
        thumb_path?: string;
    }

    let version = $state("Loading...");
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

    onMount(async () => {
        try {
            version = await invoke("get_core_version");
            const appData = await appDataDir();
            dbPath = await join(appData, "fotos.db");
            thumbDir = await join(appData, "thumbnails");

            // Listen for progress
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

                // Refresh occasionally for live update
                if (payload.current <= 5 || payload.current % 50 === 0) {
                    loadPhotos();
                }
            });

            await listen("reload-photos", () => {
                console.log("Reloading photos due to cache clear...");
                loadPhotos();
            });

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

    async function handleScan() {
        try {
            console.log("Opening folder picker...");
            const selected = await open({
                directory: true,
                multiple: false,
            });

            console.log("Picker selected:", selected);
            if (!selected) return;

            // In case it returns an array even with multiple: false
            const rootPath = Array.isArray(selected) ? selected[0] : selected;

            isScanning = true;
            error = "";

            console.log("Calling import_photos with:", {
                rootPath,
                dbPath,
                thumbDir,
            });
            const result = await invoke("import_photos", {
                rootPath,
                dbPath: dbPath,
                thumbDir: thumbDir,
            });
            console.log("Import result:", result);
            importStatus = result as any;
            await loadPhotos();
        } catch (e) {
            console.error("Scan error:", e);
            error = String(e);
        } finally {
            isScanning = false;
        }
    }

    async function handleShowInFinder(path: string, e: MouseEvent) {
        e.stopPropagation(); // Prevent opening preview
        try {
            await revealItemInDir(path);
        } catch (e) {
            console.error("Failed to reveal item:", e);
            alert("Failed to open location: " + e);
        }
    }

    function openPreview(photo: PhotoInfo) {
        previewPhoto = photo;
    }

    function closePreview() {
        previewPhoto = null;
    }

    function formatDate(dateStr?: string): string {
        if (!dateStr) return "";
        try {
            // EXIF date format is usually "YYYY:MM:DD HH:MM:SS"
            // If it's standard ISO, Date parse works. If it's EXIF, we might need manual parsing or just display.
            // Let's try simple display first, or replace : with - for the date part.
            // Many parsers handle it, but to be safe:
            const standardized = dateStr.replace(
                /^(\d{4}):(\d{2}):(\d{2})/,
                "$1-$2-$3",
            );
            const date = new Date(standardized);
            if (isNaN(date.getTime())) return dateStr;
            return date.toLocaleDateString(undefined, {
                year: "numeric",
                month: "short",
                day: "numeric",
            });
        } catch {
            return dateStr;
        }
    }
</script>

<main
    class="fixed inset-0 flex bg-[#0f172a] text-slate-200 overflow-hidden font-sans"
>
    <!-- Sidebar -->
    <aside
        class="w-64 shrink-0 border-r border-slate-800 bg-[#0f172a]/50 backdrop-blur-xl flex flex-col p-6 gap-8 h-full overflow-y-auto"
    >
        <div class="flex items-center gap-3 px-2">
            <div
                class="w-8 h-8 rounded-xl bg-gradient-to-tr from-indigo-500 to-purple-500 flex items-center justify-center shadow-lg shadow-indigo-500/20"
            >
                <i class="fa-solid fa-camera text-white text-sm"></i>
            </div>
            <h1 class="text-xl font-bold tracking-tight text-white">Fotos</h1>
        </div>

        <nav class="flex-1 space-y-2">
            <button
                onclick={() => (currentView = "library")}
                class="w-full flex items-center gap-3 px-4 py-3 rounded-xl transition-all group {currentView ===
                'library'
                    ? 'bg-white/10 text-white'
                    : 'text-slate-400 hover:bg-white/5 hover:text-white'}"
            >
                <i
                    class="fa-solid fa-images text-indigo-400 group-hover:scale-110 transition-transform"
                ></i>
                <span class="font-medium">Library</span>
            </button>
            <button
                onclick={() => (currentView = "map")}
                class="w-full flex items-center gap-3 px-4 py-3 rounded-xl transition-all group {currentView ===
                'map'
                    ? 'bg-white/10 text-white'
                    : 'text-slate-400 hover:bg-white/5 hover:text-white'}"
            >
                <i
                    class="fa-solid fa-map-location-dot group-hover:scale-110 transition-transform"
                ></i>
                <span class="font-medium">Map</span>
            </button>
            <button
                onclick={() => (currentView = "settings")}
                class="w-full flex items-center gap-3 px-4 py-3 rounded-xl transition-all group {currentView ===
                'settings'
                    ? 'bg-white/10 text-white'
                    : 'text-slate-400 hover:bg-white/5 hover:text-white'}"
            >
                <i
                    class="fa-solid fa-gear group-hover:rotate-45 transition-transform"
                ></i>
                <span class="font-medium">Settings</span>
            </button>
        </nav>

        <div
            class="mt-auto p-4 rounded-2xl bg-gradient-to-br from-indigo-500/10 to-purple-500/10 border border-indigo-500/20"
        >
            <div class="flex items-center gap-2 mb-2 text-indigo-300">
                <i class="fa-solid fa-circle-info text-[10px]"></i>
                <span class="text-xs font-semibold uppercase tracking-wider"
                    >Engine Info</span
                >
            </div>
            <p class="text-sm text-slate-400">
                Version: <span class="text-white font-mono">{version}</span>
            </p>
        </div>
    </aside>

    <!-- Main Content -->
    <section
        class="flex-1 flex flex-col min-w-0 h-full overflow-hidden relative {currentView ===
        'map'
            ? 'p-0'
            : 'p-8'}"
    >
        {#if currentView === "library"}
            <!-- Header -->
            <header class="flex justify-between items-end mb-8 shrink-0">
                <div>
                    <h2 class="text-3xl font-extrabold text-white mb-2">
                        My Gallery
                    </h2>
                    <p class="text-slate-400">
                        Manage your memories with high-precision Rust engine.
                    </p>
                </div>

                <button
                    onclick={handleScan}
                    disabled={isScanning}
                    class="flex items-center gap-3 px-6 py-3 rounded-2xl bg-indigo-600 hover:bg-indigo-500 active:scale-95 text-white font-semibold transition-all shadow-lg shadow-indigo-600/20 disabled:opacity-50 disabled:cursor-not-allowed group"
                >
                    <i
                        class="fa-solid fa-folder-open text-lg {isScanning
                            ? 'animate-pulse'
                            : 'group-hover:-rotate-12 transition-transform'}"
                    ></i>
                    <span>{isScanning ? "Scanning..." : "Import Photos"}</span>
                </button>
            </header>

            {#if error}
                <div
                    class="mb-6 p-4 rounded-2xl bg-red-500/10 border border-red-500/20 text-red-400 text-sm flex items-start gap-3 shrink-0"
                >
                    <i class="fa-solid fa-circle-info shrink-0 mt-0.5"></i>
                    <p>{error}</p>
                </div>
            {/if}

            <!-- Photo Grid -->
            <div class="flex-1 overflow-y-auto pr-2 custom-scrollbar">
                {#if photos.length > 0}
                    <div
                        class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 gap-4"
                    >
                        {#each photos as photo (photo.id.id)}
                            <div
                                class="aspect-square rounded-2xl bg-slate-800 overflow-hidden group relative border border-slate-700/50 hover:border-indigo-500/50 transition-all shadow-lg cursor-pointer"
                                onclick={() => openPreview(photo)}
                            >
                                <ThumbnailImage
                                    path={photo.thumb_path || photo.path}
                                    refreshKey={uniqueTs}
                                    alt="Photo thumbnail"
                                    className="w-full h-full object-cover group-hover:scale-110 transition-transform duration-500"
                                />
                                <div
                                    class="absolute inset-0 bg-gradient-to-t from-black/80 via-black/20 to-transparent opacity-0 group-hover:opacity-100 transition-opacity p-3 flex flex-col justify-between"
                                >
                                    <div class="flex justify-end">
                                        <button
                                            onclick={(e) =>
                                                handleShowInFinder(
                                                    photo.path,
                                                    e,
                                                )}
                                            class="p-1.5 rounded-full bg-black/40 hover:bg-black/60 text-white/80 hover:text-white transition-colors backdrop-blur-sm"
                                            title="Show in Finder"
                                        >
                                            <i
                                                class="fa-solid fa-folder-open text-[10px]"
                                            ></i>
                                        </button>
                                    </div>

                                    <div class="flex flex-col gap-0.5">
                                        <p
                                            class="text-[10px] text-white font-bold truncate"
                                        >
                                            {photo.path.split("/").pop()}
                                        </p>

                                        <div
                                            class="flex items-center gap-2 text-[9px] text-slate-300 font-mono opacity-90"
                                        >
                                            <span>
                                                {photo.metadata.width}x{photo
                                                    .metadata.height}
                                            </span>
                                            <span class="uppercase">
                                                {photo.path.split(".").pop()}
                                            </span>
                                        </div>

                                        {#if photo.metadata.date_taken}
                                            <div
                                                class="flex items-center gap-1 text-[9px] text-indigo-300"
                                            >
                                                <i
                                                    class="fa-regular fa-calendar text-[8px]"
                                                ></i>
                                                <span
                                                    >{formatDate(
                                                        photo.metadata
                                                            .date_taken,
                                                    )}</span
                                                >
                                            </div>
                                        {/if}

                                        {#if photo.metadata.iso || photo.metadata.f_number}
                                            <div
                                                class="flex items-center gap-2 text-[8px] text-slate-400 mt-0.5"
                                            >
                                                {#if photo.metadata.iso}
                                                    <span
                                                        class="bg-slate-700/50 px-1 rounded"
                                                        >ISO {photo.metadata
                                                            .iso}</span
                                                    >
                                                {/if}
                                                {#if photo.metadata.f_number}
                                                    <span
                                                        >ƒ/{photo.metadata
                                                            .f_number}</span
                                                    >
                                                {/if}
                                            </div>
                                        {/if}
                                    </div>
                                </div>
                            </div>
                        {/each}
                    </div>
                {:else if !isScanning}
                    <div
                        class="h-full flex flex-col items-center justify-center p-12 text-center"
                    >
                        <div
                            class="w-20 h-20 rounded-full bg-slate-800 flex items-center justify-center mb-6 text-slate-600"
                        >
                            <i class="fa-solid fa-images text-4xl"></i>
                        </div>
                        <h3 class="text-xl font-bold text-slate-300 mb-2">
                            No photos imported yet
                        </h3>
                        <p class="text-slate-500 max-w-xs">
                            Start by importing a folder to see your collection
                            organized by the core engine.
                        </p>
                    </div>
                {:else}
                    <div
                        class="h-full flex flex-col items-center justify-center p-12 text-center"
                    >
                        <i
                            class="fa-solid fa-circle-notch fa-spin text-4xl text-indigo-500 mb-4"
                        ></i>
                        <p class="text-slate-400">Analyzing your photos...</p>
                        {#if importStatus.total > 0}
                            <p class="text-xs text-slate-500 mt-2">
                                Processed {importStatus.current} of {importStatus.total}
                            </p>
                        {/if}
                    </div>
                {/if}
            </div>

            <!-- Scanning Overlay / Progress -->
            {#if isScanning && importStatus.success > 0}
                <div
                    class="absolute bottom-12 right-12 p-6 rounded-3xl bg-slate-900/90 backdrop-blur-xl border border-indigo-500/30 shadow-2xl flex items-center gap-6 animate-in fade-in slide-in-from-bottom-4"
                >
                    <div class="flex flex-col">
                        <span
                            class="text-xs font-bold text-indigo-400 uppercase tracking-widest mb-1"
                            >Indexing Status</span
                        >
                        <div class="flex gap-4">
                            <div class="flex items-baseline gap-1">
                                <span class="text-2xl font-black text-white"
                                    >{importStatus.success}</span
                                >
                                <span
                                    class="text-[10px] text-slate-500 font-bold uppercase"
                                    >Imported</span
                                >
                            </div>
                            {#if importStatus.failure > 0}
                                <div class="flex items-baseline gap-1">
                                    <span
                                        class="text-2xl font-black text-red-500"
                                        >{importStatus.failure}</span
                                    >
                                    <span
                                        class="text-[10px] text-slate-500 font-bold uppercase"
                                        >Skipped</span
                                    >
                                </div>
                            {/if}
                        </div>
                    </div>
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
    <!-- Photo Preview Overlay -->
    <div
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/95 backdrop-blur-sm animate-in fade-in duration-200"
        onclick={closePreview}
    >
        <div class="absolute top-6 right-6 flex items-center gap-4 z-10">
            <button
                onclick={(e) => handleShowInFinder(previewPhoto!.path, e)}
                class="p-3 rounded-full bg-white/10 hover:bg-white/20 text-white transition-colors backdrop-blur-md"
                title="Show in Finder"
            >
                <i class="fa-solid fa-folder-open text-lg"></i>
            </button>
            <button
                onclick={closePreview}
                class="p-3 rounded-full bg-white/10 hover:bg-white/20 text-white transition-colors backdrop-blur-md"
            >
                <i class="fa-solid fa-xmark text-lg"></i>
            </button>
        </div>

        <div
            class="relative max-w-[95vw] max-h-[95vh] rounded-lg overflow-hidden shadow-2xl"
            onclick={(e) => e.stopPropagation()}
        >
            <ThumbnailImage
                path={previewPhoto.path}
                alt="Full preview"
                className="max-w-full max-h-[90vh] object-contain"
            />

            <div
                class="absolute bottom-0 left-0 right-0 p-6 bg-gradient-to-t from-black/80 to-transparent text-white opacity-0 hover:opacity-100 transition-opacity"
            >
                <p class="font-mono text-sm truncate">
                    {previewPhoto.path.split("/").pop()}
                </p>
                <p class="text-xs text-slate-400 mt-1">
                    {previewPhoto.metadata.width} x {previewPhoto.metadata
                        .height}
                </p>
            </div>
        </div>
    </div>
{/if}

<style>
    :global(body) {
        overflow: hidden;
    }

    .custom-scrollbar::-webkit-scrollbar {
        width: 6px;
    }
    .custom-scrollbar::-webkit-scrollbar-track {
        background: transparent;
    }
    .custom-scrollbar::-webkit-scrollbar-thumb {
        background: #1e293b;
        border-radius: 10px;
    }
    .custom-scrollbar::-webkit-scrollbar-thumb:hover {
        background: #334155;
    }
</style>
