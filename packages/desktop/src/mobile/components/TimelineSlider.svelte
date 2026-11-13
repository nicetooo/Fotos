<script lang="ts">
    import { untrack } from "svelte";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";
    import {
        windowCounts,
        columnCounts,
        distinctLines,
    } from "../../shared/utils/timeline-index";

    // The timeline works on a sorted array of wall-clock timestamps (ms).
    // Every computation is a binary search or a loop over pixels, so dragging
    // costs the same for 500 photos or 500,000.
    let {
        stamps,
        mapConstrained = false,
        onTimeRangeChange,
        onShowAll,
        t,
        locale
    } = $props<{
        /** Ascending timestamps of the photos this timeline represents. */
        stamps: Float64Array;
        /** True when `stamps` covers only what the map currently shows. */
        mapConstrained?: boolean;
        /** User picked a time range; `null, null` means "no filter". Never fired by data changes. */
        onTimeRangeChange: (startMs: number | null, endMs: number | null) => void;
        onShowAll?: () => void;
        t: Translations;
        locale: Locale;
    }>();

    // === Constants ===
    const HOUR = 60 * 60 * 1000;
    const DAY = 24 * HOUR;
    const MIN_DURATION = 5 * 60 * 1000; // 5 minutes minimum

    // Duration presets (0 = All)
    let durationPresets = $derived([
        { label: '1h', value: HOUR },
        { label: '6h', value: 6 * HOUR },
        { label: '1d', value: DAY },
        { label: '7d', value: 7 * DAY },
        { label: '30d', value: 30 * DAY },
        { label: t.timeline.all, value: 0 },
    ]);

    // === Full time range of the given stamps ===
    let total = $derived(stamps.length);
    let rangeMin = $derived(total > 0 ? stamps[0] : 0);
    let rangeMax = $derived(total > 0 ? stamps[total - 1] : 0);
    let rangeDurationMs = $derived(rangeMax - rangeMin);

    // === Core state: view duration and center ===
    let viewDurationMs = $state(DAY);
    let viewCenterMs = $state(0);

    // Track if user has customized duration via pinch zoom
    let isCustomDuration = $state(false);

    // True from the first user gesture until the selection covers everything again.
    // While false, the view simply shows the whole range and nothing is reported upward.
    let userActive = $state(false);

    // New data: show all of it, unless the user is holding a selection.
    $effect(() => {
        stamps;
        untrack(() => {
            if (total === 0) return;
            if (!userActive) {
                viewCenterMs = rangeMin + rangeDurationMs / 2;
                viewDurationMs = rangeDurationMs;
                isCustomDuration = false;
            } else {
                viewDurationMs = Math.min(viewDurationMs, Math.max(rangeDurationMs, MIN_DURATION));
                clampViewCenter();
            }
        });
    });

    // === Derived: view window ===
    let viewWindow = $derived.by(() => {
        const halfDuration = viewDurationMs / 2;
        let start = viewCenterMs - halfDuration;
        let end = viewCenterMs + halfDuration;

        if (start < rangeMin) {
            start = rangeMin;
            end = Math.min(rangeMax, start + viewDurationMs);
        }
        if (end > rangeMax) {
            end = rangeMax;
            start = Math.max(rangeMin, end - viewDurationMs);
        }

        return { startMs: start, endMs: end };
    });

    let coversAll = $derived(
        viewWindow.startMs <= rangeMin + 1000 && viewWindow.endMs >= rangeMax - 1000
    );

    // === Derived: position on overview (percentage) ===
    let overviewLeftPercent = $derived.by(() => {
        if (rangeDurationMs === 0) return 0;
        return ((viewWindow.startMs - rangeMin) / rangeDurationMs) * 100;
    });

    let overviewRightPercent = $derived.by(() => {
        if (rangeDurationMs === 0) return 100;
        return ((viewWindow.endMs - rangeMin) / rangeDurationMs) * 100;
    });

    let overviewWidthPercent = $derived(overviewRightPercent - overviewLeftPercent);

    const MIN_HANDLE_WIDTH_PX = 4; // 2px border on each side

    // Report the selection upward — only when the user made it. Once per frame:
    // the map query is cheap enough to follow the finger, and the map itself
    // drops intermediate requests while one is in flight.
    $effect(() => {
        const start = viewWindow.startMs;
        const end = viewWindow.endMs;
        const all = coversAll;
        if (!userActive) return;
        const raf = requestAnimationFrame(() => {
            if (all) {
                // Keep following the finger; only hand control back to the map
                // once the gesture is over.
                onTimeRangeChange(null, null);
                if (dragMode === 'none' && !isPinching) userActive = false;
            } else {
                onTimeRangeChange(start, end);
            }
        });
        return () => cancelAnimationFrame(raf);
    });

    // === Dragging state ===
    type DragMode = 'none' | 'overview-box' | 'detail-pan';
    let dragMode = $state<DragMode>('none');
    let overviewTrack: HTMLDivElement;
    let detailTrack: HTMLDivElement;
    let dragStartX = 0;
    let dragStartCenterMs = 0;

    // === Pinch zoom state ===
    let isPinching = $state(false);
    let pinchStartDistance = 0;
    let pinchStartDuration = 0;

    function clampCenter(center: number): number {
        const halfDuration = viewDurationMs / 2;
        return Math.max(rangeMin + halfDuration, Math.min(rangeMax - halfDuration, center));
    }

    // === Detail timeline: single finger pan ===
    function handleDetailTouchStart(e: TouchEvent) {
        if (e.touches.length === 1) {
            e.preventDefault();
            dragMode = 'detail-pan';
            dragStartX = e.touches[0].clientX;
            dragStartCenterMs = viewCenterMs;
            userActive = true;
        } else if (e.touches.length === 2) {
            e.preventDefault();
            isPinching = true;
            dragMode = 'none';
            pinchStartDistance = getPinchDistance(e.touches);
            pinchStartDuration = viewDurationMs;
            userActive = true;
        }
    }

    function handleDetailTouchMove(e: TouchEvent) {
        if (isPinching && e.touches.length === 2) {
            e.preventDefault();
            const currentDistance = getPinchDistance(e.touches);
            const scale = pinchStartDistance / currentDistance;

            let newDuration = pinchStartDuration * scale;
            newDuration = applySmartStep(newDuration);
            newDuration = Math.max(MIN_DURATION, Math.min(rangeDurationMs, newDuration));

            viewDurationMs = newDuration;
            isCustomDuration = true;
            clampViewCenter();
        } else if (dragMode === 'detail-pan' && e.touches.length === 1) {
            e.preventDefault();
            const deltaX = e.touches[0].clientX - dragStartX;
            const width = detailSize.w;
            if (width <= 0) return;

            // Moving finger right = earlier time (scroll left)
            const pxPerMs = width / viewDurationMs;
            viewCenterMs = clampCenter(dragStartCenterMs - deltaX / pxPerMs);
        }
    }

    function handleDetailTouchEnd(e: TouchEvent) {
        if (e.touches.length === 0) {
            dragMode = 'none';
            isPinching = false;
        } else if (e.touches.length === 1 && isPinching) {
            isPinching = false;
            dragMode = 'detail-pan';
            dragStartX = e.touches[0].clientX;
            dragStartCenterMs = viewCenterMs;
        }
    }

    // === Overview timeline: tap to jump, drag box to move ===
    function handleOverviewTouchStart(e: TouchEvent) {
        if (e.touches.length !== 1) return;
        e.preventDefault();
        e.stopPropagation();

        const touch = e.touches[0];
        const rect = overviewTrack.getBoundingClientRect();
        const touchPercent = ((touch.clientX - rect.left) / rect.width) * 100;

        // Hot zone: 50px each side of the box
        const hotZonePercent = (50 / rect.width) * 100;

        if (touchPercent >= overviewLeftPercent - hotZonePercent &&
            touchPercent <= overviewRightPercent + hotZonePercent) {
            dragMode = 'overview-box';
            dragStartX = touch.clientX;
            dragStartCenterMs = viewCenterMs;
        } else {
            viewCenterMs = clampCenter(rangeMin + (touchPercent / 100) * rangeDurationMs);
        }

        userActive = true;
    }

    function handleOverviewTouchMove(e: TouchEvent) {
        if (dragMode !== 'overview-box') return;
        if (e.touches.length !== 1) return;
        e.preventDefault();

        const deltaX = e.touches[0].clientX - dragStartX;
        const width = overviewSize.w;
        if (width <= 0) return;
        viewCenterMs = clampCenter(dragStartCenterMs + (deltaX / width) * rangeDurationMs);
    }

    function handleOverviewTouchEnd() {
        dragMode = 'none';
    }

    // Mouse support for overview (click to jump)
    function handleOverviewClick(e: MouseEvent) {
        const rect = overviewTrack.getBoundingClientRect();
        const clickPercent = ((e.clientX - rect.left) / rect.width) * 100;

        if (clickPercent < overviewLeftPercent || clickPercent > overviewRightPercent) {
            viewCenterMs = clampCenter(rangeMin + (clickPercent / 100) * rangeDurationMs);
            userActive = true;
        }
    }

    // === Duration preset buttons ===
    function selectDurationPreset(durationMs: number) {
        if (durationMs === 0) {
            viewDurationMs = rangeDurationMs;
            viewCenterMs = rangeMin + rangeDurationMs / 2;
        } else {
            viewDurationMs = Math.min(durationMs, rangeDurationMs);
            clampViewCenter();
        }
        isCustomDuration = false;
        userActive = true;
    }

    function isPresetSelected(presetValue: number): boolean {
        if (isCustomDuration) return false;
        if (presetValue === 0) {
            return Math.abs(viewDurationMs - rangeDurationMs) < 1000;
        }
        return Math.abs(viewDurationMs - presetValue) < 1000;
    }

    // === Helper functions ===
    function getPinchDistance(touches: TouchList): number {
        const dx = touches[0].clientX - touches[1].clientX;
        const dy = touches[0].clientY - touches[1].clientY;
        return Math.sqrt(dx * dx + dy * dy);
    }

    /** Snap a pinch-zoomed duration to a step that fits its scale. */
    function applySmartStep(newDuration: number): number {
        if (newDuration >= 7 * DAY) {
            return Math.round(newDuration / DAY) * DAY;
        } else if (newDuration >= DAY) {
            return Math.round(newDuration / (6 * HOUR)) * (6 * HOUR);
        } else if (newDuration >= 6 * HOUR) {
            return Math.round(newDuration / HOUR) * HOUR;
        } else if (newDuration >= HOUR) {
            return Math.round(newDuration / (15 * 60 * 1000)) * (15 * 60 * 1000);
        } else {
            return Math.round(newDuration / (5 * 60 * 1000)) * (5 * 60 * 1000);
        }
    }

    function clampViewCenter() {
        viewCenterMs = clampCenter(viewCenterMs);
    }

    function formatDuration(ms: number): string {
        if (ms >= DAY) {
            return `${Math.round(ms / DAY)}d`;
        } else if (ms >= HOUR) {
            return `${Math.round(ms / HOUR)}h`;
        } else {
            return `${Math.round(ms / (60 * 1000))}m`;
        }
    }

    // Timestamps are wall-clock ms, so format them as UTC to show the clock reading as taken.
    function formatDate(ms: number): string {
        return new Date(ms).toLocaleDateString(locale, {
            month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC'
        });
    }

    function formatDateTimeShort(ms: number): string {
        const d = new Date(ms);
        return d.toLocaleDateString(locale, { month: 'short', day: 'numeric', timeZone: 'UTC' }) + ' ' +
               d.toLocaleTimeString(locale, { hour: 'numeric', minute: '2-digit', timeZone: 'UTC' });
    }

    export function resetSelection() {
        viewDurationMs = rangeDurationMs;
        viewCenterMs = rangeMin + rangeDurationMs / 2;
        isCustomDuration = false;
        userActive = false;
        onTimeRangeChange(null, null);
        onShowAll?.();
    }

    // === Counts and edge stacking: binary searches only ===
    let counts = $derived(windowCounts(stamps, viewWindow.startMs, viewWindow.endMs));

    // Two photos collapse into one edge line when closer than one pixel of time.
    const EDGE_LINE_CAP = 200;
    let edgeLines = $derived.by(() => {
        const msPerPixel = viewDurationMs / 350;
        return {
            left: distinctLines(stamps, 0, counts.left, msPerPixel, EDGE_LINE_CAP),
            right: distinctLines(stamps, total - counts.right, total, msPerPixel, EDGE_LINE_CAP),
        };
    });

    // === Canvas drawing ===
    let overviewCanvas: HTMLCanvasElement;
    let detailCanvas: HTMLCanvasElement;
    // CSS sizes, kept current by one ResizeObserver per canvas.
    let overviewSize = $state({ w: 0, h: 0 });
    let detailSize = $state({ w: 0, h: 0 });

    function trackSize(node: HTMLElement, report: (w: number, h: number) => void) {
        const ro = new ResizeObserver((entries) => {
            const r = entries[0]?.contentRect;
            if (r) report(r.width, r.height);
        });
        ro.observe(node);
        return { destroy: () => ro.disconnect() };
    }

    function getAccentColor(): string {
        if (typeof document === 'undefined') return '#fbbf24';
        return getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#fbbf24';
    }

    /** Size the backing store only when it actually changed, then clear. */
    function prepare(canvas: HTMLCanvasElement | undefined, w: number, h: number): CanvasRenderingContext2D | null {
        if (!canvas || w <= 0 || h <= 0) return null;
        const ctx = canvas.getContext('2d');
        if (!ctx) return null;
        const dpr = window.devicePixelRatio || 1;
        const pw = Math.round(w * dpr);
        const ph = Math.round(h * dpr);
        if (canvas.width !== pw) canvas.width = pw;
        if (canvas.height !== ph) canvas.height = ph;
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, w, h);
        return ctx;
    }

    // One line per occupied pixel column; its opacity shows how many photos
    // share that column, so a dense library still reads as a distribution
    // instead of a solid block.
    const DENSITY_LEVELS = 5;
    function strokeDensity(ctx: CanvasRenderingContext2D, counts: Uint32Array, h: number, color: string) {
        let max = 0;
        for (let x = 0; x < counts.length; x++) if (counts[x] > max) max = counts[x];
        if (max === 0) return;
        const logMax = Math.log1p(max);
        ctx.strokeStyle = color;
        ctx.lineWidth = 1;
        // Group columns by opacity level: a handful of strokes, not one per column.
        for (let level = 1; level <= DENSITY_LEVELS; level++) {
            ctx.beginPath();
            let any = false;
            for (let x = 0; x < counts.length; x++) {
                const c = counts[x];
                if (c === 0) continue;
                const l = max === 1 ? DENSITY_LEVELS : Math.max(1, Math.ceil((Math.log1p(c) / logMax) * DENSITY_LEVELS));
                if (l !== level) continue;
                ctx.moveTo(x + 0.5, 0);
                ctx.lineTo(x + 0.5, h);
                any = true;
            }
            if (!any) continue;
            ctx.globalAlpha = 0.15 + 0.75 * (level / DENSITY_LEVELS);
            ctx.stroke();
        }
        ctx.globalAlpha = 1;
    }

    function drawOverview() {
        const { w, h } = overviewSize;
        const ctx = prepare(overviewCanvas, w, h);
        if (!ctx) return;
        strokeDensity(ctx, columnCounts(stamps, rangeMin, rangeMax, w), h, getAccentColor());
    }

    function drawEdgeStack(
        ctx: CanvasRenderingContext2D,
        side: 'left' | 'right',
        lines: number,
        count: number,
        w: number,
        h: number,
        color: string
    ) {
        if (lines <= 0) return;
        // Tightly packed, anchored to the edge, at most 20% of the width.
        const lineCount = Math.min(lines, Math.floor(w * 0.2));

        ctx.strokeStyle = color;
        ctx.globalAlpha = 0.7;
        ctx.lineWidth = 1;
        ctx.beginPath();
        for (let i = 0; i < lineCount; i++) {
            const x = side === 'left' ? i + 0.5 : w - 1 - i + 0.5;
            ctx.moveTo(x, 0);
            ctx.lineTo(x, h);
        }
        ctx.stroke();

        // Label shows the number of photos, not the number of lines.
        const label = `${count}`;
        ctx.font = 'bold 10px sans-serif';
        const textWidth = ctx.measureText(label).width;
        const labelX = side === 'left' ? lineCount + 4 : w - lineCount - textWidth - 4;

        ctx.globalAlpha = 0.8;
        ctx.fillStyle = 'black';
        ctx.fillRect(labelX - 2, h - 14, textWidth + 4, 12);

        ctx.globalAlpha = 1;
        ctx.fillStyle = color;
        ctx.textAlign = 'left';
        ctx.fillText(label, labelX, h - 4);
    }

    function drawDetail() {
        const { w, h } = detailSize;
        const ctx = prepare(detailCanvas, w, h);
        if (!ctx) return;
        const color = getAccentColor();
        strokeDensity(ctx, columnCounts(stamps, viewWindow.startMs, viewWindow.endMs, w), h, color);
        drawEdgeStack(ctx, 'left', edgeLines.left, counts.left, w, h, color);
        drawEdgeStack(ctx, 'right', edgeLines.right, counts.right, w, h, color);
    }

    // The overview only depends on the data and its size: dragging never redraws it.
    let overviewRaf = 0;
    $effect(() => {
        stamps; overviewSize; rangeMin; rangeMax;
        cancelAnimationFrame(overviewRaf);
        overviewRaf = requestAnimationFrame(drawOverview);
        return () => cancelAnimationFrame(overviewRaf);
    });

    let detailRaf = 0;
    $effect(() => {
        stamps; detailSize; viewWindow; counts; edgeLines;
        cancelAnimationFrame(detailRaf);
        detailRaf = requestAnimationFrame(drawDetail);
        return () => cancelAnimationFrame(detailRaf);
    });

    // Global touch handlers for drag continuation
    function handleGlobalTouchMove(e: TouchEvent) {
        if (dragMode === 'overview-box') {
            handleOverviewTouchMove(e);
        }
    }

    function handleGlobalTouchEnd() {
        if (dragMode === 'overview-box') {
            handleOverviewTouchEnd();
        }
    }
