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
        // Format: "2024-01-15 10:30:00" or similar
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

    // Duration options
    const durationOptions = [
        { label: '1h', value: 60 * 60 * 1000 },
        { label: '6h', value: 6 * 60 * 60 * 1000 },
        { label: '1d', value: 24 * 60 * 60 * 1000 },
        { label: '7d', value: 7 * 24 * 60 * 60 * 1000 },
        { label: '30d', value: 30 * 24 * 60 * 60 * 1000 },
        { label: 'All', value: 0 },
    ];

    let selectedDuration = $state(durationOptions[2].value); // Default: 1 day
    let sliderPosition = $state(0); // 0-100 percentage

    // Calculate window based on slider position
    let windowRange = $derived.by(() => {
        const range = timeRange;
        const totalMs = range.max.getTime() - range.min.getTime();

        // "All" mode - return full range
        if (selectedDuration === 0) {
            return { start: range.min, end: range.max };
        }

        // Calculate window position
        const windowMs = Math.min(selectedDuration, totalMs);
        const maxOffset = totalMs - windowMs;
        const offset = (sliderPosition / 100) * maxOffset;

        const start = new Date(range.min.getTime() + offset);
        const end = new Date(start.getTime() + windowMs);

        return { start, end };
    });

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
        if (selectedDuration <= 6 * 60 * 60 * 1000) {
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
    let isDragging = $state(false);
    let sliderTrack: HTMLDivElement;
    let dragStartX = 0;
    let dragStartPosition = 0;

    function handleHandleMouseDown(e: MouseEvent) {
        if (selectedDuration === 0) return;
        e.preventDefault();
        e.stopPropagation();
        isDragging = true;
        dragStartX = e.clientX;
        dragStartPosition = sliderPosition;
    }

    function handleTrackClick(e: MouseEvent) {
        if (selectedDuration === 0) return;
        if (!sliderTrack) return;

        const rect = sliderTrack.getBoundingClientRect();
        const clickX = e.clientX - rect.left;
        const clickPercent = (clickX / rect.width) * 100;

        // Move window center to click position
        const widthPct = windowWidthPercent;
        const halfWidth = widthPct / 2;
        const maxLeftPercent = 100 - widthPct;

        // Calculate where the window left edge should be
        const targetLeft = Math.max(0, Math.min(clickPercent - halfWidth, maxLeftPercent));

        // Convert to sliderPosition
        sliderPosition = maxLeftPercent > 0 ? (targetLeft / maxLeftPercent) * 100 : 0;
    }

    function handleMouseMove(e: MouseEvent) {
        if (!isDragging || !sliderTrack) return;

        const rect = sliderTrack.getBoundingClientRect();
        const deltaX = e.clientX - dragStartX;
        const deltaPercent = (deltaX / rect.width) * 100;

        // Convert delta to slider position change
        const widthPct = windowWidthPercent;
        const maxLeftPercent = 100 - widthPct;

        if (maxLeftPercent > 0) {
            const positionDelta = (deltaPercent / maxLeftPercent) * 100;
            sliderPosition = Math.max(0, Math.min(100, dragStartPosition + positionDelta));
        }
    }

    function handleMouseUp() {
        isDragging = false;
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

    // Calculate window width percentage
    let windowWidthPercent = $derived.by(() => {
        if (selectedDuration === 0) return 100;
        const range = timeRange;
        const totalMs = range.max.getTime() - range.min.getTime();
        if (totalMs === 0) return 100;
        return Math.min(100, (selectedDuration / totalMs) * 100);
    });

    // Pre-compute density bins (O(n) instead of O(n²))
    const NUM_BINS = 50;
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

    // Calculate actual left position of window on the timeline
    let windowLeftPercent = $derived.by(() => {
        const widthPct = windowWidthPercent;
        // sliderPosition 0-100 maps to window position 0 to (100-width)
        return (sliderPosition / 100) * (100 - widthPct);
    });
</script>

<svelte:window on:mousemove={handleMouseMove} on:mouseup={handleMouseUp} />

<div class="timeline-container bg-slate-900/95 backdrop-blur-sm border-t border-slate-700 px-4 py-3">
    <!-- Duration selector -->
    <div class="flex items-center justify-between mb-2">
        <div class="flex items-center gap-2">
            <span class="text-xs text-slate-400">Window:</span>
            <div class="flex gap-1">
                {#each durationOptions as option}
                    <button
                        onclick={() => { selectedDuration = option.value; if (option.value === 0) sliderPosition = 0; }}
                        class="px-2 py-0.5 text-xs rounded transition-colors
                            {selectedDuration === option.value
                                ? 'bg-blue-600 text-white'
                                : 'bg-slate-700 text-slate-300 hover:bg-slate-600'}"
                    >
                        {option.label}
                    </button>
                {/each}
            </div>
        </div>
        <div class="text-xs text-slate-400">
            <i class="fa-solid fa-images mr-1"></i>
            {photosInWindow} photos
        </div>
    </div>

    <!-- Time display -->
    <div class="flex justify-between text-xs text-slate-300 mb-1">
        <span>{formatDateTime(windowRange.start)}</span>
        <span class="text-slate-500">to</span>
        <span>{formatDateTime(windowRange.end)}</span>
    </div>

    <!-- Slider track -->
    <div
        bind:this={sliderTrack}
        role="slider"
        aria-label="Time range selector"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={sliderPosition}
        tabindex="0"
        class="relative h-8 bg-slate-800 rounded-lg overflow-hidden cursor-pointer"
        onclick={handleTrackClick}
    >
        <!-- Background ticks for time scale -->
        <div class="absolute inset-0 flex items-end pointer-events-none">
            {#each Array(20) as _, i}
                <div
                    class="flex-1 border-l border-slate-700/50 h-2"
                    style="margin-left: {i === 0 ? 0 : 0}px"
                ></div>
            {/each}
        </div>

        <!-- Photo density visualization (pre-computed) -->
        <div class="absolute inset-0 flex items-end px-0.5 gap-px pointer-events-none">
            {#each densityBins as count, i}
                <div
                    class="flex-1 bg-blue-500/30 rounded-t"
                    style="height: {Math.max(2, (count / maxBinCount) * 100)}%"
                ></div>
            {/each}
        </div>

        <!-- Selection window -->
        {#if selectedDuration !== 0}
            <div
                class="absolute top-0 bottom-0 bg-blue-500/30 border-x-2 border-blue-400 rounded pointer-events-none"
                style="left: {windowLeftPercent}%; width: {windowWidthPercent}%"
            >
                <!-- Draggable Handle -->
                <div
                    class="absolute inset-y-0 left-0 right-0 flex items-center justify-center pointer-events-auto cursor-grab active:cursor-grabbing"
                    onmousedown={handleHandleMouseDown}
                >
                    <div class="w-10 h-5 bg-blue-500 rounded-full flex items-center justify-center shadow-lg hover:bg-blue-400 transition-colors">
                        <i class="fa-solid fa-grip-lines-vertical text-[10px] text-white"></i>
                    </div>
                </div>
            </div>
        {/if}
    </div>

    <!-- Full range labels -->
    <div class="flex justify-between text-[10px] text-slate-500 mt-1">
        <span>{formatDate(timeRange.min)}</span>
        <span>{formatDate(timeRange.max)}</span>
    </div>
</div>

<style>
    .timeline-container {
        user-select: none;
    }
</style>
