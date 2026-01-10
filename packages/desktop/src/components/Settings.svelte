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

    async function handleClearCache() {
        if (!thumbDir || !dbPath) return;

        const confirmed = confirm("This will delete all imported photos and thumbnails. Map cache will be preserved.\n\nContinue?");
        if (!confirmed) return;

        clearCacheLoading = true;
        try {
            await invoke("clear_app_data", { thumbDir, dbPath });
            await emit("reload-photos");
        } catch (e) {
            alert("Failed to clear data: " + e);
        } finally {
            clearCacheLoading = false;
        }
    }

    async function handleOpenPath(path: string, mode: "open" | "reveal" = "open") {
        if (!path) return;
        try {
            if (mode === "reveal") {
                await revealItemInDir(path);
            } else {
                await openPath(path);
            }
        } catch (e) {
            alert(`Failed to open: ${e}`);
        }
    }
</script>

<div class="p-4 max-w-2xl">
    <h2 class="text-lg font-medium text-white mb-6">Settings</h2>

    <!-- Storage -->
    <section class="mb-6">
        <h3 class="text-sm text-neutral-400 mb-3">Storage</h3>

        <div class="space-y-3">
            <div class="flex items-center justify-between py-2">
                <div class="min-w-0 flex-1">
                    <p class="text-sm text-neutral-300">Database</p>
                    <p class="text-xs text-neutral-500 font-mono truncate">{dbPath || "..."}</p>
                </div>
                <button
                    onclick={() => handleOpenPath(dbPath, "reveal")}
                    class="ml-3 p-1.5 rounded hover:bg-neutral-800 text-neutral-400 hover:text-white text-xs"
                >
                    <i class="fa-solid fa-folder-open"></i>
                </button>
            </div>

            <div class="flex items-center justify-between py-2">
                <div class="min-w-0 flex-1">
                    <p class="text-sm text-neutral-300">Thumbnails</p>
                    <p class="text-xs text-neutral-500 font-mono truncate">{thumbDir || "..."}</p>
                </div>
                <button
                    onclick={() => handleOpenPath(thumbDir)}
                    class="ml-3 p-1.5 rounded hover:bg-neutral-800 text-neutral-400 hover:text-white text-xs"
                >
                    <i class="fa-solid fa-folder-open"></i>
                </button>
            </div>
        </div>
    </section>

    <!-- Cache -->
    <section class="mb-6">
        <h3 class="text-sm text-neutral-400 mb-3">Cache</h3>

        <div class="flex gap-2">
            <button
                onclick={handleClearCache}
                disabled={clearCacheLoading}
                class="px-3 py-1.5 rounded bg-neutral-800 border border-neutral-700 text-sm text-neutral-300 hover:bg-neutral-700 disabled:opacity-50"
            >
                {#if clearCacheLoading}
                    <i class="fa-solid fa-spinner fa-spin mr-1"></i>
                {/if}
                Clear Cache
            </button>
        </div>
    </section>

    <!-- About -->
    <section>
        <h3 class="text-sm text-neutral-400 mb-3">About</h3>

        <div class="text-sm">
            <p class="text-neutral-300">Fotos <span class="text-neutral-500">v{version}</span></p>
            <p class="text-neutral-500 text-xs mt-1">Tauri + Svelte + Rust</p>
        </div>
    </section>
</div>
