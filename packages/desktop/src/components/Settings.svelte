<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { emit } from "@tauri-apps/api/event";
    import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

    let { dbPath, thumbDir, version } = $props<{
        dbPath: string;
        thumbDir: string;
        version: string;
    }>();

    let clearCacheLoading = $state(false);
    let regenerateLoading = $state(false);

    async function handleClearCache() {
        if (!thumbDir) return;
        clearCacheLoading = true;
        try {
            await invoke("clear_thumbnail_cache", { thumbDir });
            await emit("reload-photos");
            alert("Thumbnail cache cleared successfully!");
        } catch (e) {
            console.error(e);
            alert("Failed to clear cache: " + e);
        } finally {
            clearCacheLoading = false;
        }
    }

    async function handleRegenerate() {
        if (!dbPath || !thumbDir) return;
        regenerateLoading = true;
        try {
            await invoke("regenerate_thumbnails", { dbPath, thumbDir });
            await emit("reload-photos");
            alert("Thumbnails regenerated successfully!");
        } catch (e) {
            console.error(e);
            alert("Failed to regenerate thumbnails: " + e);
        } finally {
            regenerateLoading = false;
        }
    }

    async function handleOpenPath(
        path: string,
        mode: "open" | "reveal" = "open",
    ) {
        if (!path) return;
        try {
            if (mode === "reveal") {
                await revealItemInDir(path);
            } else {
                await openPath(path);
            }
        } catch (e) {
            console.error("Failed to open path:", path, e);
            alert(`Failed to open path: ${path}\nError: ${e}`);
        }
    }
</script>

<div
    class="flex flex-col gap-8 animate-in fade-in slide-in-from-bottom-4 duration-500"
>
    <!-- Header -->
    <div class="flex flex-col gap-2">
        <h2 class="text-3xl font-extrabold text-white">Settings</h2>
        <p class="text-slate-400">
            Configure your global preferences and storage.
        </p>
    </div>

    <!-- Storage Section -->
    <div
        class="rounded-2xl bg-slate-800/50 border border-slate-700/50 overflow-hidden"
    >
        <div class="p-6 border-b border-slate-700/50">
            <h3 class="text-lg font-bold text-white flex items-center gap-3">
                <i class="fa-solid fa-database text-indigo-400"></i>
                Storage & Database
            </h3>
        </div>
        <div class="p-6 space-y-6">
            <div class="flex flex-col gap-2">
                <label class="text-sm font-medium text-slate-400"
                    >Database Location</label
                >
                <div
                    class="flex items-center gap-3 p-3 rounded-xl bg-slate-900/50 border border-slate-700 font-mono text-xs text-slate-300"
                >
                    <i class="fa-solid fa-file-code text-slate-500"></i>
                    <span class="truncate flex-1">{dbPath || "Loading..."}</span
                    >
                    <button
                        onclick={() => handleOpenPath(dbPath, "reveal")}
                        class="px-2 py-1 rounded bg-slate-800 hover:bg-slate-700 text-xs text-indigo-400 border border-slate-700 transition"
                        title="Show in Finder"
                    >
                        <i class="fa-solid fa-folder-open"></i>
                    </button>
                </div>
            </div>

            <div class="flex flex-col gap-2">
                <label class="text-sm font-medium text-slate-400"
                    >Thumbnail Cache</label
                >
                <div
                    class="flex items-center gap-3 p-3 rounded-xl bg-slate-900/50 border border-slate-700 font-mono text-xs text-slate-300"
                >
                    <i class="fa-solid fa-images text-slate-500"></i>
                    <span class="truncate flex-1"
                        >{thumbDir || "Loading..."}</span
                    >
                    <button
                        onclick={() => handleOpenPath(thumbDir)}
                        class="px-2 py-1 rounded bg-slate-800 hover:bg-slate-700 text-xs text-indigo-400 border border-slate-700 transition"
                        title="Open in Finder"
                    >
                        <i class="fa-solid fa-folder-open"></i>
                    </button>
                </div>
            </div>

            <div class="pt-4 border-t border-slate-700/50 flex gap-4">
                <button
                    onclick={handleClearCache}
                    disabled={clearCacheLoading}
                    class="flex items-center gap-2 px-4 py-2 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 hover:bg-red-500/20 transition-colors text-sm font-medium disabled:opacity-50"
                >
                    {#if clearCacheLoading}
                        <i class="fa-solid fa-circle-notch fa-spin"></i>
                    {:else}
                        <i class="fa-solid fa-trash-can"></i>
                    {/if}
                    Clear Cache
                </button>

                <button
                    onclick={handleRegenerate}
                    disabled={regenerateLoading}
                    class="flex items-center gap-2 px-4 py-2 rounded-lg bg-indigo-500/10 text-indigo-400 border border-indigo-500/20 hover:bg-indigo-500/20 transition-colors text-sm font-medium disabled:opacity-50"
                >
                    {#if regenerateLoading}
                        <i class="fa-solid fa-circle-notch fa-spin"></i>
                    {:else}
                        <i class="fa-solid fa-arrows-rotate"></i>
                    {/if}
                    Regenerate All
                </button>
            </div>
        </div>
    </div>

    <!-- Application Section -->
    <div
        class="rounded-2xl bg-slate-800/50 border border-slate-700/50 overflow-hidden"
    >
        <div class="p-6 border-b border-slate-700/50">
            <h3 class="text-lg font-bold text-white flex items-center gap-3">
                <i class="fa-solid fa-cube text-indigo-400"></i>
                Application
            </h3>
        </div>
        <div class="p-6 grid grid-cols-2 gap-6">
            <div class="flex flex-col gap-1">
                <span class="text-sm font-medium text-slate-400"
                    >Core Engine Version</span
                >
                <span class="text-lg font-bold text-white tracking-tight"
                    >{version}</span
                >
            </div>
            <div class="flex flex-col gap-1">
                <span class="text-sm font-medium text-slate-400"
                    >UI Platform</span
                >
                <span class="text-lg font-bold text-white tracking-tight"
                    >Tauri v2 + Svelte 5</span
                >
            </div>
        </div>
    </div>
</div>
