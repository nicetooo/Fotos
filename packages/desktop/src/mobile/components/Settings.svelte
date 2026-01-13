<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { emit } from "@tauri-apps/api/event";
    import type { Theme } from "../../shared/types";

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

        const confirmed = confirm("This will delete all imported photos and thumbnails.\n\nContinue?");
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
</script>

<div class="max-w-2xl">
    <!-- Appearance -->
    <section class="mb-6">
        <h3 class="text-sm theme-text-muted mb-3">Appearance</h3>

        <div class="flex items-center gap-2">
            <button
                onclick={() => onThemeChange("dark")}
                class="flex-1 flex items-center justify-center gap-2 px-3 py-3 rounded-lg border transition-all
                    {theme === 'dark'
                        ? 'bg-[var(--accent)]/20 border-[var(--accent)] text-[var(--accent)]'
                        : 'theme-bg-tertiary theme-border theme-text-muted hover:theme-text-primary'}"
            >
                <i class="fa-solid fa-moon"></i>
                <span class="text-sm">Dark</span>
            </button>
            <button
                onclick={() => onThemeChange("light")}
                class="flex-1 flex items-center justify-center gap-2 px-3 py-3 rounded-lg border transition-all
                    {theme === 'light'
                        ? 'bg-[var(--accent)]/20 border-[var(--accent)] text-[var(--accent)]'
                        : 'theme-bg-tertiary theme-border theme-text-muted hover:theme-text-primary'}"
            >
                <i class="fa-solid fa-sun"></i>
                <span class="text-sm">Light</span>
            </button>
            <button
                onclick={() => onThemeChange("system")}
                class="flex-1 flex items-center justify-center gap-2 px-3 py-3 rounded-lg border transition-all
                    {theme === 'system'
                        ? 'bg-[var(--accent)]/20 border-[var(--accent)] text-[var(--accent)]'
                        : 'theme-bg-tertiary theme-border theme-text-muted hover:theme-text-primary'}"
            >
                <i class="fa-solid fa-circle-half-stroke"></i>
                <span class="text-sm">Auto</span>
            </button>
        </div>
    </section>

    <!-- Cache -->
    <section class="mb-6">
        <h3 class="text-sm theme-text-muted mb-3">Data</h3>

        <button
            onclick={handleClearCache}
            disabled={clearCacheLoading}
            class="w-full px-4 py-3 rounded-lg theme-bg-tertiary border theme-border text-sm theme-text-secondary hover:theme-text-primary disabled:opacity-50 flex items-center justify-center gap-2"
        >
            {#if clearCacheLoading}
                <i class="fa-solid fa-spinner fa-spin"></i>
            {/if}
            Clear All Data
        </button>
    </section>

    <!-- About -->
    <section>
        <h3 class="text-sm theme-text-muted mb-3">About</h3>

        <div class="text-sm text-center">
            <p class="theme-text-secondary">Fotos <span class="theme-text-muted">v{version}</span></p>
        </div>
    </section>
</div>
