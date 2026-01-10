<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { onMount, onDestroy } from "svelte";

    let { path, alt, className, refreshKey } = $props<{
        path: string | undefined;
        alt: string;
        className?: string;
        refreshKey?: any;
    }>();

    let src = $state("");
    let loading = $state(false);
    let error = $state(false);
    let objectUrl: string | null = null;

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

    // React to path changes or refresh
    $effect(() => {
        path; // dependency
        refreshKey; // dependency
        load();
    });

    onDestroy(() => {
        if (objectUrl) {
            URL.revokeObjectURL(objectUrl);
        }
    });
</script>

{#if error || !path}
    <div
        class={className +
            " flex items-center justify-center bg-slate-800 text-slate-600"}
    >
        <i class="fa-solid fa-image-slash text-2xl"></i>
    </div>
{:else if loading && !src}
    <div
        class={className +
            " flex items-center justify-center bg-slate-800 animate-pulse"}
    >
        <i class="fa-solid fa-circle-notch fa-spin text-slate-600"></i>
    </div>
{:else}
    <img {src} {alt} class={className} />
{/if}