</script>

<svelte:window on:touchmove={handleGlobalTouchMove} on:touchend={handleGlobalTouchEnd} />

<div class="timeline-container theme-bg-primary px-4 py-3">
    <!-- Header -->
    <div class="flex items-center justify-between mb-2">
        <div class="text-[10px] theme-text-secondary">
            {formatDateTimeShort(viewWindow.startMs)} - {formatDateTimeShort(viewWindow.endMs)}
        </div>
        <div class="flex items-center gap-2">
            <span class="text-[10px] theme-text-secondary">
                {counts.inside}/{total}
            </span>
        </div>
    </div>

    <!-- Duration presets -->
    <div class="flex items-center gap-2 mb-2">
        <span class="text-[10px] theme-text-muted">{t.timeline.range}:</span>
        <div class="flex gap-1">
            {#each durationPresets as preset}
                <button
                    onclick={() => selectDurationPreset(preset.value)}
                    class="px-2 py-0.5 text-[10px] rounded transition-colors
                        {isPresetSelected(preset.value)
                            ? 'bg-[var(--accent)] text-black'
                            : 'theme-bg-secondary theme-text-muted hover:theme-bg-tertiary'}"
                >
                    {preset.label}
                </button>
            {/each}
            {#if isCustomDuration}
                <span class="px-2 py-0.5 text-[10px] rounded bg-[var(--accent)] text-black">
                    {formatDuration(viewDurationMs)}
                </span>
            {/if}
        </div>
    </div>

    <!-- === Overview Timeline (top) === -->
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="relative h-8 mx-2 mb-3">
        <div
            bind:this={overviewTrack}
            class="absolute inset-0 theme-bg-primary rounded-lg overflow-hidden"
            onclick={handleOverviewClick}
            ontouchstart={handleOverviewTouchStart}
        >
            <!-- Photo lines (canvas) -->
            <canvas bind:this={overviewCanvas} use:trackSize={(w, h) => { overviewSize = { w, h }; }} class="absolute inset-0 w-full h-full"></canvas>

            <!-- Dimmed areas -->
            <div class="absolute top-0 bottom-0 left-0 bg-black/50 dark:bg-black/70" style="width: {overviewLeftPercent}%"></div>
            <div class="absolute top-0 bottom-0 right-0 bg-black/50 dark:bg-black/70" style="width: {100 - overviewRightPercent}%"></div>
        </div>

        <!-- Yellow selection box (draggable) - outside overflow-hidden container -->
        <div
            class="absolute top-0 bottom-0 border-2 border-[var(--accent)] rounded pointer-events-none"
            style="left: {overviewLeftPercent}%; width: {overviewWidthPercent}%; min-width: {MIN_HANDLE_WIDTH_PX}px"
        ></div>
    </div>

    <!-- Overview labels -->
    <div class="flex justify-between text-[10px] theme-text-muted mb-3 mx-2">
        <span>{formatDate(rangeMin)}</span>
        {#if mapConstrained}
            <span class="text-[var(--accent)]">Map View</span>
        {/if}
        <span>{formatDate(rangeMax)}</span>
    </div>

    <!-- === Detail Timeline (bottom) === -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="relative h-16 mx-2">
        <div
            bind:this={detailTrack}
            class="absolute inset-0 theme-bg-secondary rounded-lg overflow-hidden border-2 border-[var(--accent)] detail-dots"
            ontouchstart={handleDetailTouchStart}
            ontouchmove={handleDetailTouchMove}
            ontouchend={handleDetailTouchEnd}
        >
            <!-- Photo lines (canvas) -->
            <canvas bind:this={detailCanvas} use:trackSize={(w, h) => { detailSize = { w, h }; }} class="absolute inset-0 w-full h-full"></canvas>
        </div>
    </div>

    <!-- Detail labels -->
    <div class="flex justify-between text-[10px] theme-text-muted mt-1 mx-2">
        <span>{formatDateTimeShort(viewWindow.startMs)}</span>
        <span>{formatDateTimeShort(viewWindow.endMs)}</span>
    </div>
</div>

<style>
    .timeline-container {
        user-select: none;
        touch-action: none;
    }
    /* Touchpad-style dot grid: static, so it is a CSS background, not canvas work. */
    .detail-dots {
        background-image: radial-gradient(circle, color-mix(in srgb, var(--accent) 14%, transparent) 1px, transparent 1.3px);
        background-size: 10px 10px;
        background-position: 0 0;
    }
</style>
