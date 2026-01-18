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

    // === Constants ===
    const HOUR = 60 * 60 * 1000;
    const DAY = 24 * HOUR;
    const MIN_DURATION = 5 * 60 * 1000; // 5 minutes minimum

    // Duration presets (0 = All)
    const durationPresets = [
        { label: '1h', value: HOUR },
        { label: '6h', value: 6 * HOUR },
        { label: '1d', value: DAY },
        { label: '7d', value: 7 * DAY },
        { label: '30d', value: 30 * DAY },
        { label: 'All', value: 0 },
    ];

    // === Parse dates from photos ===
    function parsePhotoDate(dateStr: string | null): Date | null {
        if (!dateStr) return null;
        const cleaned = dateStr.replace(/"/g, '').trim();
        const date = new Date(cleaned.replace(' ', 'T'));
        return isNaN(date.getTime()) ? null : date;
    }

    // === Full time range from all photos ===
    let fullTimeRange = $derived.by(() => {
        const dates = photos
            .map(p => parsePhotoDate(p.metadata?.date_taken))
            .filter((d): d is Date => d !== null)
            .sort((a, b) => a.getTime() - b.getTime());

        if (dates.length === 0) {
            const now = new Date();
            return { min: now, max: now, durationMs: 0 };
        }
        const min = dates[0];
        const max = dates[dates.length - 1];
        return { min, max, durationMs: max.getTime() - min.getTime() };
    });

    // Display time range (can be constrained by map view)
    let displayTimeRange = $state<{ min: Date; max: Date } | null>(null);

    // Actual time range used for display (map range or full range)
    let timeRange = $derived(displayTimeRange ?? fullTimeRange);
    let timeRangeDurationMs = $derived(timeRange.max.getTime() - timeRange.min.getTime());

    // Is showing constrained range from map?
    let isMapConstrained = $derived(displayTimeRange !== null);

    // === Core state: view duration and center ===
    // viewDurationMs: how much time the detail view shows
    // viewCenterMs: the center time point (in absolute milliseconds)
    let viewDurationMs = $state(DAY); // Default to 1 day
    let viewCenterMs = $state(0); // Will be initialized from photos

    // Track if user has customized duration via pinch zoom
    let isCustomDuration = $state(false);

    // Initialize viewCenterMs when photos change
    let prevPhotosLength = $state(0);
    $effect(() => {
        const len = photos.length;
        if (len !== prevPhotosLength) {
            if (len > 0 && fullTimeRange.durationMs > 0) {
                // Center on the middle of the time range
                viewCenterMs = fullTimeRange.min.getTime() + fullTimeRange.durationMs / 2;
                // Default to show ALL photos initially so user sees everything on map
                // User can then narrow down using presets or pinch zoom
                viewDurationMs = fullTimeRange.durationMs;
                isCustomDuration = false;
            }
            prevPhotosLength = len;
        }
    });

    // Handle map view time range
    $effect(() => {
        if (!mapViewTimeRange) return;
        displayTimeRange = {
            min: mapViewTimeRange.start,
            max: mapViewTimeRange.end
        };
    });

    // === Derived: view window (what photos to show) ===
    let viewWindow = $derived.by(() => {
        const halfDuration = viewDurationMs / 2;
        let start = viewCenterMs - halfDuration;
        let end = viewCenterMs + halfDuration;

        // Clamp to time range
        const minMs = timeRange.min.getTime();
        const maxMs = timeRange.max.getTime();

        if (start < minMs) {
            start = minMs;
            end = Math.min(maxMs, start + viewDurationMs);
        }
        if (end > maxMs) {
            end = maxMs;
            start = Math.max(minMs, end - viewDurationMs);
        }

        return {
            start: new Date(start),
            end: new Date(end),
            startMs: start,
            endMs: end
        };
    });

    // === Derived: position on overview (percentage) ===
    let overviewLeftPercent = $derived.by(() => {
        if (timeRangeDurationMs === 0) return 0;
        return ((viewWindow.startMs - timeRange.min.getTime()) / timeRangeDurationMs) * 100;
    });

    let overviewRightPercent = $derived.by(() => {
        if (timeRangeDurationMs === 0) return 100;
        return ((viewWindow.endMs - timeRange.min.getTime()) / timeRangeDurationMs) * 100;
    });

    let overviewWidthPercent = $derived(overviewRightPercent - overviewLeftPercent);

    // Minimum visual width for the handle border (just the two border lines visible)
    const MIN_HANDLE_WIDTH_PX = 4; // 2px border on each side

    // Notify parent
    $effect(() => {
        onTimeRangeChange(viewWindow.start, viewWindow.end);
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

    // === Detail timeline: single finger pan ===
    function handleDetailTouchStart(e: TouchEvent) {
        if (e.touches.length === 1) {
            // Single finger - pan
            e.preventDefault();
            dragMode = 'detail-pan';
            dragStartX = e.touches[0].clientX;
            dragStartCenterMs = viewCenterMs;
            onMapRangeConsumed?.();
        } else if (e.touches.length === 2) {
            // Two fingers - start pinch
            e.preventDefault();
            isPinching = true;
            dragMode = 'none';
            pinchStartDistance = getPinchDistance(e.touches);
            pinchStartDuration = viewDurationMs;
            onMapRangeConsumed?.();
        }
    }

    function handleDetailTouchMove(e: TouchEvent) {
        if (isPinching && e.touches.length === 2) {
            e.preventDefault();
            const currentDistance = getPinchDistance(e.touches);
            const scale = pinchStartDistance / currentDistance;

            // Calculate new duration with smart stepping
            let newDuration = pinchStartDuration * scale;
            newDuration = applySmartStep(newDuration, pinchStartDuration, scale > 1);

            // Clamp duration
            const maxDuration = timeRangeDurationMs || fullTimeRange.durationMs;
            newDuration = Math.max(MIN_DURATION, Math.min(maxDuration, newDuration));

            viewDurationMs = newDuration;
            isCustomDuration = true;

            // Clamp center to keep view within bounds
            clampViewCenter();
        } else if (dragMode === 'detail-pan' && e.touches.length === 1) {
            e.preventDefault();
            const deltaX = e.touches[0].clientX - dragStartX;
            const rect = detailTrack?.getBoundingClientRect();
            if (!rect) return;

            // Convert pixel delta to time delta
            // Moving finger right = earlier time (scroll left)
            const pxPerMs = rect.width / viewDurationMs;
            const deltaMs = -deltaX / pxPerMs;

            let newCenter = dragStartCenterMs + deltaMs;

            // Clamp to bounds
            const minMs = timeRange.min.getTime();
            const maxMs = timeRange.max.getTime();
            const halfDuration = viewDurationMs / 2;
            newCenter = Math.max(minMs + halfDuration, Math.min(maxMs - halfDuration, newCenter));

            viewCenterMs = newCenter;
        }
    }

    function handleDetailTouchEnd(e: TouchEvent) {
        if (e.touches.length === 0) {
            dragMode = 'none';
            isPinching = false;
        } else if (e.touches.length === 1 && isPinching) {
            // Transitioned from pinch to single touch
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

        // Calculate hot zone (50px each side, converted to percentage)
        const hotZonePx = 50;
        const hotZonePercent = (hotZonePx / rect.width) * 100;

        // Check if touch is within expanded hot zone of the yellow box
        const boxLeftWithHotZone = overviewLeftPercent - hotZonePercent;
        const boxRightWithHotZone = overviewRightPercent + hotZonePercent;

        if (touchPercent >= boxLeftWithHotZone && touchPercent <= boxRightWithHotZone) {
            // Drag the box (expanded hot zone)
            dragMode = 'overview-box';
            dragStartX = touch.clientX;
            dragStartCenterMs = viewCenterMs;
        } else {
            // Tap outside - jump to that position
            const newCenterPercent = touchPercent;
            const newCenterMs = timeRange.min.getTime() + (newCenterPercent / 100) * timeRangeDurationMs;

            // Clamp to keep view within bounds
            const halfDuration = viewDurationMs / 2;
            const minMs = timeRange.min.getTime();
            const maxMs = timeRange.max.getTime();
            viewCenterMs = Math.max(minMs + halfDuration, Math.min(maxMs - halfDuration, newCenterMs));
        }

        onMapRangeConsumed?.();
    }

    function handleOverviewTouchMove(e: TouchEvent) {
        if (dragMode !== 'overview-box') return;
        if (e.touches.length !== 1) return;
        e.preventDefault();

        const deltaX = e.touches[0].clientX - dragStartX;
        const rect = overviewTrack.getBoundingClientRect();

        // Convert pixel delta to time delta
        const deltaPercent = (deltaX / rect.width) * 100;
        const deltaMs = (deltaPercent / 100) * timeRangeDurationMs;

        let newCenter = dragStartCenterMs + deltaMs;

        // Clamp to bounds
        const minMs = timeRange.min.getTime();
        const maxMs = timeRange.max.getTime();
        const halfDuration = viewDurationMs / 2;
        newCenter = Math.max(minMs + halfDuration, Math.min(maxMs - halfDuration, newCenter));

        viewCenterMs = newCenter;
    }

    function handleOverviewTouchEnd() {
        dragMode = 'none';
    }

    // Mouse support for overview (click to jump)
    function handleOverviewClick(e: MouseEvent) {
        const rect = overviewTrack.getBoundingClientRect();
        const clickPercent = ((e.clientX - rect.left) / rect.width) * 100;

        // If click is outside current box, jump to that position
        if (clickPercent < overviewLeftPercent || clickPercent > overviewRightPercent) {
            const newCenterMs = timeRange.min.getTime() + (clickPercent / 100) * timeRangeDurationMs;

            const halfDuration = viewDurationMs / 2;
            const minMs = timeRange.min.getTime();
            const maxMs = timeRange.max.getTime();
            viewCenterMs = Math.max(minMs + halfDuration, Math.min(maxMs - halfDuration, newCenterMs));

            onMapRangeConsumed?.();
        }
    }

    // === Duration preset buttons ===
    function selectDurationPreset(durationMs: number) {
        const maxDuration = timeRangeDurationMs || fullTimeRange.durationMs;
        if (durationMs === 0) {
            // "All" option - show full range
            viewDurationMs = maxDuration;
            viewCenterMs = timeRange.min.getTime() + maxDuration / 2;
        } else {
            viewDurationMs = Math.min(durationMs, maxDuration);
            clampViewCenter();
        }
        isCustomDuration = false;
        onMapRangeConsumed?.();
    }

    // Check if a preset is currently selected
    function isPresetSelected(presetValue: number): boolean {
        if (isCustomDuration) return false;
        const maxDuration = timeRangeDurationMs || fullTimeRange.durationMs;
        if (presetValue === 0) {
            // "All" is selected if viewDuration equals full range
            return Math.abs(viewDurationMs - maxDuration) < 1000;
        }
        // Allow small tolerance for floating point
        return Math.abs(viewDurationMs - presetValue) < 1000;
    }

    // === Helper functions ===
    function getPinchDistance(touches: TouchList): number {
        const dx = touches[0].clientX - touches[1].clientX;
        const dy = touches[0].clientY - touches[1].clientY;
        return Math.sqrt(dx * dx + dy * dy);
    }

    function applySmartStep(newDuration: number, baseDuration: number, isZoomingOut: boolean): number {
        // Determine the time scale and apply appropriate stepping
        if (newDuration >= 7 * DAY) {
            // Week+ scale: step by days
            return Math.round(newDuration / DAY) * DAY;
        } else if (newDuration >= DAY) {
            // Day scale: step by 6 hours
            return Math.round(newDuration / (6 * HOUR)) * (6 * HOUR);
        } else if (newDuration >= 6 * HOUR) {
            // Multi-hour scale: step by hours
            return Math.round(newDuration / HOUR) * HOUR;
        } else if (newDuration >= HOUR) {
            // Hour scale: step by 15 minutes
            return Math.round(newDuration / (15 * 60 * 1000)) * (15 * 60 * 1000);
        } else {
            // Sub-hour scale: step by 5 minutes
            return Math.round(newDuration / (5 * 60 * 1000)) * (5 * 60 * 1000);
        }
    }

    function clampViewCenter() {
        const minMs = timeRange.min.getTime();
        const maxMs = timeRange.max.getTime();
        const halfDuration = viewDurationMs / 2;
        viewCenterMs = Math.max(minMs + halfDuration, Math.min(maxMs - halfDuration, viewCenterMs));
    }

    function formatDuration(ms: number): string {
        if (ms >= DAY) {
            const days = Math.round(ms / DAY);
            return `${days}d`;
        } else if (ms >= HOUR) {
            const hours = Math.round(ms / HOUR);
            return `${hours}h`;
        } else {
            const mins = Math.round(ms / (60 * 1000));
            return `${mins}m`;
        }
    }

    function formatDate(date: Date): string {
        return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
    }

    function formatDateTimeShort(date: Date): string {
        return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' }) + ' ' +
               date.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
    }

    function resetSelection() {
        viewDurationMs = Math.min(DAY, fullTimeRange.durationMs);
        viewCenterMs = fullTimeRange.min.getTime() + fullTimeRange.durationMs / 2;
        isCustomDuration = false;
        displayTimeRange = null;
        onMapRangeConsumed?.();
        onShowAll?.();
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
        if (timeRangeDurationMs === 0) return [];

        const positions: number[] = [];
        const minMs = timeRange.min.getTime();
        for (const p of photos) {
            const date = parsePhotoDate(p.metadata?.date_taken);
            if (!date) continue;
            const pos = ((date.getTime() - minMs) / timeRangeDurationMs) * 100;
            positions.push(pos);
        }
        return positions;
    });

    // Photo positions for detail view (percentage 0-100 within view window)
    // Also collect data for edge stacking
    let detailPhotoData = $derived.by(() => {
        if (viewDurationMs === 0) return {
            positions: [],
            leftCount: 0,
            rightCount: 0,
            leftLineCount: 0,
            rightLineCount: 0
        };

        const positions: number[] = [];
        const leftTimestamps: number[] = [];
        const rightTimestamps: number[] = [];

        for (const p of photos) {
            const date = parsePhotoDate(p.metadata?.date_taken);
            if (!date) continue;
            const dateMs = date.getTime();
            const pos = ((dateMs - viewWindow.startMs) / viewDurationMs) * 100;

            if (dateMs < viewWindow.startMs) {
                leftTimestamps.push(dateMs);
            } else if (dateMs > viewWindow.endMs) {
                rightTimestamps.push(dateMs);
            } else {
                positions.push(pos);
            }
        }

        // Calculate unique line count based on time intervals
        // Two photos overlap if their time difference is less than what 1 pixel represents
        // Assume canvas width ~350px, so 1px = viewDurationMs / 350
        const msPerPixel = viewDurationMs / 350;

        function countUniqueLines(timestamps: number[]): number {
            if (timestamps.length === 0) return 0;
            const sorted = [...timestamps].sort((a, b) => a - b);
            let count = 1;
            let lastTs = sorted[0];
            for (let i = 1; i < sorted.length; i++) {
                if (sorted[i] - lastTs >= msPerPixel) {
                    count++;
                    lastTs = sorted[i];
                }
            }
            return count;
        }

        return {
            positions,
            leftCount: leftTimestamps.length,
            rightCount: rightTimestamps.length,
            leftLineCount: countUniqueLines(leftTimestamps),
            rightLineCount: countUniqueLines(rightTimestamps)
        };
    });

    let detailPhotoPositions = $derived(detailPhotoData.positions);
    let leftCount = $derived(detailPhotoData.leftCount);
    let rightCount = $derived(detailPhotoData.rightCount);
    let leftLineCount = $derived(detailPhotoData.leftLineCount);
    let rightLineCount = $derived(detailPhotoData.rightLineCount);

    // === Canvas drawing ===
    let overviewCanvas: HTMLCanvasElement;
    let detailCanvas: HTMLCanvasElement;

    function getAccentColor(): string {
        if (typeof document === 'undefined') return '#fbbf24';
        return getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#fbbf24';
    }

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

    // Draw detail canvas with edge indicators for photos outside view
    function drawDetailPhotoLines(
        canvas: HTMLCanvasElement | undefined,
        positions: number[],
        leftTotal: number,
        rightTotal: number,
        leftLines: number,
        rightLines: number
    ) {
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
        const accentColor = getAccentColor();

        // Draw main photo lines
        ctx.strokeStyle = accentColor;
        ctx.globalAlpha = 0.7;
        ctx.lineWidth = 1;

        ctx.beginPath();
        for (const pos of positions) {
            const x = Math.round((pos / 100) * rect.width) + 0.5;
            ctx.moveTo(x, 0);
            ctx.lineTo(x, rect.height);
        }
        ctx.stroke();

        // Draw edge indicators for photos outside view
        // Line count is pre-calculated based on time intervals (stable)
        // Limit max stack width to 20% of screen width
        const maxStackWidth = Math.floor(rect.width * 0.2);

        // Left edge indicator
        if (leftLines > 0) {
            const lineCount = Math.min(leftLines, maxStackWidth);

            ctx.strokeStyle = accentColor;
            ctx.globalAlpha = 0.7;
            ctx.lineWidth = 1;
            ctx.beginPath();

            // Draw tightly from left edge: x = 0, 1, 2, ...
            for (let i = 0; i < lineCount; i++) {
                const x = i + 0.5;
                ctx.moveTo(x, 0);
                ctx.lineTo(x, rect.height);
            }
            ctx.stroke();

            // Draw count label (shows total photos)
            const label = `${leftTotal}`;
            ctx.font = 'bold 10px sans-serif';
            const textWidth = ctx.measureText(label).width;
            const labelX = lineCount + 4;

            ctx.globalAlpha = 0.8;
            ctx.fillStyle = 'black';
            ctx.fillRect(labelX - 2, rect.height - 14, textWidth + 4, 12);

            ctx.globalAlpha = 1;
            ctx.fillStyle = accentColor;
            ctx.textAlign = 'left';
            ctx.fillText(label, labelX, rect.height - 4);
        }

        // Right edge indicator
        if (rightLines > 0) {
            const lineCount = Math.min(rightLines, maxStackWidth);

            ctx.strokeStyle = accentColor;
            ctx.globalAlpha = 0.7;
            ctx.lineWidth = 1;
            ctx.beginPath();

            // Draw tightly from right edge: x = width-1, width-2, ...
            for (let i = 0; i < lineCount; i++) {
                const x = rect.width - 1 - i + 0.5;
                ctx.moveTo(x, 0);
                ctx.lineTo(x, rect.height);
            }
            ctx.stroke();

            // Draw count label (shows total photos)
            const label = `${rightTotal}`;
            ctx.font = 'bold 10px sans-serif';
            const textWidth = ctx.measureText(label).width;
            const labelX = rect.width - lineCount - textWidth - 4;

            ctx.globalAlpha = 0.8;
            ctx.fillStyle = 'black';
            ctx.fillRect(labelX - 2, rect.height - 14, textWidth + 4, 12);

            ctx.globalAlpha = 1;
            ctx.fillStyle = accentColor;
            ctx.textAlign = 'left';
            ctx.fillText(label, labelX, rect.height - 4);
        }
    }

    // Resize observers and animation frame IDs for debouncing
    let overviewResizeObserver: ResizeObserver | null = null;
    let detailResizeObserver: ResizeObserver | null = null;
    let overviewRafId: number | null = null;
    let detailRafId: number | null = null;

    // Debounced draw functions to prevent flickering
    function scheduleOverviewDraw(canvas: HTMLCanvasElement, positions: number[]) {
        if (overviewRafId !== null) {
            cancelAnimationFrame(overviewRafId);
        }
        overviewRafId = requestAnimationFrame(() => {
            drawPhotoLines(canvas, positions);
            overviewRafId = null;
        });
    }

    function scheduleDetailDraw(
        canvas: HTMLCanvasElement,
        positions: number[],
        leftTotal: number,
        rightTotal: number,
        leftLines: number,
        rightLines: number
    ) {
        if (detailRafId !== null) {
            cancelAnimationFrame(detailRafId);
        }
        detailRafId = requestAnimationFrame(() => {
            drawDetailPhotoLines(canvas, positions, leftTotal, rightTotal, leftLines, rightLines);
            detailRafId = null;
        });
    }

    $effect(() => {
        const positions = photoPositions;
        const canvas = overviewCanvas;
        if (!canvas) return;

        scheduleOverviewDraw(canvas, positions);

        overviewResizeObserver?.disconnect();
        overviewResizeObserver = new ResizeObserver(() => {
            scheduleOverviewDraw(canvas, photoPositions);
        });
        overviewResizeObserver.observe(canvas);

        return () => {
            overviewResizeObserver?.disconnect();
            if (overviewRafId !== null) {
                cancelAnimationFrame(overviewRafId);
            }
        };
    });

    $effect(() => {
        const positions = detailPhotoPositions;
        const lc = leftCount;
        const rc = rightCount;
        const ll = leftLineCount;
        const rl = rightLineCount;
        const canvas = detailCanvas;
        if (!canvas) return;

        scheduleDetailDraw(canvas, positions, lc, rc, ll, rl);

        detailResizeObserver?.disconnect();
        detailResizeObserver = new ResizeObserver(() => {
            scheduleDetailDraw(canvas, detailPhotoPositions, leftCount, rightCount, leftLineCount, rightLineCount);
        });
        detailResizeObserver.observe(canvas);

        return () => {
            detailResizeObserver?.disconnect();
            if (detailRafId !== null) {
                cancelAnimationFrame(detailRafId);
            }
        };
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

    <!-- Duration presets -->
    <div class="flex items-center gap-2 mb-2">
        <span class="text-[10px] theme-text-muted">Range:</span>
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
            <canvas bind:this={overviewCanvas} class="absolute inset-0 w-full h-full"></canvas>

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
        <span>{formatDate(timeRange.min)}</span>
        {#if isMapConstrained}
            <span class="text-[var(--accent)]">Map View</span>
        {/if}
        <span>{formatDate(timeRange.max)}</span>
    </div>

    <!-- === Detail Timeline (bottom) === -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="relative h-16 mx-2">
        <div
            bind:this={detailTrack}
            class="absolute inset-0 theme-bg-secondary rounded-lg overflow-hidden border-2 border-[var(--accent)]"
            ontouchstart={handleDetailTouchStart}
            ontouchmove={handleDetailTouchMove}
            ontouchend={handleDetailTouchEnd}
        >
            <!-- Photo lines (canvas) -->
            <canvas bind:this={detailCanvas} class="absolute inset-0 w-full h-full"></canvas>
        </div>
    </div>

    <!-- Detail labels -->
    <div class="flex justify-between text-[10px] theme-text-muted mt-1 mx-2">
        <span>{formatDateTimeShort(viewWindow.start)}</span>
        <span>{formatDateTimeShort(viewWindow.end)}</span>
    </div>
</div>

<style>
    .timeline-container {
        user-select: none;
        touch-action: none;
    }
</style>
