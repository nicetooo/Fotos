<script lang="ts">
    /**
     * Limits the map, timeline and album to one media type.
     * A round button like its neighbours; tapping it opens the choices beside it.
     */
    import type { MediaFilter } from "../../shared/query";
    import type { Translations } from "../../shared/i18n";

    let { value, onChange, t } = $props<{
        value: MediaFilter;
        onChange: (value: MediaFilter) => void;
        t: Translations;
    }>();

    let open = $state(false);

    const options = $derived([
        { value: "all" as MediaFilter, icon: "fa-layer-group", label: t.library.filterAll },
        { value: "photo" as MediaFilter, icon: "fa-image", label: t.library.filterPhotos },
        { value: "video" as MediaFilter, icon: "fa-video", label: t.library.filterVideos },
        { value: "live_photo" as MediaFilter, icon: "fa-circle-dot", label: t.library.filterLivePhotos },
    ]);
    const current = $derived(options.find((o) => o.value === value) ?? options[0]);

    function pick(next: MediaFilter) {
        open = false;
        if (next !== value) onChange(next);
    }
</script>

<div class="relative self-start" data-map-overlay>
    <button
        onclick={() => (open = !open)}
        data-media-filter
        class="w-12 h-12 rounded-full border flex items-center justify-center transition-colors {value === 'all'
            ? 'theme-bg-card theme-border theme-text-secondary'
            : 'bg-[var(--accent)] border-transparent text-black'}"
        aria-label={current.label}
        aria-expanded={open}
    >
        <i class="fa-solid {value === 'all' ? 'fa-filter' : current.icon} text-lg"></i>
    </button>

    {#if open}
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div class="fixed inset-0 z-[1]" data-map-blocker onclick={() => (open = false)}></div>
        <div class="absolute left-14 top-0 z-[2] py-1 rounded-2xl theme-bg-secondary border theme-border min-w-[9rem] overflow-hidden" data-map-overlay>
            {#each options as option (option.value)}
                <button
                    onclick={() => pick(option.value)}
                    data-media-filter-option={option.value}
                    class="w-full h-11 px-4 flex items-center gap-3 text-sm whitespace-nowrap {option.value === value ? 'text-[var(--accent)] font-medium' : 'theme-text-primary'}"
                >
                    <i class="fa-solid {option.icon} w-5 text-center"></i>
                    <span class="flex-1 text-left">{option.label}</span>
                    {#if option.value === value}
                        <i class="fa-solid fa-check text-xs"></i>
                    {/if}
                </button>
            {/each}
        </div>
    {/if}
</div>
