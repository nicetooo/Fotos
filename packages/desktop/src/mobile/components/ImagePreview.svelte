<script lang="ts">
    import { convertFileSrc } from "@tauri-apps/api/core";
    import { invoke } from "@tauri-apps/api/core";
    import { appDataDir } from "@tauri-apps/api/path";

    let { src, alt = "Preview", thumbPath = "" } = $props<{
        src: string;
        alt?: string;
        thumbPath?: string;
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
    let rawPreviewFailed = $state(false);
    let loading = $state(false);
    let lastSrc = "";

    // Check if file is a RAW format
    const RAW_EXTENSIONS = ["cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "dng", "raf", "orf", "rw2", "pef", "raw"];
    function isRawFile(path: string): boolean {
        const ext = path.split(".").pop()?.toLowerCase() || "";
        return RAW_EXTENSIONS.includes(ext);
    }

    // Is current file a RAW?
    let isRaw = $derived(isRawFile(src));

    // Get the image source URL
    let imageSrc = $derived.by(() => {
        if (isRaw) {
            if (rawPreviewPath) {
                return convertFileSrc(rawPreviewPath);
            }
            if (rawPreviewFailed && thumbPath) {
                return convertFileSrc(thumbPath);
            }
            return "";
        }
        return convertFileSrc(src);
    });

    // Handle src changes
    $effect(() => {
        const currentSrc = src;

        if (currentSrc === lastSrc) return;
        lastSrc = currentSrc;

        // Reset view state
        zoom = 1;
        loaded = false;
        error = false;
        rawPreviewPath = null;
        rawPreviewFailed = false;
        loading = false;
        naturalWidth = 0;
        naturalHeight = 0;

        if (container) {
            container.scrollLeft = 0;
            container.scrollTop = 0;
        }

        if (isRawFile(currentSrc)) {
            loading = true;
            loadRawPreview(currentSrc);
        }
    });

    async function loadRawPreview(path: string) {
        try {
            const cacheDir = await appDataDir();
            const previewPath = await invoke<string>("get_raw_preview", { path, cacheDir });
            if (path === src) {
                rawPreviewPath = previewPath;
            }
        } catch (e) {
            console.error("[RAW] Failed to load full preview, will use thumbnail:", e);
            if (path === src) {
                rawPreviewFailed = true;
                if (!thumbPath) {
                    error = true;
                }
            }
        } finally {
            if (path === src) {
                loading = false;
            }
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

        const contentX = container.scrollLeft + mouseX;
        const contentY = container.scrollTop + mouseY;

        const scale = newZoom / oldZoom;

        zoom = newZoom;

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
                <p class="text-xs text-neutral-600">
                    {#if isRaw}
                        Failed to extract preview from RAW file
                    {:else}
                        File may have been moved or the drive disconnected
                    {/if}
                </p>
            </div>
        {:else if !loaded || loading}
            <div class="absolute inset-0 flex flex-col items-center justify-center gap-2">
                <i class="fa-solid fa-spinner fa-spin text-neutral-500 text-2xl"></i>
                {#if isRaw && loading}
                    <p class="text-neutral-500 text-xs">Extracting RAW preview...</p>
                {/if}
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

{#if isRaw && rawPreviewFailed && thumbPath && loaded}
    <div class="absolute top-4 left-1/2 -translate-x-1/2 px-3 py-1.5 rounded bg-amber-900/80 text-xs text-amber-200 pointer-events-none">
        <i class="fa-solid fa-triangle-exclamation mr-1.5"></i>
        Showing thumbnail (full preview unavailable)
    </div>
{/if}
