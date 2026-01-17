<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { emit } from "@tauri-apps/api/event";
    import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
    import type { Theme } from "../../shared/types";
    import { locales, type Translations, type Locale } from "../../shared/i18n";

    let { dbPath, thumbDir, version, theme, onThemeChange, t, locale, onLocaleChange } = $props<{
        dbPath: string;
        thumbDir: string;
        version: string;
        theme: Theme;
        onThemeChange: (theme: Theme) => void;
        t: Translations;
        locale: Locale;
        onLocaleChange: (locale: Locale) => void;
    }>();

    let clearCacheLoading = $state(false);

    async function handleClearCache() {
        if (!thumbDir || !dbPath) return;

        const confirmed = confirm(t.settings.clearCacheConfirm);
        if (!confirmed) return;

        clearCacheLoading = true;
        try {
            await invoke("clear_app_data", { thumbDir, dbPath });
            await emit("reload-photos");
        } catch (e) {
            alert(t.errors.clearFailed + ": " + e);
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
            alert(t.errors.openFailed + ": " + e);
        }
    }
</script>

<div class="max-w-2xl">
    <!-- Appearance -->
    <section class="mb-6">
        <h3 class="text-sm theme-text-muted mb-3">{t.settings.appearance}</h3>

        <div class="flex items-center gap-2">
            <button
                onclick={() => onThemeChange("dark")}
                class="flex-1 flex items-center justify-center gap-2 px-3 py-2 rounded-lg border transition-all
                    {theme === 'dark'
                        ? 'bg-[var(--accent)]/20 border-[var(--accent)] text-[var(--accent)]'
                        : 'theme-bg-tertiary theme-border theme-text-muted hover:theme-text-primary'}"
            >
                <i class="fa-solid fa-moon"></i>
                <span class="text-sm">{t.settings.themeDark}</span>
            </button>
            <button
                onclick={() => onThemeChange("light")}
                class="flex-1 flex items-center justify-center gap-2 px-3 py-2 rounded-lg border transition-all
                    {theme === 'light'
                        ? 'bg-[var(--accent)]/20 border-[var(--accent)] text-[var(--accent)]'
                        : 'theme-bg-tertiary theme-border theme-text-muted hover:theme-text-primary'}"
            >
                <i class="fa-solid fa-sun"></i>
                <span class="text-sm">{t.settings.themeLight}</span>
            </button>
            <button
                onclick={() => onThemeChange("system")}
                class="flex-1 flex items-center justify-center gap-2 px-3 py-2 rounded-lg border transition-all
                    {theme === 'system'
                        ? 'bg-[var(--accent)]/20 border-[var(--accent)] text-[var(--accent)]'
                        : 'theme-bg-tertiary theme-border theme-text-muted hover:theme-text-primary'}"
            >
                <i class="fa-solid fa-circle-half-stroke"></i>
                <span class="text-sm">{t.settings.themeSystem}</span>
            </button>
        </div>
    </section>

    <!-- Language -->
    <section class="mb-6">
        <h3 class="text-sm theme-text-muted mb-3">{t.settings.language}</h3>

        <select
            value={locale}
            onchange={(e) => onLocaleChange(e.currentTarget.value as Locale)}
            class="w-full px-3 py-2 rounded-lg border theme-bg-tertiary theme-border theme-text-primary text-sm cursor-pointer focus:outline-none focus:ring-2 focus:ring-[var(--accent)]"
        >
            {#each locales as loc}
                <option value={loc.code} selected={locale === loc.code}>
                    {loc.nativeName} ({loc.name})
                </option>
            {/each}
        </select>
    </section>

    <!-- Storage -->
    <section class="mb-6">
        <h3 class="text-sm theme-text-muted mb-3">{t.settings.storage}</h3>

        <div class="space-y-3">
            <div class="flex items-center justify-between py-2">
                <div class="min-w-0 flex-1">
                    <p class="text-sm theme-text-secondary">{t.settings.database}</p>
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
                    <p class="text-sm theme-text-secondary">{t.settings.thumbnails}</p>
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
        <h3 class="text-sm theme-text-muted mb-3">{t.settings.cache}</h3>

        <div class="flex gap-2">
            <button
                onclick={handleClearCache}
                disabled={clearCacheLoading}
                class="px-3 py-1.5 rounded theme-bg-tertiary border theme-border text-sm theme-text-secondary hover:theme-text-primary disabled:opacity-50"
            >
                {#if clearCacheLoading}
                    <i class="fa-solid fa-spinner fa-spin mr-1"></i>
                {/if}
                {t.settings.clearCache}
            </button>
        </div>
    </section>

    <!-- About -->
    <section>
        <h3 class="text-sm theme-text-muted mb-3">{t.settings.about}</h3>

        <div class="text-sm">
            <p class="theme-text-secondary">{t.app.name} <span class="theme-text-muted">v{version}</span></p>
            <p class="theme-text-muted text-xs mt-1">{t.settings.techStack}</p>
        </div>
    </section>
</div>
