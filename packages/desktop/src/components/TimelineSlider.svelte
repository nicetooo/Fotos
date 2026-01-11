<script lang="ts">
    let {
        photos,
        externalTimeRange,
        mapViewTimeRange,
        onTimeRangeChange,
        onExternalRangeConsumed,
        onMapRangeConsumed,
        onShowAll
    } = $props<{
        photos: any[];
        externalTimeRange?: { start: Date; end: Date } | null;
        mapViewTimeRange?: { start: Date; end: Date } | null;
        onTimeRangeChange: (start: Date, end: Date) => void;
        onExternalRangeConsumed?: () => void;
        onMapRangeConsumed?: () => void;
        onShowAll?: () => void;
    }>();

    // Parse dates from photos
    function parsePhotoDate(dateStr: string | null): Date | null {
        if (!dateStr) return null;
        const cleaned = dateStr.replace(/"/g, '').trim();
        const date = new Date(cleaned.replace(' ', 'T'));
        return isNaN(date.getTime()) ? null : date;
    }

    // Get time range from all photos
    let fullTimeRange = $derived.by(() => {
        const dates = photos
            .map(p => parsePhotoDate(p.metadata?.date_taken))
            .filter((d): d is Date => d !== null)
            .sort((a, b) => a.getTime() - b.getTime());

        if (dates.length === 0) {
            const now = new Date();
            return { min: now, max: now };
        }
        return { min: dates[0], max: dates[dates.length - 1] };
    });

    // Display time range (can be constrained by map view)
    let displayTimeRange = $state<{ min: Date; max: Date } | null>(null);

    // Actual time range used for display (map range or full range)
    let timeRange = $derived(displayTimeRange ?? fullTimeRange);

    // Is showing constrained range from map?
    let isMapConstrained = $derived(displayTimeRange !== null);

    // === Layer 1: Range selection (iPhone-style handles) ===
    let leftPercent = $state(0);
    let rightPercent = $state(100);

    // Is zoomed in (has a selection)?
    let isZoomed = $derived(leftPercent > 0 || rightPercent < 100);

    // Selected time range in milliseconds
    let selectedRange = $derived.by(() => {
        const total = timeRange.max.getTime() - timeRange.min.getTime();
        const startMs = timeRange.min.getTime() + (leftPercent / 100) * total;
        const endMs = timeRange.min.getTime() + (rightPercent / 100) * total;
        return { startMs, endMs, durationMs: endMs - startMs };
    });

    // === Layer 2: Fixed-width window slider (inside zoomed view) ===
    const durationOptions = [
        { label: '1h', value: 60 * 60 * 1000 },
        { label: '6h', value: 6 * 60 * 60 * 1000 },
        { label: '1d', value: 24 * 60 * 60 * 1000 },
        { label: '7d', value: 7 * 24 * 60 * 60 * 1000 },
        { label: '30d', value: 30 * 24 * 60 * 60 * 1000 },
        { label: 'All', value: 0 },
    ];

    // Load saved duration from localStorage, default to 1 hour
    const STORAGE_KEY = 'timeline-window-duration';
    let selectedDuration = $state((() => {
        if (typeof localStorage !== 'undefined') {
            const saved = localStorage.getItem(STORAGE_KEY);
            if (saved) {
                const value = parseInt(saved, 10);
                if (durationOptions.some(o => o.value === value)) {
                    return value;
                }
            }
        }
        return durationOptions[0].value;
    })());

    // Save duration when changed
    $effect(() => {
        if (typeof localStorage !== 'undefined') {
            localStorage.setItem(STORAGE_KEY, selectedDuration.toString());
        }
    });

    let windowPosition = $state(0); // 0-100 percentage within selected range

    // Reset selection only when photos are ADDED (e.g., after import)
    // Don't reset when photos are removed (delete operation)
    let prevPhotosLength = $state(0);
    $effect(() => {
        const len = photos.length;
        if (len > prevPhotosLength && prevPhotosLength > 0) {
            // Reset to full range only when new photos are added
            leftPercent = 0;
            rightPercent = 100;
            windowPosition = 0;
            displayTimeRange = null;
        }
        prevPhotosLength = len;
    });

    // Handle external time range (from map box selection)
    $effect(() => {
        if (!externalTimeRange) return;

        const totalMs = timeRange.max.getTime() - timeRange.min.getTime();
        if (totalMs === 0) return;

        const startMs = externalTimeRange.start.getTime();
        const endMs = externalTimeRange.end.getTime();

        // Convert to percentages
        let newLeft = ((startMs - timeRange.min.getTime()) / totalMs) * 100;
        let newRight = ((endMs - timeRange.min.getTime()) / totalMs) * 100;

        // Clamp to valid range
        newLeft = Math.max(0, Math.min(100, newLeft));
        newRight = Math.max(0, Math.min(100, newRight));

        // Ensure minimum width
        if (newRight - newLeft < 1) {
            const center = (newLeft + newRight) / 2;
            newLeft = Math.max(0, center - 0.5);
            newRight = Math.min(100, center + 0.5);
        }

        leftPercent = newLeft;
        rightPercent = newRight;
        windowPosition = 0;

        // Notify parent that we consumed the external range
        onExternalRangeConsumed?.();
    });

    // Handle map view time range (continuous sync from map pan/zoom)
    // This changes the TOTAL displayed timeline range, not the selection
    $effect(() => {
        if (!mapViewTimeRange) return;

        // Add some padding to the time range (10% on each side)
        const rangeMs = mapViewTimeRange.end.getTime() - mapViewTimeRange.start.getTime();
        const padding = rangeMs * 0.1;

        displayTimeRange = {
            min: new Date(mapViewTimeRange.start.getTime() - padding),
            max: new Date(mapViewTimeRange.end.getTime() + padding)
        };

        // Reset selection to full range when map constrains the view
        leftPercent = 0;
        rightPercent = 100;
    });

    // Actual viewing window (what photos to show)
    let viewWindow = $derived.by(() => {
        // If "All" is selected, show full range (selected or total)
        if (selectedDuration === 0) {
            if (isZoomed) {
                return {
                    start: new Date(selectedRange.startMs),
                    end: new Date(selectedRange.endMs)
                };
            } else {
                return {
                    start: timeRange.min,
                    end: timeRange.max
                };
            }
        }

        // Fixed window within range (zoomed or full)
        const baseStartMs = isZoomed ? selectedRange.startMs : timeRange.min.getTime();
        const baseDurationMs = isZoomed
            ? selectedRange.durationMs
            : timeRange.max.getTime() - timeRange.min.getTime();

        const windowMs = Math.min(selectedDuration, baseDurationMs);
        const maxOffset = baseDurationMs - windowMs;
        const offset = (windowPosition / 100) * maxOffset;

        return {
            start: new Date(baseStartMs + offset),
            end: new Date(baseStartMs + offset + windowMs)
        };
    });

    // Notify parent
    $effect(() => {
        onTimeRangeChange(viewWindow.start, viewWindow.end);
    });

    // === Dragging state ===
    type DragMode = 'none' | 'left' | 'right' | 'middle' | 'window';
    let dragMode = $state<DragMode>('none');
    let sliderTrack: HTMLDivElement;
    let zoomedTrack: HTMLDivElement;
    let dragStartX = 0;
    let dragStartLeft = 0;
    let dragStartRight = 0;
    let dragStartWindow = 0;

    function handleMouseDown(e: MouseEvent, mode: DragMode) {
        e.preventDefault();
        e.stopPropagation();
        dragMode = mode;
        dragStartX = e.clientX;
        dragStartLeft = leftPercent;
        dragStartRight = rightPercent;
        dragStartWindow = windowPosition;

        // User is interacting with timeline - stop map sync
        onMapRangeConsumed?.();
    }

    function handleMouseMove(e: MouseEvent) {
        if (dragMode === 'none') return;

        if (dragMode === 'window' && zoomedTrack) {
            // Drag fixed window in zoomed view (1:1 mouse tracking)
            const rect = zoomedTrack.getBoundingClientRect();
            const deltaPercent = ((e.clientX - dragStartX) / rect.width) * 100;
            // Convert to windowPosition space (compensate for window size)
            const maxMovement = 100 - windowWidthPercent;
            if (maxMovement > 0) {
                const deltaWindowPos = (deltaPercent / maxMovement) * 100;
                let newPos = dragStartWindow + deltaWindowPos;
                windowPosition = Math.max(0, Math.min(100, newPos));
            }
        } else if (sliderTrack) {
            // Drag handles in overview
            const rect = sliderTrack.getBoundingClientRect();
            const deltaPercent = ((e.clientX - dragStartX) / rect.width) * 100;

            if (dragMode === 'left') {
                let newLeft = dragStartLeft + deltaPercent;
                newLeft = Math.max(0, Math.min(newLeft, rightPercent - 1));
                leftPercent = newLeft;
            } else if (dragMode === 'right') {
                let newRight = dragStartRight + deltaPercent;
                newRight = Math.max(leftPercent + 1, Math.min(newRight, 100));
                rightPercent = newRight;
            } else if (dragMode === 'middle') {
                const width = dragStartRight - dragStartLeft;
                let newLeft = dragStartLeft + deltaPercent;
                let newRight = dragStartRight + deltaPercent;

                if (newLeft < 0) { newLeft = 0; newRight = width; }
                if (newRight > 100) { newRight = 100; newLeft = 100 - width; }

                leftPercent = newLeft;
                rightPercent = newRight;
            }
        }
    }

    function handleMouseUp() {
        dragMode = 'none';
    }

    // === Mouse wheel handlers ===
    function handleOverviewWheel(e: WheelEvent) {
        e.preventDefault();

        // User is interacting with timeline - stop map sync
        onMapRangeConsumed?.();

        // Get mouse position relative to track
        const rect = sliderTrack.getBoundingClientRect();
        const mousePercent = ((e.clientX - rect.left) / rect.width) * 100;

        // Determine zoom direction (negative deltaY = scroll up = zoom in)
        const zoomIn = e.deltaY < 0;
        const zoomAmount = 1; // percentage change per scroll (reduced for smoother control)

        const currentWidth = rightPercent - leftPercent;
        const center = (leftPercent + rightPercent) / 2;

        if (zoomIn) {
            // Zoom in: shrink selection, center towards mouse position
            if (currentWidth <= 5) return; // minimum 5% selection

            // Bias center towards mouse position
            const bias = 0.3;
            const newCenter = center + (mousePercent - center) * bias;
            const newWidth = Math.max(5, currentWidth - zoomAmount * 2);

            leftPercent = Math.max(0, newCenter - newWidth / 2);
            rightPercent = Math.min(100, newCenter + newWidth / 2);

            // Adjust if hitting boundaries
            if (leftPercent === 0) rightPercent = newWidth;
            if (rightPercent === 100) leftPercent = 100 - newWidth;
        } else {
            // Zoom out: expand selection
            if (currentWidth >= 100) return;

            const newWidth = Math.min(100, currentWidth + zoomAmount * 2);
            const expand = (newWidth - currentWidth) / 2;

            leftPercent = Math.max(0, leftPercent - expand);
            rightPercent = Math.min(100, rightPercent + expand);

            // Reset if near full range
            if (rightPercent - leftPercent >= 98) {
                leftPercent = 0;
                rightPercent = 100;
            }
        }
    }

    function handleZoomedWheel(e: WheelEvent) {
        e.preventDefault();

        // User is interacting with timeline - stop map sync
        onMapRangeConsumed?.();

        if (selectedDuration === 0) return; // "All" mode, no window to move

        // Scroll to move window position (reduced sensitivity)
        const delta = e.deltaY > 0 ? 1 : -1;
        windowPosition = Math.max(0, Math.min(100, windowPosition + delta));
    }

    // === Helpers ===
    function formatDate(date: Date): string {
        return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
    }

    function formatDateTime(date: Date): string {
        return `${formatDate(date)} ${date.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' })}`;
    }

    function resetSelection() {
        leftPercent = 0;
        rightPercent = 100;
        windowPosition = 0;
        displayTimeRange = null; // Reset to full photo range
        onMapRangeConsumed?.(); // Stop map sync
        onShowAll?.(); // Tell map to show all photos
    }

    // Count photos in view window
    let photosInWindow = $derived.by(() => {
        return photos.filter(p => {
            const date = parsePhotoDate(p.metadata?.date_taken);
            if (!date) return false;
            return date >= viewWindow.start && date <= viewWindow.end;
        }).length;
    });

    // Density bins for overview
    const NUM_BINS = 60;
    let densityBins = $derived.by(() => {
        const totalMs = timeRange.max.getTime() - timeRange.min.getTime();
        if (totalMs === 0) return Array(NUM_BINS).fill(0);

        const bins = Array(NUM_BINS).fill(0);
        for (const p of photos) {
            const date = parsePhotoDate(p.metadata?.date_taken);
            if (!date) continue;
            const pos = (date.getTime() - timeRange.min.getTime()) / totalMs;
            const binIndex = Math.min(NUM_BINS - 1, Math.floor(pos * NUM_BINS));
            if (binIndex >= 0) bins[binIndex]++;
        }
        return bins;
    });

    let maxBinCount = $derived(Math.max(1, ...densityBins));

    // Density bins for zoomed/detail view (selected range or full range)
    let zoomedDensityBins = $derived.by(() => {
        // Use selected range if zoomed, otherwise use full timeRange
        const startMs = isZoomed ? selectedRange.startMs : timeRange.min.getTime();
        const endMs = isZoomed ? selectedRange.endMs : timeRange.max.getTime();
        const durationMs = endMs - startMs;
        if (durationMs === 0) return Array(NUM_BINS).fill(0);

        const bins = Array(NUM_BINS).fill(0);
        for (const p of photos) {
            const date = parsePhotoDate(p.metadata?.date_taken);
            if (!date) continue;
            const dateMs = date.getTime();
            if (dateMs < startMs || dateMs > endMs) continue;
            const pos = (dateMs - startMs) / durationMs;
            const binIndex = Math.min(NUM_BINS - 1, Math.floor(pos * NUM_BINS));
            if (binIndex >= 0) bins[binIndex]++;
        }
        return bins;
    });

    let maxZoomedBinCount = $derived(Math.max(1, ...zoomedDensityBins));

    // Window width percentage in detail view
    let windowWidthPercent = $derived.by(() => {
        if (selectedDuration === 0) return 100;
        // Use selected range if zoomed, otherwise full timeRange
        const durationMs = isZoomed
            ? selectedRange.durationMs
            : timeRange.max.getTime() - timeRange.min.getTime();
        if (durationMs === 0) return 100;
        return Math.min(100, (selectedDuration / durationMs) * 100);
    });

    // Window left position in zoomed view
    let windowLeftPercent = $derived.by(() => {
        const width = windowWidthPercent;
        return (windowPosition / 100) * (100 - width);
    });
</script>

<svelte:window on:mousemove={handleMouseMove} on:mouseup={handleMouseUp} />

<div class="timeline-container theme-bg-overlay backdrop-blur-sm px-4 py-3">
    <!-- Header -->
    <div class="flex items-center justify-between mb-2">
        <div class="text-xs theme-text-secondary">
            {formatDateTime(viewWindow.start)} - {formatDateTime(viewWindow.end)}
        </div>
        <div class="flex items-center gap-3">
            <span class="text-xs theme-text-secondary">
                {photosInWindow} / {photos.length}
            </span>
            {#if isZoomed || isMapConstrained}
                <button onclick={resetSelection} class="text-xs text-[var(--accent)] hover:text-[var(--accent-hover)]">
                    {isMapConstrained ? 'Show All' : 'Reset'}
                </button>
            {/if}
        </div>
    </div>

    <!-- === Detail View (always visible) === -->
    <div class="mb-3">
        <!-- Duration selector - always show all options -->
        <div class="flex items-center gap-2 mb-2">
            <span class="text-[10px] theme-text-muted">Window:</span>
            <div class="flex gap-1">
                {#each durationOptions as option}
                    <button
                        onclick={() => { selectedDuration = option.value; if (option.value === 0) windowPosition = 0; }}
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

        <!-- Detail slider track -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div bind:this={zoomedTrack} class="relative h-10 theme-bg-secondary rounded-lg overflow-hidden" onwheel={handleZoomedWheel}>
            <!-- Density visualization -->
            <div class="absolute inset-0 flex items-end">
                {#each zoomedDensityBins as count}
                    <div
                        class="flex-1"
                        style="height: {Math.max(2, (count / maxZoomedBinCount) * 100)}%; background-color: color-mix(in srgb, var(--accent) 40%, transparent);"
                    ></div>
                {/each}
            </div>

            <!-- Fixed window (draggable) - show when duration selected (not All) -->
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

        <!-- Detail range labels -->
        <div class="flex justify-between text-[10px] theme-text-muted mt-1">
            {#if isZoomed}
                <span>{formatDate(new Date(selectedRange.startMs))}</span>
                <span>{formatDate(new Date(selectedRange.endMs))}</span>
            {:else}
                <span>{formatDate(timeRange.min)}</span>
                <span>{formatDate(timeRange.max)}</span>
            {/if}
        </div>
    </div>

    <!-- === Overview (full timeline with handles) === -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="relative h-16 mx-2" onwheel={handleOverviewWheel}>
        <!-- Track background -->
        <div
            bind:this={sliderTrack}
            class="absolute inset-0 theme-bg-primary rounded overflow-hidden"
        >
            <!-- Density visualization -->
            <div class="absolute inset-0 flex items-end">
                {#each densityBins as count}
                    <div
                        class="flex-1"
                        style="height: {Math.max(2, (count / maxBinCount) * 100)}%; background-color: color-mix(in srgb, var(--accent) 50%, transparent);"
                    ></div>
                {/each}
            </div>

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
        <span>{formatDate(timeRange.min)}</span>
        {#if isMapConstrained}
            <span class="text-[var(--accent)]">Map View</span>
        {/if}
        <span>{formatDate(timeRange.max)}</span>
    </div>
</div>

<style>
    .timeline-container {
        user-select: none;
    }
</style>
