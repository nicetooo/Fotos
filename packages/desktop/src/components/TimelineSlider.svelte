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

    // Selection state (0-100 percentage)
    let leftPercent = $state(0);
    let rightPercent = $state(100);

    // Convert percentage to date
    function percentToDate(percent: number): Date {
        const range = timeRange;
        const totalMs = range.max.getTime() - range.min.getTime();
        return new Date(range.min.getTime() + (percent / 100) * totalMs);
    }

    // Current window range
    let windowRange = $derived.by(() => ({
        start: percentToDate(leftPercent),
        end: percentToDate(rightPercent)
    }));

    // Format date for display
    function formatDate(date: Date): string {
        return date.toLocaleDateString('en-US', {
            month: 'short',
            day: 'numeric',
            year: 'numeric'
        });
    }

    function formatTime(date: Date): string {
        return date.toLocaleTimeString('en-US', {
            hour: '2-digit',
            minute: '2-digit'
        });
    }

    function formatDateTime(date: Date): string {
        const range = timeRange;
        const totalMs = range.max.getTime() - range.min.getTime();
        const dayMs = 24 * 60 * 60 * 1000;

        if (totalMs <= dayMs) {
            return formatTime(date);
        } else if (totalMs <= 7 * dayMs) {
            return `${formatDate(date)} ${formatTime(date)}`;
        }
        return formatDate(date);
    }

    // Notify parent of changes
    $effect(() => {
        const range = windowRange;
        onTimeRangeChange(range.start, range.end);
    });

    // Dragging state
    type DragMode = 'none' | 'left' | 'right' | 'middle';
    let dragMode = $state<DragMode>('none');
    let sliderTrack: HTMLDivElement;
    let dragStartX = 0;
    let dragStartLeft = 0;
    let dragStartRight = 0;

    function handleMouseDown(e: MouseEvent, mode: DragMode) {
        e.preventDefault();
        e.stopPropagation();
        dragMode = mode;
        dragStartX = e.clientX;
        dragStartLeft = leftPercent;
        dragStartRight = rightPercent;
    }

    function handleMouseMove(e: MouseEvent) {
        if (dragMode === 'none' || !sliderTrack) return;

        const rect = sliderTrack.getBoundingClientRect();
        const deltaPercent = ((e.clientX - dragStartX) / rect.width) * 100;

        if (dragMode === 'left') {
            // Drag left handle
            let newLeft = dragStartLeft + deltaPercent;
            newLeft = Math.max(0, Math.min(newLeft, rightPercent - 1)); // Min 1% width
            leftPercent = newLeft;
        } else if (dragMode === 'right') {
            // Drag right handle
            let newRight = dragStartRight + deltaPercent;
            newRight = Math.max(leftPercent + 1, Math.min(newRight, 100)); // Min 1% width
            rightPercent = newRight;
        } else if (dragMode === 'middle') {
            // Drag middle to pan
            const width = dragStartRight - dragStartLeft;
            let newLeft = dragStartLeft + deltaPercent;
            let newRight = dragStartRight + deltaPercent;

            // Clamp to bounds
            if (newLeft < 0) {
                newLeft = 0;
                newRight = width;
            }
            if (newRight > 100) {
                newRight = 100;
                newLeft = 100 - width;
            }

            leftPercent = newLeft;
            rightPercent = newRight;
        }
    }

    function handleMouseUp() {
        dragMode = 'none';
    }

    // Count photos in current window
    let photosInWindow = $derived.by(() => {
        const range = windowRange;
        return photos.filter(p => {
            const date = parsePhotoDate(p.metadata?.date_taken);
            if (!date) return false;
            return date >= range.start && date <= range.end;
        }).length;
    });

    // Pre-compute density bins
    const NUM_BINS = 60;
    let densityBins = $derived.by(() => {
        const range = timeRange;
        const totalMs = range.max.getTime() - range.min.getTime();
        if (totalMs === 0) return Array(NUM_BINS).fill(0);

        const bins = Array(NUM_BINS).fill(0);

        for (const p of photos) {
            const date = parsePhotoDate(p.metadata?.date_taken);
            if (!date) continue;
            const pos = (date.getTime() - range.min.getTime()) / totalMs;
            const binIndex = Math.min(NUM_BINS - 1, Math.floor(pos * NUM_BINS));
            if (binIndex >= 0 && binIndex < NUM_BINS) {
                bins[binIndex]++;
            }
        }

        return bins;
    });

    let maxBinCount = $derived(Math.max(1, ...densityBins));

    // Reset to full range
    function resetSelection() {
        leftPercent = 0;
        rightPercent = 100;
    }
