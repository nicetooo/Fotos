<script lang="ts">
    import { locales, type Locale } from "../../shared/i18n";

    let { value, onChange, label } = $props<{
        value: Locale;
        onChange: (locale: Locale) => void;
        label?: string;
    }>();

    let open = $state(false);

    let selectedDisplay = $derived.by(() => {
        const loc = locales.find((l) => l.code === value);
        return loc ? `${loc.nativeName} (${loc.name})` : value;
    });

    function select(code: Locale) {
        onChange(code);
        open = false;
    }

    function handleBackdropClick() {
        open = false;
    }
</script>

<!-- Trigger button -->
<button
    type="button"
    onclick={() => (open = !open)}
    class="w-full px-3 py-3 rounded-lg border theme-bg-tertiary theme-border theme-text-primary text-sm
        flex items-center justify-between gap-2 transition-all active:scale-[0.98]
        {open ? 'ring-2' : ''}"
    style={open ? 'ring-color: var(--accent); --tw-ring-color: var(--accent);' : ''}
>
    <div class="flex items-center gap-2.5 min-w-0">
        <i class="fa-solid fa-globe text-[var(--accent)] text-base shrink-0"></i>
        <span class="truncate">{selectedDisplay}</span>
    </div>
    <i
        class="fa-solid fa-chevron-down text-xs theme-text-muted shrink-0 transition-transform duration-200
            {open ? 'rotate-180' : ''}"
    ></i>
</button>

<!-- Dropdown overlay -->
{#if open}
    <!-- Backdrop -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
        class="fixed inset-0 z-[100] bg-black/40 backdrop-blur-[2px]"
        onclick={handleBackdropClick}
        onkeydown={(e) => e.key === 'Escape' && (open = false)}
    ></div>

    <!-- Bottom sheet dropdown -->
    <div
        class="fixed bottom-0 left-0 right-0 z-[101] animate-slide-up"
    >
        <div
            class="mx-2 mb-2 rounded-2xl border theme-border overflow-hidden shadow-2xl"
            style="background-color: var(--bg-secondary);"
        >
            <!-- Header -->
            <div class="flex items-center justify-between px-4 py-3 border-b theme-border">
                <span class="text-sm font-medium theme-text-primary">{label ?? 'Language'}</span>
                <button
                    type="button"
                    onclick={() => (open = false)}
                    class="w-7 h-7 flex items-center justify-center rounded-full theme-bg-tertiary theme-text-muted"
                >
                    <i class="fa-solid fa-xmark text-xs"></i>
                </button>
            </div>

            <!-- Options list -->
            <div class="max-h-[50vh] overflow-y-auto overscroll-contain py-1">
                {#each locales as loc}
                    <button
                        type="button"
                        onclick={() => select(loc.code)}
                        class="w-full flex items-center gap-3 px-4 py-3 text-left transition-colors active:bg-white/5
                            {value === loc.code ? 'selected-item' : ''}"
                        style={value === loc.code ? 'background-color: color-mix(in srgb, var(--accent) 10%, transparent);' : ''}
                    >
                        <!-- Check icon -->
                        <div class="w-5 h-5 flex items-center justify-center shrink-0">
                            {#if value === loc.code}
                                <i class="fa-solid fa-check text-[var(--accent)] text-sm"></i>
                            {/if}
                        </div>

                        <!-- Language info -->
                        <div class="flex-1 min-w-0">
                            <span
                                class="text-sm {value === loc.code ? 'font-medium' : 'theme-text-primary'}"
                                style={value === loc.code ? 'color: var(--accent);' : ''}
                            >
                                {loc.nativeName}
                            </span>
                            <span
                                class="text-xs ml-1.5 {value === loc.code ? '' : 'theme-text-muted'}"
                                style={value === loc.code ? 'color: var(--accent); opacity: 0.7;' : ''}
                            >
                                {loc.name}
                            </span>
                        </div>
                    </button>
                {/each}
            </div>
        </div>
    </div>
{/if}

<style>
    @keyframes slide-up {
        from {
            transform: translateY(100%);
            opacity: 0.5;
        }
        to {
            transform: translateY(0);
            opacity: 1;
        }
    }

    .animate-slide-up {
        animation: slide-up 0.25s cubic-bezier(0.16, 1, 0.3, 1);
    }
</style>
