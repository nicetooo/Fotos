<script lang="ts">
    import { convertFileSrc } from "@tauri-apps/api/core";

    let {
        path,
        alt,
        className,
        refreshKey,
        lazy = false,
    } = $props<{
        path: string | undefined;
        alt: string;
        className?: string;
        refreshKey?: any;
        lazy?: boolean;
    }>();

    let src = $state("");
    let loading = $state(true);
    let error = $state(false);
    let containerElement: HTMLDivElement | undefined = $state();
    let isIntersecting = $state(false);
    let observer: IntersectionObserver | undefined;

    function updateSrc() {
        if (!path) {
            src = "";
            loading = false;
            return;
        }
        // Use asset protocol - no IPC, direct file access
        src = convertFileSrc(path);
        loading = true;
        error = false;
    }

    // Setup intersection observer for lazy loading
    $effect(() => {
        if (lazy && containerElement) {
            observer = new IntersectionObserver(
                (entries) => {
                    entries.forEach((entry) => {
                        if (entry.isIntersecting) {
                            isIntersecting = true;
                            observer?.disconnect();
                        }
                    });
                },
                {
                    rootMargin: "200px",
                },
            );

            observer.observe(containerElement);

            return () => {
                observer?.disconnect();
            };
        }
    });

    // React to path changes, refresh, or intersection
    $effect(() => {
        path;
        refreshKey;

        if (lazy) {
            if (isIntersecting) {
                updateSrc();
            }
        } else {
            updateSrc();
        }
    });

    function handleLoad() {
        loading = false;
        error = false;
    }

    function handleError() {
        loading = false;
        error = true;
    }
</script>

{#if error || !path}
    <div
        bind:this={containerElement}
        class="{className} flex items-center justify-center bg-neutral-800 text-neutral-600"
    >
        <i class="fa-solid fa-image text-xl"></i>
    </div>
{:else}
    <div bind:this={containerElement} class="{className} relative bg-neutral-800">
        {#if loading}
            <div class="absolute inset-0 flex items-center justify-center">
                <i class="fa-solid fa-spinner fa-spin text-neutral-600"></i>
            </div>
        {/if}
        <img
            {src}
            {alt}
            class="w-full h-full object-cover {loading ? 'opacity-0' : 'opacity-100'}"
            onload={handleLoad}
            onerror={handleError}
        />
    </div>
{/if}
