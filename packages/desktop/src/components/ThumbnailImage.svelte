<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { onDestroy } from "svelte";

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
    let loading = $state(false);
    let error = $state(false);
    let objectUrl: string | null = null;
    let containerElement: HTMLDivElement | undefined = $state();
    let isIntersecting = $state(false);
    let observer: IntersectionObserver | undefined;

    async function load() {
        if (!path) {
            src = "";
            return;
        }

        loading = true;
        error = false;
        try {
            const bytes = await invoke<number[]>("read_file_bytes", { path });
            const blob = new Blob([new Uint8Array(bytes)]);
            if (objectUrl) {
                URL.revokeObjectURL(objectUrl);
            }
            objectUrl = URL.createObjectURL(blob);
            src = objectUrl;
        } catch (e) {
            console.error("Failed to load thumbnail:", path, e);
            error = true;
        } finally {
            loading = false;
        }
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
                    rootMargin: "200px", // Start loading 200px before entering viewport
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
        path; // dependency
        refreshKey; // dependency

        if (lazy) {
            // Only load when intersecting
            if (isIntersecting) {
                load();
            }
        } else {
            // Load immediately if not lazy
            load();
        }
    });

    onDestroy(() => {
        if (objectUrl) {
            URL.revokeObjectURL(objectUrl);
        }
        observer?.disconnect();
    });
</script>

{#if error || !path}
    <div
        bind:this={containerElement}
        class={className +
            " flex items-center justify-center bg-slate-800 text-slate-600"}
    >
        <i class="fa-solid fa-image-slash text-2xl"></i>
    </div>
{:else if loading && !src}
    <div
        bind:this={containerElement}
        class={className +
            " flex items-center justify-center bg-slate-800 animate-pulse"}
    >
        <i class="fa-solid fa-circle-notch fa-spin text-slate-600"></i>
    </div>
{:else}
    <img bind:this={containerElement} {src} {alt} class={className} />
{/if}
