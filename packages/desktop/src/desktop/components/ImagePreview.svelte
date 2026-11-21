<script module lang="ts">
    // The playback speed chosen last: the next video starts at it (the
    // viewer makes a new preview for every item).
    let rememberedSpeed = 1;
</script>

<script lang="ts">
    import { onDestroy } from "svelte";
    import { convertFileSrc } from "@tauri-apps/api/core";
    import { invoke } from "@tauri-apps/api/core";
    import { appDataDir } from "@tauri-apps/api/path";
    import type { Translations } from "../../shared/i18n/types";

    let { src, alt = "Preview", thumbPath = "", t, mediaType = "photo", videoPath = "", onZoomed, active = true } = $props<{
        /** The one on display. The neighbours beside it in the viewer are
            loaded and ready but do nothing: no playing, no keys, no tools. */
        active?: boolean;
        /** Told when the photo becomes zoomed in, and when it no longer is. */
        onZoomed?: (zoomed: boolean) => void;
        src: string;
        alt?: string;
        thumbPath?: string;
        t: Translations;
        mediaType?: string;
        videoPath?: string;
    }>();

    let container: HTMLDivElement | undefined = $state();
    let zoom = $state(1);
    $effect(() => {
        if (active) onZoomed?.(zoom > 1.001);
    });
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
    let liveVideoPlaying = $state(false);

    // ── Rotation (of the view only: the file is not touched) ──
    // In quarter turns clockwise. A new item starts upright.
    let rotation = $state(0);
    let sideways = $derived(rotation % 180 !== 0);
    function rotate(by: number) {
        rotation = (rotation + by + 360) % 360;
        zoom = 1;
        if (container) {
            container.scrollLeft = 0;
            container.scrollTop = 0;
        }
    }

    // ── Playback speed ──
    const SPEEDS = [0.5, 1, 1.5, 2, 3];
    let speed = $state(rememberedSpeed);
    let videoEl: HTMLVideoElement | undefined = $state();
    function applySpeed() {
        if (!videoEl) return;
        // Loading a video puts the rate back to the default one.
        videoEl.defaultPlaybackRate = speed;
        videoEl.playbackRate = speed;
    }
    function setSpeed(value: number) {
        speed = value;
        rememberedSpeed = value;
        applySpeed();
    }
    $effect(() => {
        void speed;
        applySpeed();
    });

    // Video helper: open in system player
    async function openInSystemPlayer() {
        try {
            await invoke("open_in_system_player", { path: src });
        } catch (e) {
            console.error("Failed to open in system player:", e);
        }
    }

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

    // Developing a RAW can take seconds, by which time another photo may be
    // showing or the viewer may be closed. Each request gets a number and only
    // the latest one may touch the state. (Comparing with `src` instead reads
    // a prop of a viewer that no longer exists, and throws.)
    let rawRequest = 0;
    onDestroy(() => { rawRequest++; });

    async function loadRawPreview(path: string) {
        const request = ++rawRequest;
        try {
            const cacheDir = await appDataDir();
            const previewPath = await invoke<string>("get_raw_preview", { path, cacheDir });
            if (request !== rawRequest) return;
            rawPreviewPath = previewPath;
        } catch (e) {
            if (request !== rawRequest) return;
            console.error("[RAW] Failed to load full preview, will use thumbnail:", e);
            rawPreviewFailed = true;
            if (!thumbPath) {
                error = true;
            }
        } finally {
            if (request === rawRequest) {
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
        // Turned on its side, the picture is as wide as it was tall.
        const w = sideways ? naturalHeight : naturalWidth;
        const h = sideways ? naturalWidth : naturalHeight;
        const ratio = Math.min(containerWidth / w, containerHeight / h);
        return {
            width: w * ratio,
            height: h * ratio
        };
    });

    // Zoomed dimensions
    let zoomedWidth = $derived(baseSize.width * zoom);
    let zoomedHeight = $derived(baseSize.height * zoom);
    // The picture sits in a box of the size it takes on screen and is turned
    // inside it; its own width and height are those before the turn.
    let boxStyle = $derived(`position: relative; flex: none; width: ${zoomedWidth}px; height: ${zoomedHeight}px;`);
    let turnedStyle = $derived(
        `position: absolute; left: 50%; top: 50%; max-width: none; max-height: none;`
        + ` width: ${sideways ? zoomedHeight : zoomedWidth}px; height: ${sideways ? zoomedWidth : zoomedHeight}px;`
        + ` transform: translate(-50%, -50%) rotate(${rotation}deg);`,
    );
    // A video fills the area it is given, turned or not. Only the picture
    // turns: the controls are drawn by this component (below), because the
    // system's own are part of the video element and would turn with it.
    let videoStyle = $derived(
        `position: absolute; left: 50%; top: 50%; max-width: none; max-height: none; object-fit: contain;`
        + ` width: ${sideways ? containerHeight : containerWidth}px; height: ${sideways ? containerWidth : containerHeight}px;`
        + ` transform: translate(-50%, -50%) rotate(${rotation}deg);`,
    );

    // ── Video controls ──
    let paused = $state(true);
    let currentTime = $state(0);
    let duration = $state(0);
    let muted = $state(false);
    function togglePlay() {
        if (!videoEl) return;
        if (videoEl.paused) videoEl.play().catch(() => {});
        else videoEl.pause();
    }
    function clock(seconds: number): string {
        if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
        const s = Math.floor(seconds);
        const h = Math.floor(s / 3600);
        const m = Math.floor((s % 3600) / 60);
        const rest = String(s % 60).padStart(2, "0");
        return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${rest}` : `${m}:${rest}`;
    }
    function seek(e: Event) {
        if (!videoEl) return;
        videoEl.currentTime = Number((e.target as HTMLInputElement).value);
    }

    function zoomAt(newZoom: number, clientX: number, clientY: number) {
        if (!container) return;
        const oldZoom = zoom;
        newZoom = Math.max(1, Math.min(10, newZoom));
        if (Math.abs(newZoom - oldZoom) < 0.001) return;

        const rect = container.getBoundingClientRect();
        const mouseX = clientX - rect.left;
        const mouseY = clientY - rect.top;

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

    function handleWheel(e: WheelEvent) {
        if (!(e.ctrlKey || e.metaKey)) return;
        e.preventDefault();
        e.stopPropagation();
        zoomAt(zoom * (1 - e.deltaY * 0.002), e.clientX, e.clientY);
    }

    // A trackpad pinch arrives in WebKit as gesture events, not as a wheel
    // event with the control key.
    let pinchBase = 1;
    $effect(() => {
        const el = container;
        if (!el) return;
        const point = (e: Event): [number, number] => {
            const { clientX: x, clientY: y } = e as MouseEvent;
            if (typeof x === "number" && typeof y === "number" && (x !== 0 || y !== 0)) return [x, y];
            const r = el.getBoundingClientRect();
            return [r.left + r.width / 2, r.top + r.height / 2];
        };
        const start = (e: Event) => {
            e.preventDefault();
            e.stopPropagation();
            pinchBase = zoom;
        };
        const change = (e: Event) => {
            e.preventDefault();
            e.stopPropagation();
            const scale = (e as Event & { scale?: number }).scale ?? 1;
            zoomAt(pinchBase * scale, ...point(e));
        };
        const options = { passive: false } as const;
        el.addEventListener("gesturestart", start, options);
        el.addEventListener("gesturechange", change, options);
        el.addEventListener("gestureend", start, options);
        return () => {
            el.removeEventListener("gesturestart", start);
            el.removeEventListener("gesturechange", change);
            el.removeEventListener("gestureend", start);
        };
    });

    function handleDoubleClick() {
        zoom = 1;
        if (container) {
            container.scrollLeft = 0;
            container.scrollTop = 0;
        }
    }

    // Out of sight, a view goes back to how it started: upright, not zoomed,
    // not playing. Back on display, a video plays.
    $effect(() => {
        if (active) {
            videoEl?.play().catch(() => {});
            return;
        }
        zoom = 1;
        rotation = 0;
        liveVideoPlaying = false;
        videoEl?.pause();
    });

    function handleKeydown(e: KeyboardEvent) {
        if (!active) return;
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

{#if mediaType === 'video'}
    <!-- Video preview: inline playback -->
    <div class="w-full h-full flex flex-col bg-black">
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div bind:this={container} class="relative flex-1 min-h-0 overflow-hidden" onclick={togglePlay}>
            <!-- svelte-ignore a11y_media_has_caption -->
            <video
                bind:this={videoEl}
                bind:paused
                bind:currentTime
                bind:duration
                bind:muted
                src={convertFileSrc(src)}
                autoplay={active}
                loop
                playsinline
                style={videoStyle}
                poster={thumbPath ? convertFileSrc(thumbPath) : undefined}
                onloadedmetadata={applySpeed}
                onplay={applySpeed}
                data-preview-video={active ? "" : undefined}
            ></video>
        </div>
        <!-- The controls stay put whichever way the picture is turned. -->
        <div class="flex-none flex items-center gap-3 px-4 py-2 theme-bg-card" data-preview-controls={active ? "" : undefined}>
            <button
                class="w-8 h-8 flex-none rounded-full hover:theme-bg-secondary theme-text-primary flex items-center justify-center"
                onclick={togglePlay}
                data-preview-play={active ? "" : undefined}
                aria-label={paused ? "Play" : "Pause"}
            ><i class="fa-solid {paused ? 'fa-play' : 'fa-pause'} text-sm"></i></button>
            <span class="flex-none text-xs tabular-nums theme-text-secondary" data-preview-time={active ? "" : undefined}>{clock(currentTime)}</span>
            <input
                type="range"
                class="seek flex-1 min-w-0"
                style="--played: {duration > 0 ? (currentTime / duration) * 100 : 0}%"
                min="0"
                max={duration || 0}
                step="0.05"
                value={currentTime}
                oninput={seek}
                data-preview-seek={active ? "" : undefined}
            />
            <span class="flex-none text-xs tabular-nums theme-text-secondary">{clock(duration)}</span>
            <button
                class="w-8 h-8 flex-none rounded-full hover:theme-bg-secondary theme-text-secondary hover:theme-text-primary flex items-center justify-center"
                onclick={() => muted = !muted}
                aria-label={muted ? "Unmute" : "Mute"}
            ><i class="fa-solid {muted ? 'fa-volume-xmark' : 'fa-volume-high'} text-sm"></i></button>
        </div>
    </div>

{:else if mediaType === 'live_photo'}
    <!-- Live Photo: show image by default, play video on hover -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
        bind:this={container}
        class="w-full h-full overflow-auto bg-transparent relative"
        onwheel={handleWheel}
        ondblclick={handleDoubleClick}
        onmouseenter={() => { if (videoPath && active) liveVideoPlaying = true; }}
        onmouseleave={() => liveVideoPlaying = false}
    >
        {#if liveVideoPlaying && videoPath}
            <div class="absolute inset-0 overflow-hidden">
                <video
                    src={convertFileSrc(videoPath)}
                    autoplay
                    loop
                    muted
                    playsinline
                    style={videoStyle}
                ></video>
            </div>
        {:else}
            <div
                class="flex items-center justify-center"
                style="min-width: {Math.max(containerWidth, zoomedWidth)}px; min-height: {Math.max(containerHeight, zoomedHeight)}px;"
            >
                {#if imageSrc}
                    <div style={boxStyle}>
                        <img
                            src={imageSrc}
                            {alt}
                            class="{loaded ? '' : 'opacity-0'} {error ? 'hidden' : ''}"
                            style={turnedStyle}
                            onload={handleLoad}
                            onerror={handleError}
                            draggable="false"
                            data-preview-image={active ? "" : undefined}
                        />
                    </div>
                {/if}
            </div>
        {/if}

        <!-- Live Photo badge -->
        <div class="absolute top-3 left-3 px-2 py-1 rounded-full
                    bg-white/20 backdrop-blur-sm text-xs text-white
                    flex items-center gap-1 pointer-events-none">
            <i class="fa-solid fa-circle-dot"></i>
            LIVE
        </div>
    </div>

{:else}
    <!-- Standard photo/RAW preview (existing logic) -->
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
                    <p class="text-sm">{t.imageError.unableToLoad}</p>
                    <p class="text-xs text-neutral-600">
                        {#if isRaw}
                            {t.imageError.rawPreviewFailed}
                        {:else}
                            {t.imageError.fileMissing}
                        {/if}
                    </p>
                </div>
            {:else if thumbPath && (!loaded || loading)}
                <!-- The full picture takes a moment (a RAW has to be developed,
                     a large JPEG decoded): show the thumbnail at once and swap
                     the full picture in when ready. -->
                <div class="absolute inset-0 flex items-center justify-center pointer-events-none">
                    <img src={convertFileSrc(thumbPath)} alt="" class="max-w-full max-h-full w-full h-full object-contain" style="transform: rotate({rotation}deg);" draggable="false" />
                    {#if isRaw}
                    <div class="absolute bottom-4 right-4 px-2.5 py-1.5 rounded bg-black/60 text-xs text-neutral-200 flex items-center gap-2">
                        <i class="fa-solid fa-spinner fa-spin"></i>
                        {t.imageError.extractingRaw}
                    </div>
                    {/if}
                </div>
            {:else if !loaded || loading}
                <div class="absolute inset-0 flex flex-col items-center justify-center gap-2">
                    <i class="fa-solid fa-spinner fa-spin text-neutral-500 text-2xl"></i>
                    {#if isRaw && loading}
                        <p class="text-neutral-500 text-xs">{t.imageError.extractingRaw}</p>
                    {/if}
                </div>
            {/if}
            {#if imageSrc}
            <div style={boxStyle}>
                <img
                    src={imageSrc}
                    {alt}
                    class="{loaded ? '' : 'opacity-0'} {error ? 'hidden' : ''}"
                    style={turnedStyle}
                    onload={handleLoad}
                    onerror={handleError}
                    draggable="false"
                    decoding="async"
                    data-preview-image={active ? "" : undefined}
                />
            </div>
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
            {t.imageError.showingThumbnail}
        </div>
    {/if}
{/if}

<!-- Turn the view; for a video, how fast it plays. -->
{#if active}
<div class="absolute top-3 right-3 z-10 flex items-center gap-2" data-preview-tools>
    {#if mediaType === 'video'}
        <div class="flex items-center rounded-full theme-bg-card overflow-hidden" title={t.preview.speed}>
            {#each SPEEDS as value}
                <button
                    class="px-2.5 py-1.5 text-xs tabular-nums {speed === value ? 'bg-amber-500 text-black font-medium' : 'theme-text-secondary hover:theme-text-primary hover:theme-bg-secondary'}"
                    onclick={() => setSpeed(value)}
                    data-preview-speed={value}
                >{value}×</button>
            {/each}
        </div>
    {/if}
    <button
        class="w-8 h-8 rounded-full theme-bg-card hover:theme-bg-secondary theme-text-secondary hover:theme-text-primary flex items-center justify-center"
        title={t.preview.rotateLeft}
        aria-label={t.preview.rotateLeft}
        onclick={() => rotate(-90)}
        data-preview-rotate="left"
    ><i class="fa-solid fa-rotate-left text-sm"></i></button>
    <button
        class="w-8 h-8 rounded-full theme-bg-card hover:theme-bg-secondary theme-text-secondary hover:theme-text-primary flex items-center justify-center"
        title={t.preview.rotateRight}
        aria-label={t.preview.rotateRight}
        onclick={() => rotate(90)}
        data-preview-rotate="right"
    ><i class="fa-solid fa-rotate-right text-sm"></i></button>
</div>
{/if}

<style>
    /* The seek bar: a thin track, the played part in the accent colour. */
    .seek {
        -webkit-appearance: none;
        appearance: none;
        height: 4px;
        border-radius: 2px;
        background: linear-gradient(to right, #f59e0b var(--played), rgba(255, 255, 255, 0.25) var(--played));
        cursor: pointer;
    }
    .seek::-webkit-slider-thumb {
        -webkit-appearance: none;
        appearance: none;
        width: 12px;
        height: 12px;
        border-radius: 50%;
        background: #f59e0b;
        border: none;
    }
</style>