</script>

<svelte:window on:mousemove={handleMouseMove} on:mouseup={handleMouseUp} />

<div class="timeline-container bg-black/90 backdrop-blur-sm px-4 py-3">
    <!-- Header -->
    <div class="flex items-center justify-between mb-2">
        <div class="text-xs text-white/60">
            {formatDateTime(windowRange.start)} - {formatDateTime(windowRange.end)}
        </div>
        <div class="flex items-center gap-3">
            <span class="text-xs text-white/60">
                {photosInWindow} / {photos.length}
            </span>
            {#if leftPercent > 0 || rightPercent < 100}
                <button
                    onclick={resetSelection}
                    class="text-xs text-yellow-400 hover:text-yellow-300"
                >
                    Reset
                </button>
            {/if}
        </div>
    </div>

    <!-- Slider track wrapper (allows handles to overflow) -->
    <div class="relative h-12 mx-2">
        <!-- Track background with overflow hidden -->
        <div
            bind:this={sliderTrack}
            class="absolute inset-0 bg-neutral-900 rounded-lg overflow-hidden"
        >
            <!-- Density visualization (background) -->
            <div class="absolute inset-0 flex items-end">
                {#each densityBins as count, i}
                    <div
                        class="flex-1 bg-white/20"
                        style="height: {Math.max(2, (count / maxBinCount) * 100)}%"
                    ></div>
                {/each}
            </div>

            <!-- Dimmed left area -->
            <div
                class="absolute top-0 bottom-0 left-0 bg-black/70"
                style="width: {leftPercent}%"
            ></div>

            <!-- Dimmed right area -->
            <div
                class="absolute top-0 bottom-0 right-0 bg-black/70"
                style="width: {100 - rightPercent}%"
            ></div>

            <!-- Selected region border -->
            <div
                class="absolute top-0 bottom-0 border-y-2 border-yellow-400 pointer-events-none"
                style="left: {leftPercent}%; right: {100 - rightPercent}%"
            ></div>
        </div>

        <!-- Draggable middle region (outside overflow) -->
        <div
            class="absolute top-0 bottom-0 cursor-grab active:cursor-grabbing z-10"
            style="left: {leftPercent}%; right: {100 - rightPercent}%"
            onmousedown={(e) => handleMouseDown(e, 'middle')}
            role="slider"
            aria-label="Selected time range"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={(leftPercent + rightPercent) / 2}
            tabindex="0"
        ></div>

        <!-- Left handle (outside overflow) -->
        <div
            class="absolute top-0 bottom-0 w-6 cursor-ew-resize flex items-center justify-center z-20"
            style="left: {leftPercent}%; transform: translateX(-50%)"
            onmousedown={(e) => handleMouseDown(e, 'left')}
            role="slider"
            aria-label="Start time"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={leftPercent}
            tabindex="0"
        >
            <div class="w-2 h-full bg-yellow-400 rounded-sm shadow-lg shadow-black/50 flex items-center justify-center">
                <div class="w-0.5 h-5 bg-yellow-900/40 rounded-full"></div>
            </div>
        </div>

        <!-- Right handle (outside overflow) -->
        <div
            class="absolute top-0 bottom-0 w-6 cursor-ew-resize flex items-center justify-center z-20"
            style="left: {rightPercent}%; transform: translateX(-50%)"
            onmousedown={(e) => handleMouseDown(e, 'right')}
            role="slider"
            aria-label="End time"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={rightPercent}
            tabindex="0"
        >
            <div class="w-2 h-full bg-yellow-400 rounded-sm shadow-lg shadow-black/50 flex items-center justify-center">
                <div class="w-0.5 h-5 bg-yellow-900/40 rounded-full"></div>
            </div>
        </div>
    </div>

    <!-- Full range labels -->
    <div class="flex justify-between text-[10px] text-white/40 mt-1">
        <span>{formatDate(timeRange.min)}</span>
        <span>{formatDate(timeRange.max)}</span>
    </div>
</div>

<style>
    .timeline-container {
        user-select: none;
    }
</style>
