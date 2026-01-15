<script lang="ts">
    let {
        photos,
        mapViewTimeRange,
        onTimeRangeChange,
        onMapRangeConsumed,
        onShowAll
    } = $props<{
        photos: any[];
        mapViewTimeRange?: { start: Date; end: Date } | null;
        onTimeRangeChange: (start: Date, end: Date) => void;
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

    // Default to "All"
    let selectedDuration = $state(0);

    let windowPosition = $state(0); // 0-100 percentage within selected range

    // Reset selection and duration when photos change (e.g., after import)
    let prevPhotosLength = $state(0);
    $effect(() => {
        const len = photos.length;
        if (len !== prevPhotosLength && prevPhotosLength > 0) {
            // Reset to full range when photos change
            leftPercent = 0;
            rightPercent = 100;
            windowPosition = 0;
            selectedDuration = 0; // Reset to "All"
            displayTimeRange = null;
        }
        prevPhotosLength = len;
    });

    // Reset duration to "All" when overview selection changes
    let prevLeft = $state(0);
    let prevRight = $state(100);
    $effect(() => {
        const l = leftPercent;
        const r = rightPercent;
        if (l !== prevLeft || r !== prevRight) {
            if (prevLeft !== 0 || prevRight !== 100) {
                // Overview selection changed, reset to All
                selectedDuration = 0;
                windowPosition = 0;
            }
            prevLeft = l;
            prevRight = r;
        }
    });

    // Handle map view time range (continuous sync from map pan/zoom)
    // This changes the TOTAL displayed timeline range, not the selection
    $effect(() => {
        if (!mapViewTimeRange) return;

        // Use exact time range from map, no padding
        displayTimeRange = {
            min: mapViewTimeRange.start,
            max: mapViewTimeRange.end
        };

        // Reset selection to full range when map constrains the view
        leftPercent = 0;
        rightPercent = 100;
    });

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

        // User is interacting with timeline - stop map sync
        onMapRangeConsumed?.();
    }

    function handleTouchStart(e: TouchEvent, mode: DragMode) {
        e.preventDefault();
        e.stopPropagation();
        if (e.touches.length !== 1) return;

        dragMode = mode;
        dragStartX = e.touches[0].clientX;
        dragStartLeft = leftPercent;
        dragStartRight = rightPercent;
        dragStartWindow = windowPosition;

        // User is interacting with timeline - stop map sync
        onMapRangeConsumed?.();
    }

    function handleMouseMove(e: MouseEvent) {
        if (dragMode === 'none') return;
        handleDragMove(e.clientX);
    }

    function handleTouchMove(e: TouchEvent) {
        if (dragMode === 'none') return;
        if (e.touches.length !== 1) return;
        e.preventDefault();
        handleDragMove(e.touches[0].clientX);
    }

    function handleDragMove(clientX: number) {
        if (dragMode === 'window' && zoomedTrack) {
            // Drag fixed window in zoomed view (1:1 mouse tracking)
            const rect = zoomedTrack.getBoundingClientRect();
            const deltaPercent = ((clientX - dragStartX) / rect.width) * 100;
            // Convert to windowPosition space (compensate for window size)
            const maxMovement = 100 - windowWidthPercent;
            if (maxMovement > 0) {
                const deltaWindowPos = (deltaPercent / maxMovement) * 100;
                let newPos = dragStartWindow + deltaWindowPos;
                windowPosition = Math.max(0, Math.min(100, newPos));
            }
        } else if (sliderTrack) {
            // Drag handles in overview - use delta from start position
            const rect = sliderTrack.getBoundingClientRect();
            const deltaPercent = ((clientX - dragStartX) / rect.width) * 100;

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

    function handleTouchEnd() {
        dragMode = 'none';
    }

    // Smart touch handler for overview track - determines which handle to drag based on touch position
    function handleTrackTouch(e: TouchEvent) {
        if (e.touches.length !== 1) return;
        e.preventDefault();
        e.stopPropagation();

        const touch = e.touches[0];
        const rect = sliderTrack.getBoundingClientRect();
        const touchPercent = ((touch.clientX - rect.left) / rect.width) * 100;

        // Calculate distances to handles
        const distToLeft = Math.abs(touchPercent - leftPercent);
        const distToRight = Math.abs(touchPercent - rightPercent);

        // Determine which handle is closer, with bias towards the nearest one
        let mode: DragMode;
        if (distToLeft < distToRight) {
            mode = 'left';
        } else if (distToRight < distToLeft) {
            mode = 'right';
        } else {
            // Equal distance - prefer moving the selection
            mode = 'middle';
        }

        // If touch is clearly inside the selection (not near handles), drag the whole selection
        const handleThreshold = 15; // percentage threshold for handle detection
        if (touchPercent > leftPercent + handleThreshold && touchPercent < rightPercent - handleThreshold) {
            mode = 'middle';
        }

        dragMode = mode;
        dragStartX = touch.clientX;
        dragStartLeft = leftPercent;
        dragStartRight = rightPercent;
        dragStartWindow = windowPosition;

        onMapRangeConsumed?.();
    }

    // Tap on track to jump selection or drag middle (for touch devices)
    function handleTrackTap(e: TouchEvent) {
        if (e.touches.length !== 1) return;

        const touch = e.touches[0];
        const rect = sliderTrack.getBoundingClientRect();
        const touchPercent = ((touch.clientX - rect.left) / rect.width) * 100;

        // If tap is outside current selection, jump to that position (priority over handle detection)
        if (touchPercent < leftPercent - 5 || touchPercent > rightPercent + 5) {
            e.preventDefault();
            e.stopPropagation();

            const width = rightPercent - leftPercent;
            const halfWidth = width / 2;

            let newLeft = touchPercent - halfWidth;
            let newRight = touchPercent + halfWidth;

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
            onMapRangeConsumed?.();
            return;
        }

        // Calculate handle hot zone in percentage (25px / track width * 100)
        const outsideOffset = (20 / rect.width) * 100;
        const insideOffset = (5 / rect.width) * 100;

        // Check if touch is within handle hot zones
        const nearLeftHandle = touchPercent >= leftPercent - outsideOffset && touchPercent <= leftPercent + insideOffset;
        const nearRightHandle = touchPercent >= rightPercent - insideOffset && touchPercent <= rightPercent + outsideOffset;

        // If near a handle, let the handle's own event handler deal with it
        if (nearLeftHandle || nearRightHandle) {
            return;
        }

        e.preventDefault();
        e.stopPropagation();

        // Touch is inside selection but not near handles - drag the whole selection
        dragMode = 'middle';
        dragStartX = touch.clientX;
        dragStartLeft = leftPercent;
        dragStartRight = rightPercent;
        dragStartWindow = windowPosition;
        onMapRangeConsumed?.();
    }

    // Click on track to jump selection (for mouse)
    function handleTrackClick(e: MouseEvent) {
        const rect = sliderTrack.getBoundingClientRect();
        const clickPercent = ((e.clientX - rect.left) / rect.width) * 100;

        // If click is outside current selection, move selection to that position
        if (clickPercent < leftPercent || clickPercent > rightPercent) {
            const width = rightPercent - leftPercent;
            const halfWidth = width / 2;

            let newLeft = clickPercent - halfWidth;
            let newRight = clickPercent + halfWidth;

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
            onMapRangeConsumed?.();
        }
    }

    // Click/tap on zoomed track to jump window position
    function handleZoomedTrackClick(e: MouseEvent) {
        const rect = zoomedTrack.getBoundingClientRect();
        const clickPercent = ((e.clientX - rect.left) / rect.width) * 100;

        // When selectedDuration === 0 (All), there's no window to move
        if (selectedDuration === 0) {
            return;
        }

        // If click is outside current window, move window to that position
        if (clickPercent < windowLeftPercent || clickPercent > windowLeftPercent + windowWidthPercent) {
            const maxLeft = 100 - windowWidthPercent;
            if (maxLeft <= 0) return;

            let newLeft = clickPercent - windowWidthPercent / 2;
            newLeft = Math.max(0, Math.min(maxLeft, newLeft));
            // windowPosition is 0-100, convert from 0-maxLeft range
            windowPosition = (newLeft / maxLeft) * 100;
            onMapRangeConsumed?.();
        }
    }

    function handleZoomedTrackTap(e: TouchEvent) {
        if (e.touches.length !== 1) return;

        const touch = e.touches[0];
        const rect = zoomedTrack.getBoundingClientRect();
        const touchPercent = ((touch.clientX - rect.left) / rect.width) * 100;

        // When selectedDuration === 0 (All), there's no window to move
        if (selectedDuration === 0) {
            return;
        }

        // Check if touch is on the window (let window's own handler deal with it)
        if (touchPercent >= windowLeftPercent && touchPercent <= windowLeftPercent + windowWidthPercent) {
            return;
        }

        e.preventDefault();
        e.stopPropagation();

        // Calculate new window position to center the window on tap
        const maxLeft = 100 - windowWidthPercent;
        if (maxLeft <= 0) return;

        let newLeft = touchPercent - windowWidthPercent / 2;
        newLeft = Math.max(0, Math.min(maxLeft, newLeft));
        // windowPosition is 0-100, convert from 0-maxLeft range
        windowPosition = (newLeft / maxLeft) * 100;
        onMapRangeConsumed?.();
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

        // Scroll to move window position (low sensitivity)
        const delta = e.deltaY > 0 ? 0.3 : -0.3;
        windowPosition = Math.max(0, Math.min(100, windowPosition + delta));
    }

    // === Helpers ===
    function formatDate(date: Date): string {
        return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
    }

    function formatDateShort(date: Date): string {
        return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: '2-digit' });
    }

    function formatDateTime(date: Date): string {
        return `${formatDate(date)} ${date.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' })}`;
    }

    function formatDateTimeShort(date: Date): string {
        return `${formatDateShort(date)} ${date.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })}`;
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

    // Photo positions for overview (percentage 0-100)
    let photoPositions = $derived.by(() => {
        const totalMs = timeRange.max.getTime() - timeRange.min.getTime();
        if (totalMs === 0) return [];

        const positions: number[] = [];
        for (const p of photos) {
            const date = parsePhotoDate(p.metadata?.date_taken);
            if (!date) continue;
            const pos = ((date.getTime() - timeRange.min.getTime()) / totalMs) * 100;
            positions.push(pos);
        }
        return positions;
    });

    // Photo positions for zoomed view (only selected range)
    let zoomedPhotoPositions = $derived.by(() => {
        if (!isZoomed) return [];
        const { startMs, endMs, durationMs } = selectedRange;
        if (durationMs === 0) return [];

        const positions: number[] = [];
        for (const p of photos) {
            const date = parsePhotoDate(p.metadata?.date_taken);
            if (!date) continue;
            const dateMs = date.getTime();
            if (dateMs < startMs || dateMs > endMs) continue;
            const pos = ((dateMs - startMs) / durationMs) * 100;
            positions.push(pos);
        }
        return positions;
    });

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

    // Canvas refs
    let overviewCanvas: HTMLCanvasElement;
    let zoomedCanvas: HTMLCanvasElement;

    // Get accent color from CSS variable
    function getAccentColor(): string {
        if (typeof document === 'undefined') return '#fbbf24';
        return getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#fbbf24';
    }

    // Draw photo lines on canvas
    function drawPhotoLines(canvas: HTMLCanvasElement | undefined, positions: number[]) {
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        if (!ctx) return;

        const rect = canvas.getBoundingClientRect();
        if (rect.width === 0 || rect.height === 0) return;

        const dpr = window.devicePixelRatio || 1;
        canvas.width = rect.width * dpr;
        canvas.height = rect.height * dpr;
        ctx.scale(dpr, dpr);

        ctx.clearRect(0, 0, rect.width, rect.height);
        ctx.strokeStyle = getAccentColor();
        ctx.globalAlpha = 0.7;
        ctx.lineWidth = 1;

        ctx.beginPath();
        for (const pos of positions) {
            const x = Math.round((pos / 100) * rect.width) + 0.5;
            ctx.moveTo(x, 0);
            ctx.lineTo(x, rect.height);
        }
        ctx.stroke();
    }

    // Resize observer to redraw canvases when container size changes
    let overviewResizeObserver: ResizeObserver | null = null;
    let zoomedResizeObserver: ResizeObserver | null = null;

    // Redraw overview canvas when data or size changes
    $effect(() => {
        const positions = photoPositions;
        const canvas = overviewCanvas;
        if (!canvas) return;

        // Initial draw
        drawPhotoLines(canvas, positions);

        // Setup resize observer
        overviewResizeObserver?.disconnect();
        overviewResizeObserver = new ResizeObserver(() => {
            drawPhotoLines(canvas, photoPositions);
        });
        overviewResizeObserver.observe(canvas);

        return () => {
            overviewResizeObserver?.disconnect();
        };
    });

    // Redraw zoomed canvas when data or size changes
    $effect(() => {
        const positions = zoomedPhotoPositions;
        const canvas = zoomedCanvas;
        if (!canvas) return;

        // Initial draw
        drawPhotoLines(canvas, positions);

        // Setup resize observer
        zoomedResizeObserver?.disconnect();
        zoomedResizeObserver = new ResizeObserver(() => {
            drawPhotoLines(canvas, zoomedPhotoPositions);
        });
        zoomedResizeObserver.observe(canvas);

        return () => {
            zoomedResizeObserver?.disconnect();
        };
    });
</script>

<svelte:window on:mousemove={handleMouseMove} on:mouseup={handleMouseUp} on:touchmove={handleTouchMove} on:touchend={handleTouchEnd} />

<div class="timeline-container theme-bg-overlay backdrop-blur-sm px-4 py-3">
    <!-- Header -->
    <div class="flex items-center justify-between mb-2">
        <div class="text-[10px] theme-text-secondary">
            {formatDateTimeShort(viewWindow.start)} - {formatDateTimeShort(viewWindow.end)}
        </div>
        <div class="flex items-center gap-2">
            <span class="text-[10px] theme-text-secondary">
                {photosInWindow}/{photos.length}
            </span>
        </div>
    </div>

    {#if isZoomed}
        <!-- === Zoomed View (full width) === -->
        <div class="mb-3">
            <!-- Duration selector -->
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

            <!-- Zoomed slider track -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div bind:this={zoomedTrack} class="relative h-10 theme-bg-secondary rounded-lg overflow-hidden" onwheel={handleZoomedWheel} onclick={handleZoomedTrackClick} ontouchstart={handleZoomedTrackTap}>
                <!-- Photo lines (canvas) -->
                <canvas bind:this={zoomedCanvas} class="absolute inset-0 w-full h-full"></canvas>

                <!-- Fixed window (draggable) -->
                {#if selectedDuration !== 0}
                    <div
                        class="absolute top-0 bottom-0 bg-[var(--accent)]/20 border-2 border-[var(--accent)] rounded-lg cursor-grab active:cursor-grabbing"
                        style="left: {windowLeftPercent}%; width: {windowWidthPercent}%"
                        onmousedown={(e) => handleMouseDown(e, 'window')}
                        ontouchstart={(e) => { e.stopPropagation(); handleTouchStart(e, 'window'); }}
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
                <span>{formatDate(new Date(selectedRange.startMs))}</span>
                <span>{formatDate(new Date(selectedRange.endMs))}</span>
            </div>
        </div>
    {/if}

    <!-- === Overview (full timeline with handles) === -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="relative h-16 mx-2" onwheel={handleOverviewWheel}>
        <!-- Track background with smart touch handling -->
        <div
            bind:this={sliderTrack}
            class="absolute inset-0 theme-bg-primary rounded-lg overflow-hidden"
            onclick={handleTrackClick}
            ontouchstart={handleTrackTap}
        >
            <!-- Photo lines (canvas) -->
            <canvas bind:this={overviewCanvas} class="absolute inset-0 w-full h-full"></canvas>

            <!-- Dimmed areas -->
            <div class="absolute top-0 bottom-0 left-0 bg-black/50 dark:bg-black/70" style="width: {leftPercent}%"></div>
            <div class="absolute top-0 bottom-0 right-0 bg-black/50 dark:bg-black/70" style="width: {100 - rightPercent}%"></div>

            <!-- Selected region with thick side borders as handles -->
            <div
                class="absolute top-0 bottom-0 border-y-2 border-[var(--accent)] rounded pointer-events-none"
                style="left: {leftPercent}%; right: {100 - rightPercent}%"
            ></div>
        </div>

        <!-- Left handle (thick border, direct drag) -->
        <div
            class="absolute top-0 bottom-0 w-2 bg-[var(--accent)] rounded-l cursor-ew-resize z-20 {dragMode === 'left' ? 'bg-white shadow-[0_0_6px_var(--accent)]' : ''}"
            style="left: {leftPercent}%; transform: translateX(-50%)"
            onmousedown={(e) => handleMouseDown(e, 'left')}
            ontouchstart={(e) => { e.stopPropagation(); handleTouchStart(e, 'left'); }}
        ></div>

        <!-- Right handle (thick border, direct drag) -->
        <div
            class="absolute top-0 bottom-0 w-2 bg-[var(--accent)] rounded-r cursor-ew-resize z-20 {dragMode === 'right' ? 'bg-white shadow-[0_0_6px_var(--accent)]' : ''}"
            style="left: {rightPercent}%; transform: translateX(-50%)"
            onmousedown={(e) => handleMouseDown(e, 'right')}
            ontouchstart={(e) => { e.stopPropagation(); handleTouchStart(e, 'right'); }}
        ></div>
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
