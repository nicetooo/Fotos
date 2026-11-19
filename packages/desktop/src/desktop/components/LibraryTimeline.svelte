<script lang="ts">
    /**
     * The timeline at the top of the library list. Its length is always the
     * whole library's span of time; the bright part is what the list shows.
     * The two edges and the part between them can be dragged, and the list
     * follows while they move.
     */
    import { columnCounts } from "../../shared/utils/timeline-index";
    import type { Locale, Translations } from "../../shared/i18n/types";

    let { stamps, start, end, onChange, t, locale = "en" as Locale } = $props<{
        t: Translations;
        /** Sorted wall-clock ms of every dated item in the library. */
        stamps: Float64Array;
        /** The selected span; null means the whole axis. */
        start: number | null;
        end: number | null;
        onChange: (start: number, end: number) => void;
        locale?: Locale;
    }>();

    let axisMin = $derived(stamps.length > 0 ? stamps[0] : 0);
    let axisMax = $derived(stamps.length > 0 ? stamps[stamps.length - 1] : 0);
    let axisSpan = $derived(Math.max(1, axisMax - axisMin));
    let from = $derived(Math.min(Math.max(start ?? axisMin, axisMin), axisMax));
    let to = $derived(Math.max(Math.min(end ?? axisMax, axisMax), from));
    let leftPercent = $derived(((from - axisMin) / axisSpan) * 100);
    let rightPercent = $derived(((to - axisMin) / axisSpan) * 100);

    let track: HTMLDivElement;
    let canvas: HTMLCanvasElement;
    let size = $state({ w: 0, h: 0 });

    function observe(node: HTMLElement) {
        const observer = new ResizeObserver(() => {
            size = { w: node.clientWidth, h: node.clientHeight };
        });
        observer.observe(node);
        size = { w: node.clientWidth, h: node.clientHeight };
        return { destroy: () => observer.disconnect() };
    }

    // The bars only depend on the data and the size: dragging never redraws them.
    $effect(() => {
        const { w, h } = size;
        const data = stamps;
        if (!canvas || w <= 0 || h <= 0) return;
        const ratio = window.devicePixelRatio || 1;
        canvas.width = Math.round(w * ratio);
        canvas.height = Math.round(h * ratio);
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
        ctx.clearRect(0, 0, w, h);
        if (data.length === 0) return;
        const counts = columnCounts(data, axisMin, axisMax, w);
        let peak = 1;
        for (const c of counts) if (c > peak) peak = c;
        ctx.fillStyle = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() || "#facc15";
        for (let x = 0; x < counts.length; x++) {
            if (counts[x] === 0) continue;
            // A square root keeps sparse days visible next to busy ones.
            const bar = Math.max(2, Math.sqrt(counts[x] / peak) * (h - 2));
            ctx.fillRect(x, h - bar, 1, bar);
        }
    });

    type Drag = { kind: "start" | "end" | "move"; x: number; from: number; to: number };
    let drag: Drag | null = null;

    function msPerPixel(): number {
        return axisSpan / Math.max(1, track?.clientWidth ?? 1);
    }

    function down(kind: Drag["kind"], e: MouseEvent) {
        drag = { kind, x: e.clientX, from, to };
        e.preventDefault();
        e.stopPropagation();
    }

    function move(e: MouseEvent) {
        if (!drag) return;
        const delta = (e.clientX - drag.x) * msPerPixel();
        // An edge never crosses the other: at least a pixel's worth stays selected.
        const least = msPerPixel();
        let a = drag.from;
        let b = drag.to;
        if (drag.kind === "start") {
            a = Math.min(Math.max(axisMin, drag.from + delta), drag.to - least);
        } else if (drag.kind === "end") {
            b = Math.max(Math.min(axisMax, drag.to + delta), drag.from + least);
        } else {
            const width = drag.to - drag.from;
            a = Math.min(Math.max(axisMin, drag.from + delta), axisMax - width);
            b = a + width;
        }
        onChange(a, b);
    }

    function up() {
        drag = null;
    }

    // Two fingers on a trackpad: a pinch makes the selection longer or
    // shorter around its middle, a sideways swipe slides it along the axis.
    // WebKit reports the pinch as gesture events with a scale; other engines
    // as a wheel event with the control key down.
    function resize(width: number) {
        const next = Math.min(axisSpan, Math.max(msPerPixel(), width));
        const middle = (from + to) / 2;
        const a = Math.min(Math.max(axisMin, middle - next / 2), axisMax - next);
        onChange(a, a + next);
    }

    function slide(deltaMs: number) {
        const width = to - from;
        const a = Math.min(Math.max(axisMin, from + deltaMs), axisMax - width);
        onChange(a, a + width);
    }

    let pinchWidth = 0;
    function gestureStart(e: Event) {
        e.preventDefault();
        pinchWidth = to - from;
    }
    function gestureChange(e: Event) {
        e.preventDefault();
        resize(pinchWidth * ((e as Event & { scale?: number }).scale ?? 1));
    }
    function wheel(e: WheelEvent) {
        if (e.ctrlKey) {
            e.preventDefault();
            resize((to - from) * Math.exp(-e.deltaY * 0.01));
            return;
        }
        // Mostly sideways: a vertical scroll over the timeline is left alone.
        if (Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
        e.preventDefault();
        slide(e.deltaX * msPerPixel());
    }
    function gestures(node: HTMLElement) {
        node.addEventListener("gesturestart", gestureStart);
        node.addEventListener("gesturechange", gestureChange);
        node.addEventListener("gestureend", gestureStart);
        node.addEventListener("wheel", wheel, { passive: false });
        return {
            destroy() {
                node.removeEventListener("gesturestart", gestureStart);
                node.removeEventListener("gesturechange", gestureChange);
                node.removeEventListener("gestureend", gestureStart);
                node.removeEventListener("wheel", wheel);
            },
        };
    }

    // Quick spans, the same set as the phone's timeline. A span keeps the
    // middle of the current selection where it is.
    const HOUR = 60 * 60 * 1000;
    const DAY = 24 * HOUR;
    const presets = [
        { label: "1h", value: HOUR },
        { label: "6h", value: 6 * HOUR },
        { label: "1d", value: DAY },
        { label: "7d", value: 7 * DAY },
        { label: "30d", value: 30 * DAY },
        { label: "1y", value: 365 * DAY },
    ];
    let wholeAxis = $derived(from <= axisMin && to >= axisMax);

    function selectPreset(duration: number) {
        const width = Math.min(duration, axisSpan);
        const middle = (from + to) / 2;
        const a = Math.min(Math.max(axisMin, middle - width / 2), axisMax - width);
        onChange(a, a + width);
    }

    function isSelected(duration: number): boolean {
        return !wholeAxis && Math.abs(to - from - duration) < 1000;
    }

    function label(ms: number): string {
        return new Date(ms).toLocaleDateString(locale, { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" });
    }
</script>

<svelte:window onmousemove={move} onmouseup={up} />

{#if stamps.length > 1}
    <div class="px-4 pt-2 pb-1 border-b theme-border select-none" data-library-timeline use:gestures>
        <div class="flex items-center gap-1 mb-1.5">
            <span class="text-[10px] theme-text-muted mr-1">{t.timeline.range}:</span>
            {#each presets as preset}
                <button
                    onclick={() => selectPreset(preset.value)}
                    data-span-preset={preset.label}
                    class="px-2 py-0.5 text-[10px] rounded transition-colors {isSelected(preset.value)
                        ? 'bg-[var(--accent)] text-black'
                        : 'theme-bg-primary theme-text-muted hover:theme-text-secondary'}"
                >
                    {preset.label}
                </button>
            {/each}
            <button
                onclick={() => onChange(axisMin, axisMax)}
                data-span-preset="all"
                class="px-2 py-0.5 text-[10px] rounded transition-colors {wholeAxis
                    ? 'bg-[var(--accent)] text-black'
                    : 'theme-bg-primary theme-text-muted hover:theme-text-secondary'}"
            >
                {t.timeline.all}
            </button>
        </div>
        <div class="relative h-9" bind:this={track}>
            <div class="absolute inset-0 rounded theme-bg-primary overflow-hidden" use:observe>
                <canvas bind:this={canvas} class="absolute inset-0 w-full h-full"></canvas>
                <!-- Outside the selection: dimmed -->
                <div class="absolute top-0 bottom-0 left-0 bg-black/60" style="width: {leftPercent}%"></div>
                <div class="absolute top-0 bottom-0 right-0 bg-black/60" style="width: {100 - rightPercent}%"></div>
            </div>
            <!-- The selection: drag it to slide, drag an edge to change one end -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
                class="absolute top-0 bottom-0 border-y-2 border-[var(--accent)] cursor-grab"
                style="left: {leftPercent}%; width: {rightPercent - leftPercent}%; min-width: 2px"
                onmousedown={(e) => down("move", e)}
            ></div>
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
                class="handle"
                data-handle="start"
                style="left: {leftPercent}%"
                onmousedown={(e) => down("start", e)}
            ></div>
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
                class="handle"
                data-handle="end"
                style="left: {rightPercent}%"
                onmousedown={(e) => down("end", e)}
            ></div>
        </div>
        <div class="flex justify-between text-[10px] mt-1 tabular-nums">
            <span class="theme-text-muted">{label(axisMin)}</span>
            <span class="text-[var(--accent)]" data-library-timeline-range>{label(from)} – {label(to)}</span>
            <span class="theme-text-muted">{label(axisMax)}</span>
        </div>
    </div>
{/if}

<style>
    .handle {
        position: absolute;
        top: -2px;
        bottom: -2px;
        width: 10px;
        margin-left: -5px;
        cursor: ew-resize;
    }
    .handle::after {
        content: "";
        position: absolute;
        top: 0;
        bottom: 0;
        left: 3px;
        width: 4px;
        border-radius: 2px;
        background: var(--accent);
    }
</style>
