<script lang="ts">
    import { untrack } from "svelte";
    import type { Translations, Locale } from "../../shared/i18n/types";
    import { columnCounts, windowCounts } from "../../shared/utils/timeline-index";

    /**
     * Desktop timeline over a sorted Float64Array of wall-clock timestamps.
     *
     * Data flows one way. `stamps` is what the map currently shows; changing it
     * never reports a time range. Only user input (drag, wheel, duration
     * buttons, a box selection handed in as `externalTimeRange`) does, at most
     * once per frame. A selection that covers everything again reports
     * `(null, null)`, which clears the filter.
     */
    let {
        stamps,
        mapConstrained = false,
        externalTimeRange,
        onTimeRangeChange,
        onExternalRangeConsumed,
        onShowAll,
        t,
        locale = 'en' as Locale,
    } = $props<{
        stamps: Float64Array;
        mapConstrained?: boolean;
        externalTimeRange?: { startMs: number; endMs: number } | null;
        onTimeRangeChange: (startMs: number | null, endMs: number | null) => void;
        onExternalRangeConsumed?: () => void;
        onShowAll?: () => void;
        t: Translations;
        locale?: Locale;
    }>();

    // Full range of the data on display.
    let rangeMin = $derived(stamps.length > 0 ? stamps[0] : 0);
    let rangeMax = $derived(stamps.length > 0 ? stamps[stamps.length - 1] : 0);

    // === Layer 1: range selection (handles on the overview) ===
    let leftPercent = $state(0);
    let rightPercent = $state(100);

    let isZoomed = $derived(leftPercent > 0 || rightPercent < 100);

    let selectedRange = $derived.by(() => {
        const total = rangeMax - rangeMin;
        const startMs = rangeMin + (leftPercent / 100) * total;
        const endMs = rangeMin + (rightPercent / 100) * total;
        return { startMs, endMs, durationMs: endMs - startMs };
    });

    // === Layer 2: fixed-width window inside the selected range ===
    let durationOptions = $derived([
        { label: '1h', value: 60 * 60 * 1000 },
        { label: '6h', value: 6 * 60 * 60 * 1000 },
        { label: '1d', value: 24 * 60 * 60 * 1000 },
        { label: '7d', value: 7 * 24 * 60 * 60 * 1000 },
        { label: '30d', value: 30 * 24 * 60 * 60 * 1000 },
        { label: t.timeline.all, value: 0 },
    ]);
    let selectedDuration = $state(0); // 0 = the whole selected range
    let windowPosition = $state(0);   // 0-100 within the selected range

    /** The time window on display. */
    let viewWindow = $derived.by(() => {
        if (!isZoomed || selectedDuration === 0) {
            return { startMs: selectedRange.startMs, endMs: selectedRange.endMs };
        }
        const windowMs = Math.min(selectedDuration, selectedRange.durationMs);
        const offset = (windowPosition / 100) * (selectedRange.durationMs - windowMs);
        return {
            startMs: selectedRange.startMs + offset,
            endMs: selectedRange.startMs + offset + windowMs,
        };
    });

    // === Reporting: user input only, once per frame ===
    let userActive = false;
    let reportRaf = 0;

    function userChanged() {
        userActive = true;
        if (reportRaf) return;
        reportRaf = requestAnimationFrame(() => {
            reportRaf = 0;
            if (!isZoomed) {
                userActive = false;
                onTimeRangeChange(null, null);
            } else {
                onTimeRangeChange(viewWindow.startMs, viewWindow.endMs);
            }
        });
    }

    function setSelection(left: number, right: number) {
        leftPercent = left;
        rightPercent = right;
        // A new overview selection starts over in the zoomed layer.
        selectedDuration = 0;
        windowPosition = 0;
        userChanged();
    }

    /** Percent selection covering `[startMs, endMs]` in the current range (at least 1% wide). */
    function percentsFor(startMs: number, endMs: number): [number, number] {
        const total = rangeMax - rangeMin;
        if (total <= 0) return [0, 100];
        let left = Math.max(0, Math.min(100, ((startMs - rangeMin) / total) * 100));
        let right = Math.max(0, Math.min(100, ((endMs - rangeMin) / total) * 100));
        if (right - left < 1) {
            const center = Math.max(0.5, Math.min(99.5, (left + right) / 2));
            left = center - 0.5;
            right = center + 0.5;
        }
        return [left, right];
    }

    // New data. Without a user selection the timeline simply shows all of it;
    // with one, keep the selected time span (the percentages move, the times do not).
    let lastStamps: Float64Array | null = null;
    let lastSelection = { startMs: 0, endMs: 0 };
    $effect(() => {
        const sel = selectedRange;
        untrack(() => { lastSelection = { startMs: sel.startMs, endMs: sel.endMs }; });
    });
    $effect.pre(() => {
        const current = stamps;
        if (current === lastStamps) return;
        const first = lastStamps === null;
        lastStamps = current;
        if (first) return;
        untrack(() => {
            if (userActive && isZoomed) {
                const [l, r] = percentsFor(lastSelection.startMs, lastSelection.endMs);
                leftPercent = l;
                rightPercent = r;
            } else {
                leftPercent = 0;
                rightPercent = 100;
                windowPosition = 0;
                selectedDuration = 0;
            }
        });
    });

    // A box drawn on the map arrives as a time range to select.
    $effect(() => {
        const ext = externalTimeRange;
        if (!ext) return;
        untrack(() => {
            const [l, r] = percentsFor(ext.startMs, ext.endMs);
            setSelection(l, r);
            onExternalRangeConsumed?.();
        });
    });

    // === Dragging ===
    type DragMode = 'none' | 'left' | 'right' | 'middle' | 'window';
    let dragMode = $state<DragMode>('none');
    let sliderTrack: HTMLDivElement;
    let zoomedTrack: HTMLDivElement | undefined = $state();
    let dragStartX = 0;
    let dragStartLeft = 0;
    let dragStartRight = 0;
    let dragStartWindow = 0;
    let dragTrackWidth = 1;

    function handleMouseDown(e: MouseEvent, mode: DragMode) {
        e.preventDefault();
        e.stopPropagation();
        dragMode = mode;
        dragStartX = e.clientX;
        dragStartLeft = leftPercent;
        dragStartRight = rightPercent;
        dragStartWindow = windowPosition;
        const track = mode === 'window' ? zoomedTrack : sliderTrack;
        dragTrackWidth = Math.max(1, track?.getBoundingClientRect().width ?? 1);
    }

    function handleMouseMove(e: MouseEvent) {
        if (dragMode === 'none') return;
        const deltaPercent = ((e.clientX - dragStartX) / dragTrackWidth) * 100;

        if (dragMode === 'window') {
            const maxMovement = 100 - windowWidthPercent;
            if (maxMovement <= 0) return;
            windowPosition = Math.max(0, Math.min(100, dragStartWindow + (deltaPercent / maxMovement) * 100));
            userChanged();
        } else if (dragMode === 'left') {
            setSelection(Math.max(0, Math.min(dragStartLeft + deltaPercent, rightPercent - 1)), rightPercent);
        } else if (dragMode === 'right') {
            setSelection(leftPercent, Math.max(leftPercent + 1, Math.min(dragStartRight + deltaPercent, 100)));
        } else if (dragMode === 'middle') {
            const width = dragStartRight - dragStartLeft;
            let newLeft = dragStartLeft + deltaPercent;
            if (newLeft < 0) newLeft = 0;
            if (newLeft + width > 100) newLeft = 100 - width;
            setSelection(newLeft, newLeft + width);
        }
    }

    function handleMouseUp() {
        dragMode = 'none';
    }

    // === Mouse wheel ===
    function handleOverviewWheel(e: WheelEvent) {
        e.preventDefault();
        const rect = sliderTrack.getBoundingClientRect();
        const mousePercent = ((e.clientX - rect.left) / rect.width) * 100;
        const zoomIn = e.deltaY < 0;
        const zoomAmount = 1;

        const currentWidth = rightPercent - leftPercent;
        const center = (leftPercent + rightPercent) / 2;

        if (zoomIn) {
            if (currentWidth <= 5) return;
            const newCenter = center + (mousePercent - center) * 0.3;
            const newWidth = Math.max(5, currentWidth - zoomAmount * 2);
            let left = Math.max(0, newCenter - newWidth / 2);
            let right = Math.min(100, newCenter + newWidth / 2);
            if (left === 0) right = newWidth;
            if (right === 100) left = 100 - newWidth;
            setSelection(left, right);
        } else {
            if (currentWidth >= 100) return;
            const newWidth = Math.min(100, currentWidth + zoomAmount * 2);
            const expand = (newWidth - currentWidth) / 2;
            let left = Math.max(0, leftPercent - expand);
            let right = Math.min(100, rightPercent + expand);
            if (right - left >= 98) {
                left = 0;
                right = 100;
            }
            setSelection(left, right);
        }
    }

    function handleZoomedWheel(e: WheelEvent) {
        e.preventDefault();
        if (selectedDuration === 0) return;
        const delta = e.deltaY > 0 ? 0.3 : -0.3;
        windowPosition = Math.max(0, Math.min(100, windowPosition + delta));
        userChanged();
    }

    function pickDuration(value: number) {
        selectedDuration = value;
        if (value === 0) windowPosition = 0;
        userChanged();
    }

    function resetSelection() {
        leftPercent = 0;
        rightPercent = 100;
        windowPosition = 0;
        selectedDuration = 0;
        userChanged();
        onShowAll?.();
    }

    // === Formatting (wall-clock ms: format as UTC) ===
    function formatDate(ms: number): string {
        return new Date(ms).toLocaleDateString(locale, { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' });
    }

    function formatDateTime(ms: number): string {
        return `${formatDate(ms)} ${new Date(ms).toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit', timeZone: 'UTC' })}`;
    }

    let photosInWindow = $derived(windowCounts(stamps, viewWindow.startMs, viewWindow.endMs).inside);

    let windowWidthPercent = $derived.by(() => {
        if (selectedDuration === 0 || selectedRange.durationMs === 0) return 100;
        return Math.min(100, (selectedDuration / selectedRange.durationMs) * 100);
    });

    let windowLeftPercent = $derived((windowPosition / 100) * (100 - windowWidthPercent));

    // === Canvases ===
    let overviewCanvas: HTMLCanvasElement | undefined = $state();
    let zoomedCanvas: HTMLCanvasElement | undefined = $state();
    let overviewSize = $state({ w: 0, h: 0 });
    let zoomedSize = $state({ w: 0, h: 0 });

    function trackSize(node: HTMLElement, set: (w: number, h: number) => void) {
        const ro = new ResizeObserver((entries) => {
            const r = entries[0]?.contentRect;
            if (r) set(Math.floor(r.width), Math.floor(r.height));
        });
        ro.observe(node);
        return { destroy: () => ro.disconnect() };
    }

    function accentColor(): string {
        return getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#fbbf24';
    }

    /** One 1px line per pixel column that has photos; busier columns are more opaque. */
    function drawColumns(
        canvas: HTMLCanvasElement | undefined,
        size: { w: number; h: number },
        data: Float64Array,
        startMs: number,
        endMs: number,
    ) {
        if (!canvas || size.w <= 0 || size.h <= 0) return;
        const dpr = window.devicePixelRatio || 1;
        const bw = Math.round(size.w * dpr);
        const bh = Math.round(size.h * dpr);
        if (canvas.width !== bw) canvas.width = bw;
        if (canvas.height !== bh) canvas.height = bh;
        const ctx = canvas.getContext('2d');
        if (!ctx) return;
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, size.w, size.h);
        ctx.fillStyle = accentColor();
        const counts = columnCounts(data, startMs, endMs, size.w);
        for (let x = 0; x < counts.length; x++) {
            const c = counts[x];
            if (c === 0) continue;
            ctx.globalAlpha = Math.min(1, 0.4 + 0.15 * Math.log2(c));
            ctx.fillRect(x, 0, 1, size.h);
        }
        ctx.globalAlpha = 1;
    }

    // Overview: redraw only when the data or its size changes.
    $effect(() => {
        drawColumns(overviewCanvas, overviewSize, stamps, rangeMin, rangeMax);
    });

    // Zoomed layer: follows the selected range, at most once per frame.
    let zoomedRaf = 0;
    $effect(() => {
        const canvas = zoomedCanvas;
        const size = zoomedSize;
        const data = stamps;
        const { startMs, endMs } = selectedRange;
        if (!canvas) return;
        cancelAnimationFrame(zoomedRaf);
        zoomedRaf = requestAnimationFrame(() => drawColumns(canvas, size, data, startMs, endMs));
        return () => cancelAnimationFrame(zoomedRaf);
    });
</script>

<svelte:window onmousemove={handleMouseMove} onmouseup={handleMouseUp} />

<div class="timeline-container theme-bg-overlay backdrop-blur-sm px-4 py-3">
    <!-- Header -->
    <div class="flex items-center justify-between mb-2">
        <div class="text-xs theme-text-secondary">
            {formatDateTime(viewWindow.startMs)} - {formatDateTime(viewWindow.endMs)}
        </div>
        <div class="flex items-center gap-2">
            <span class="text-xs theme-text-secondary">
                {photosInWindow}/{stamps.length}
            </span>
            {#if isZoomed || mapConstrained}
                <button onclick={resetSelection} class="text-xs text-[var(--accent)] hover:text-[var(--accent-hover)]">
                    {mapConstrained && !isZoomed ? t.timeline.all : t.timeline.reset}
                </button>
            {/if}
        </div>
    </div>

    {#if isZoomed}
        <!-- === Zoomed View (full width) === -->
        <div class="mb-3">
            <!-- Duration selector -->
            <div class="flex items-center gap-2 mb-2">
                <span class="text-[10px] theme-text-muted">{t.timeline.range}:</span>
                <div class="flex gap-1">
                    {#each durationOptions as option}
                        <button
                            onclick={() => pickDuration(option.value)}
                            class="px-2 py-0.5 text-[10px] rounded transition-colors
                                {selectedDuration === option.value
                                    ? 'bg-[var(--accent)] text-black'
                                    : 'theme-bg-secondary theme-text-muted hover:theme-bg-tertiary'}"
                        >
                            {option.label}
                        </button>
                    {/each}
                </div>
            </div>

            <!-- Zoomed slider track -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div bind:this={zoomedTrack} class="relative h-10 theme-bg-secondary rounded-lg overflow-hidden" onwheel={handleZoomedWheel}>
                <!-- Photo lines (canvas) -->
                <canvas bind:this={zoomedCanvas} use:trackSize={(w, h) => { zoomedSize = { w, h }; }} class="absolute inset-0 w-full h-full"></canvas>

                <!-- Fixed window (draggable) -->
                {#if selectedDuration !== 0}
                    <div
                        class="absolute top-0 bottom-0 bg-[var(--accent)]/20 border-2 border-[var(--accent)] rounded cursor-grab active:cursor-grabbing"
                        style="left: {windowLeftPercent}%; width: {windowWidthPercent}%"
                        onmousedown={(e) => handleMouseDown(e, 'window')}
                    >
                        <div class="absolute inset-0 flex items-center justify-center">
                            <div class="w-8 h-4 bg-[var(--accent)] rounded-full flex items-center justify-center">
                                <div class="w-4 h-0.5 bg-black/30 rounded"></div>
                            </div>
                        </div>
                    </div>
                {/if}
            </div>

            <!-- Zoomed range labels -->
            <div class="flex justify-between text-[10px] theme-text-muted mt-1">
                <span>{formatDate(selectedRange.startMs)}</span>
                <span>{formatDate(selectedRange.endMs)}</span>
            </div>
        </div>
    {/if}

    <!-- === Overview (full timeline with handles) === -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="relative h-16 mx-2" onwheel={handleOverviewWheel}>
        <!-- Track background -->
        <div
            bind:this={sliderTrack}
            class="absolute inset-0 theme-bg-primary rounded overflow-hidden"
        >
            <!-- Photo lines (canvas) -->
            <canvas bind:this={overviewCanvas} use:trackSize={(w, h) => { overviewSize = { w, h }; }} class="absolute inset-0 w-full h-full"></canvas>

            <!-- Dimmed areas -->
            <div class="absolute top-0 bottom-0 left-0 bg-black/50 dark:bg-black/70" style="width: {leftPercent}%"></div>
            <div class="absolute top-0 bottom-0 right-0 bg-black/50 dark:bg-black/70" style="width: {100 - rightPercent}%"></div>

            <!-- Selected region border -->
            <div
                class="absolute top-0 bottom-0 border-y-2 border-[var(--accent)] pointer-events-none"
                style="left: {leftPercent}%; right: {100 - rightPercent}%"
            ></div>
        </div>

        <!-- Draggable middle -->
        <div
            class="absolute top-0 bottom-0 cursor-grab active:cursor-grabbing z-10"
            style="left: {leftPercent}%; right: {100 - rightPercent}%"
            onmousedown={(e) => handleMouseDown(e, 'middle')}
        ></div>

        <!-- Left handle -->
        <div
            class="absolute top-0 bottom-0 w-5 cursor-ew-resize flex items-center justify-center z-20"
            style="left: {leftPercent}%; transform: translateX(-50%)"
            onmousedown={(e) => handleMouseDown(e, 'left')}
        >
            <div class="w-1.5 h-full bg-[var(--accent)] rounded-sm shadow-lg"></div>
        </div>

        <!-- Right handle -->
        <div
            class="absolute top-0 bottom-0 w-5 cursor-ew-resize flex items-center justify-center z-20"
            style="left: {rightPercent}%; transform: translateX(-50%)"
            onmousedown={(e) => handleMouseDown(e, 'right')}
        >
            <div class="w-1.5 h-full bg-[var(--accent)] rounded-sm shadow-lg"></div>
        </div>
    </div>

    <!-- Full range labels -->
    <div class="flex justify-between text-[10px] theme-text-muted mt-1 mx-2">
        <span>{formatDate(rangeMin)}</span>
        {#if mapConstrained}
            <span class="text-[var(--accent)]">Map View</span>
        {/if}
        <span>{formatDate(rangeMax)}</span>
    </div>
</div>

<style>
    .timeline-container {
        user-select: none;
    }
</style>
