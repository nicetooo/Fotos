<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { emit } from "@tauri-apps/api/event";
    import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

    type Theme = "dark" | "light" | "system";

    let { dbPath, thumbDir, version, theme, onThemeChange } = $props<{
        dbPath: string;
        thumbDir: string;
        version: string;
        theme: Theme;
        onThemeChange: (theme: Theme) => void;
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

<div class="max-w-2xl">
    <!-- Appearance -->
    <section class="mb-6">
        <h3 class="text-sm theme-text-muted mb-3">Appearance</h3>

        <div class="flex items-center gap-2">
            <button
                onclick={() => onThemeChange("dark")}
                class="flex-1 flex items-center justify-center gap-2 px-3 py-2 rounded-lg border transition-all
                    {theme === 'dark'
                        ? 'bg-[var(--accent)]/20 border-[var(--accent)] text-[var(--accent)]'
                        : 'theme-bg-tertiary theme-border theme-text-muted hover:theme-text-primary'}"
            >
                <i class="fa-solid fa-moon"></i>
                <span class="text-sm">Dark</span>
            </button>
            <button
                onclick={() => onThemeChange("light")}
                class="flex-1 flex items-center justify-center gap-2 px-3 py-2 rounded-lg border transition-all
                    {theme === 'light'
                        ? 'bg-[var(--accent)]/20 border-[var(--accent)] text-[var(--accent)]'
                        : 'theme-bg-tertiary theme-border theme-text-muted hover:theme-text-primary'}"
            >
                <i class="fa-solid fa-sun"></i>
                <span class="text-sm">Light</span>
            </button>
            <button
                onclick={() => onThemeChange("system")}
                class="flex-1 flex items-center justify-center gap-2 px-3 py-2 rounded-lg border transition-all
                    {theme === 'system'
                        ? 'bg-[var(--accent)]/20 border-[var(--accent)] text-[var(--accent)]'
                        : 'theme-bg-tertiary theme-border theme-text-muted hover:theme-text-primary'}"
            >
                <i class="fa-solid fa-circle-half-stroke"></i>
                <span class="text-sm">System</span>
            </button>
        </div>
    </section>

    <!-- Storage -->
    <section class="mb-6">
        <h3 class="text-sm theme-text-muted mb-3">Storage</h3>

        <div class="space-y-3">
            <div class="flex items-center justify-between py-2">
                <div class="min-w-0 flex-1">
                    <p class="text-sm theme-text-secondary">Database</p>
                    <p class="text-xs theme-text-muted font-mono truncate">{dbPath || "..."}</p>
                </div>
                <button
                    onclick={() => handleOpenPath(dbPath, "reveal")}
                    class="ml-3 p-1.5 rounded hover:theme-bg-tertiary theme-text-muted hover:theme-text-primary text-xs"
                >
                    <i class="fa-solid fa-folder-open"></i>
                </button>
            </div>

            <div class="flex items-center justify-between py-2">
                <div class="min-w-0 flex-1">
                    <p class="text-sm theme-text-secondary">Thumbnails</p>
                    <p class="text-xs theme-text-muted font-mono truncate">{thumbDir || "..."}</p>
                </div>
                <button
                    onclick={() => handleOpenPath(thumbDir)}
                    class="ml-3 p-1.5 rounded hover:theme-bg-tertiary theme-text-muted hover:theme-text-primary text-xs"
                >
                    <i class="fa-solid fa-folder-open"></i>
                </button>
            </div>
        </div>
    </section>

    <!-- Cache -->
    <section class="mb-6">
        <h3 class="text-sm theme-text-muted mb-3">Cache</h3>

        <div class="flex gap-2">
            <button
                onclick={handleClearCache}
                disabled={clearCacheLoading}
                class="px-3 py-1.5 rounded theme-bg-tertiary border theme-border text-sm theme-text-secondary hover:theme-text-primary disabled:opacity-50"
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
        <h3 class="text-sm theme-text-muted mb-3">About</h3>

        <div class="text-sm">
            <p class="theme-text-secondary">足迹相册 <span class="theme-text-muted">v{version}</span></p>
            <p class="theme-text-muted text-xs mt-1">Tauri + Svelte + Rust</p>
        </div>
    </section>
</div>
