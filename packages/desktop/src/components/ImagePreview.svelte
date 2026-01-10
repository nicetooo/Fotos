<script lang="ts">
    import { convertFileSrc } from "@tauri-apps/api/core";
    import { invoke } from "@tauri-apps/api/core";
    import { appDataDir } from "@tauri-apps/api/path";

    let { src, alt = "Preview" } = $props<{
        src: string;
        alt?: string;
    }>();

    let container: HTMLDivElement | undefined = $state();
    let zoom = $state(1);
    let loaded = $state(false);
    let error = $state(false);
    let naturalWidth = $state(0);
    let naturalHeight = $state(0);
    let containerWidth = $state(0);
    let containerHeight = $state(0);
    let rawPreviewPath = $state<string | null>(null);
    let loadedSrc = $state<string>("");
    let loading = $state(false);

    // Check if file is a RAW format
    const RAW_EXTENSIONS = ["cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "dng", "raf", "orf", "rw2", "pef", "raw"];
    function isRawFile(path: string): boolean {
        const ext = path.split(".").pop()?.toLowerCase() || "";
        return RAW_EXTENSIONS.includes(ext);
    }

    // Get the image source URL
    let imageSrc = $derived.by(() => {
        if (isRawFile(src)) {
            return rawPreviewPath ? convertFileSrc(rawPreviewPath) : "";
        }
        return convertFileSrc(src);
    });

    // Reset state when src changes
    $effect(() => {
        const currentSrc = src;

        // Reset zoom and scroll
        zoom = 1;
        loaded = false;
        error = false;
        if (container) {
            container.scrollLeft = 0;
            container.scrollTop = 0;
        }

        // Clear old RAW preview path if src changed
        if (loadedSrc !== currentSrc) {
            rawPreviewPath = null;
        }

        // Load RAW preview if needed
        if (isRawFile(currentSrc) && loadedSrc !== currentSrc && !loading) {
            loading = true;
            loadRawPreview(currentSrc);
        }
    });

    async function loadRawPreview(path: string) {
        console.log("[RAW] Loading preview for:", path);
        try {
            const cacheDir = await appDataDir();
            const previewPath = await invoke<string>("get_raw_preview", { path, cacheDir });
            console.log("[RAW] Preview cached at:", previewPath);
            // Only set if this is still the current src
            if (path === src) {
                rawPreviewPath = previewPath;
                loadedSrc = path;
            }
        } catch (e) {
            console.error("[RAW] Failed to load RAW preview:", e);
            if (path === src) {
                error = true;
            }
        } finally {
            loading = false;
        }
    }

    // Observe container size
    $effect(() => {
        if (!container) return;
        const observer = new ResizeObserver((entries) => {
            for (const entry of entries) {
                containerWidth = entry.contentRect.width;
                containerHeight = entry.contentRect.height;
            }
        });
        observer.observe(container);
        return () => observer.disconnect();
    });

    function handleLoad(e: Event) {
        const img = e.target as HTMLImageElement;
        naturalWidth = img.naturalWidth;
        naturalHeight = img.naturalHeight;
        loaded = true;
        error = false;
    }

    function handleError() {
        error = true;
        loaded = false;
    }

    // Calculate base size that fits container while preserving aspect ratio
    let baseSize = $derived.by(() => {
        if (!naturalWidth || !naturalHeight || !containerWidth || !containerHeight) {
            return { width: 0, height: 0 };
        }
        const ratio = Math.min(containerWidth / naturalWidth, containerHeight / naturalHeight);
        return {
            width: naturalWidth * ratio,
            height: naturalHeight * ratio
        };
    });

    // Zoomed dimensions
    let zoomedWidth = $derived(baseSize.width * zoom);
    let zoomedHeight = $derived(baseSize.height * zoom);

    function handleWheel(e: WheelEvent) {
        if (!(e.ctrlKey || e.metaKey)) return;
        e.preventDefault();
        e.stopPropagation();

        if (!container) return;

        const oldZoom = zoom;
        const delta = -e.deltaY * 0.002;
        const newZoom = Math.max(1, Math.min(10, oldZoom * (1 + delta)));

        if (Math.abs(newZoom - oldZoom) < 0.001) return;

        const rect = container.getBoundingClientRect();
        const mouseX = e.clientX - rect.left;
        const mouseY = e.clientY - rect.top;

        // Point in content space before zoom
        const contentX = container.scrollLeft + mouseX;
        const contentY = container.scrollTop + mouseY;

        // Scale ratio
        const scale = newZoom / oldZoom;

        zoom = newZoom;

        // Adjust scroll to zoom toward mouse position
        requestAnimationFrame(() => {
            if (container) {
                container.scrollLeft = contentX * scale - mouseX;
                container.scrollTop = contentY * scale - mouseY;
            }
        });
    }

    function handleDoubleClick() {
        zoom = 1;
        if (container) {
            container.scrollLeft = 0;
            container.scrollTop = 0;
        }
    }

    function handleKeydown(e: KeyboardEvent) {
        if (e.key === "0") {
            zoom = 1;
            if (container) {
                container.scrollLeft = 0;
                container.scrollTop = 0;
            }
        } else if (e.key === "=" || e.key === "+") {
            zoom = Math.min(10, zoom * 1.25);
        } else if (e.key === "-") {
            zoom = Math.max(1, zoom / 1.25);
        }
    }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
    bind:this={container}
    class="w-full h-full overflow-auto bg-transparent"
    onwheel={handleWheel}
    ondblclick={handleDoubleClick}
>
    <div
        class="flex items-center justify-center"
        style="min-width: {Math.max(containerWidth, zoomedWidth)}px; min-height: {Math.max(containerHeight, zoomedHeight)}px;"
    >
        {#if error && !loading}
            <div class="flex flex-col items-center justify-center text-neutral-500 gap-3">
                <i class="fa-solid fa-image-slash text-4xl"></i>
                <p class="text-sm">Unable to load image</p>
                <p class="text-xs text-neutral-600">File may have been moved or the drive disconnected</p>
            </div>
        {:else if !loaded || loading}
            <div class="absolute inset-0 flex items-center justify-center">
                <i class="fa-solid fa-spinner fa-spin text-neutral-500 text-2xl"></i>
            </div>
        {/if}
        {#if imageSrc}
        <img
            src={imageSrc}
            {alt}
            class="{loaded ? '' : 'opacity-0'} {error ? 'hidden' : ''}"
            style="width: {zoomedWidth}px; height: {zoomedHeight}px;"
            onload={handleLoad}
            onerror={handleError}
            draggable="false"
        />
        {/if}
    </div>
</div>

{#if zoom !== 1}
    <div class="absolute bottom-4 left-1/2 -translate-x-1/2 px-2 py-1 rounded bg-black/60 text-xs text-neutral-300 pointer-events-none">
        {Math.round(zoom * 100)}%
    </div>
{/if}
