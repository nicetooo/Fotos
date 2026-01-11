<script lang="ts">
    let {
        photos,
        onTimeRangeChange
    } = $props<{
        photos: any[];
        onTimeRangeChange: (start: Date, end: Date) => void;
    }>();

    // Parse dates from photos
    function parsePhotoDate(dateStr: string | null): Date | null {
        if (!dateStr) return null;
        const cleaned = dateStr.replace(/"/g, '').trim();
        const date = new Date(cleaned.replace(' ', 'T'));
        return isNaN(date.getTime()) ? null : date;
    }

    // Get time range from all photos
    let timeRange = $derived.by(() => {
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

    // Actual viewing window (what photos to show)
    let viewWindow = $derived.by(() => {
        if (!isZoomed || selectedDuration === 0) {
            // Show full selected range
            return {
                start: new Date(selectedRange.startMs),
                end: new Date(selectedRange.endMs)
            };
        }

        // Fixed window within selected range
        const windowMs = Math.min(selectedDuration, selectedRange.durationMs);
        const maxOffset = selectedRange.durationMs - windowMs;
        const offset = (windowPosition / 100) * maxOffset;

        return {
            start: new Date(selectedRange.startMs + offset),
            end: new Date(selectedRange.startMs + offset + windowMs)
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

    // Density bins for zoomed view (only selected range)
    let zoomedDensityBins = $derived.by(() => {
        if (!isZoomed) return [];
        const { startMs, endMs, durationMs } = selectedRange;
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

    // Window width percentage in zoomed view
    let windowWidthPercent = $derived.by(() => {
        if (selectedDuration === 0 || selectedRange.durationMs === 0) return 100;
        return Math.min(100, (selectedDuration / selectedRange.durationMs) * 100);
    });

    // Window left position in zoomed view
    let windowLeftPercent = $derived.by(() => {
        const width = windowWidthPercent;
        return (windowPosition / 100) * (100 - width);
    });
</script>

<svelte:window on:mousemove={handleMouseMove} on:mouseup={handleMouseUp} />

<div class="timeline-container bg-black/90 backdrop-blur-sm px-4 py-3">
    <!-- Header -->
    <div class="flex items-center justify-between mb-2">
        <div class="text-xs text-white/60">
            {formatDateTime(viewWindow.start)} - {formatDateTime(viewWindow.end)}
        </div>
        <div class="flex items-center gap-3">
            <span class="text-xs text-white/60">
                {photosInWindow} / {photos.length}
            </span>
            {#if isZoomed}
                <button onclick={resetSelection} class="text-xs text-yellow-400 hover:text-yellow-300">
                    Reset
                </button>
            {/if}
        </div>
    </div>

    {#if isZoomed}
        <!-- === Zoomed View (full width) === -->
        <div class="mb-3">
            <!-- Duration selector -->
            <div class="flex items-center gap-2 mb-2">
                <span class="text-[10px] text-white/40">Window:</span>
                <div class="flex gap-1">
                    {#each durationOptions as option}
                        <button
                            onclick={() => { selectedDuration = option.value; if (option.value === 0) windowPosition = 0; }}
                            class="px-2 py-0.5 text-[10px] rounded transition-colors
                                {selectedDuration === option.value
                                    ? 'bg-yellow-500 text-black'
                                    : 'bg-white/10 text-white/60 hover:bg-white/20'}"
                        >
                            {option.label}
                        </button>
                    {/each}
                </div>
            </div>

            <!-- Zoomed slider track -->
            <div bind:this={zoomedTrack} class="relative h-10 bg-neutral-800 rounded-lg overflow-hidden">
                <!-- Density visualization -->
                <div class="absolute inset-0 flex items-end">
                    {#each zoomedDensityBins as count}
                        <div
                            class="flex-1 bg-yellow-400/30"
                            style="height: {Math.max(2, (count / maxZoomedBinCount) * 100)}%"
                        ></div>
                    {/each}
                </div>

                <!-- Fixed window (draggable) -->
                {#if selectedDuration !== 0}
                    <div
                        class="absolute top-0 bottom-0 bg-yellow-400/20 border-2 border-yellow-400 rounded cursor-grab active:cursor-grabbing"
                        style="left: {windowLeftPercent}%; width: {windowWidthPercent}%"
                        onmousedown={(e) => handleMouseDown(e, 'window')}
                    >
                        <div class="absolute inset-0 flex items-center justify-center">
                            <div class="w-8 h-4 bg-yellow-400 rounded-full flex items-center justify-center">
                                <div class="w-4 h-0.5 bg-yellow-900/50 rounded"></div>
                            </div>
                        </div>
                    </div>
                {/if}
            </div>

            <!-- Zoomed range labels -->
            <div class="flex justify-between text-[10px] text-white/40 mt-1">
                <span>{formatDate(new Date(selectedRange.startMs))}</span>
                <span>{formatDate(new Date(selectedRange.endMs))}</span>
            </div>
        </div>
    {/if}

    <!-- === Overview (full timeline with handles) === -->
    <div class="relative h-8 mx-2">
        <!-- Track background -->
        <div
            bind:this={sliderTrack}
            class="absolute inset-0 bg-neutral-900 rounded overflow-hidden"
        >
            <!-- Density visualization -->
            <div class="absolute inset-0 flex items-end">
                {#each densityBins as count}
                    <div
                        class="flex-1 bg-white/20"
                        style="height: {Math.max(2, (count / maxBinCount) * 100)}%"
                    ></div>
                {/each}
            </div>

            <!-- Dimmed areas -->
            <div class="absolute top-0 bottom-0 left-0 bg-black/70" style="width: {leftPercent}%"></div>
            <div class="absolute top-0 bottom-0 right-0 bg-black/70" style="width: {100 - rightPercent}%"></div>

            <!-- Selected region border -->
            <div
                class="absolute top-0 bottom-0 border-y-2 border-yellow-400 pointer-events-none"
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
            <div class="w-1.5 h-full bg-yellow-400 rounded-sm shadow-lg"></div>
        </div>

        <!-- Right handle -->
        <div
            class="absolute top-0 bottom-0 w-5 cursor-ew-resize flex items-center justify-center z-20"
            style="left: {rightPercent}%; transform: translateX(-50%)"
            onmousedown={(e) => handleMouseDown(e, 'right')}
        >
            <div class="w-1.5 h-full bg-yellow-400 rounded-sm shadow-lg"></div>
        </div>
    </div>

    <!-- Full range labels -->
    <div class="flex justify-between text-[10px] text-white/40 mt-1 mx-2">
        <span>{formatDate(timeRange.min)}</span>
        <span>{formatDate(timeRange.max)}</span>
    </div>
</div>

<style>
    .timeline-container {
        user-select: none;
    }
</style>
