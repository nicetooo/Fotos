/**
 * Timeline helpers over a sorted Float64Array of timestamps.
 * Everything is O(log n) or O(pixels · log n): cost does not grow with the library.
 */

/** First index whose value is >= `value`. */
export function lowerBound(sorted: Float64Array, value: number, from = 0): number {
    let lo = from;
    let hi = sorted.length;
    while (lo < hi) {
        const mid = (lo + hi) >>> 1;
        if (sorted[mid] < value) lo = mid + 1;
        else hi = mid;
    }
    return lo;
}

/** First index whose value is > `value`. */
export function upperBound(sorted: Float64Array, value: number, from = 0): number {
    let lo = from;
    let hi = sorted.length;
    while (lo < hi) {
        const mid = (lo + hi) >>> 1;
        if (sorted[mid] <= value) lo = mid + 1;
        else hi = mid;
    }
    return lo;
}

export interface WindowCounts {
    left: number;
    inside: number;
    right: number;
}

/** How many timestamps fall before, inside and after `[startMs, endMs]`. */
export function windowCounts(sorted: Float64Array, startMs: number, endMs: number): WindowCounts {
    const lo = lowerBound(sorted, startMs);
    const hi = upperBound(sorted, endMs, lo);
    return { left: lo, inside: hi - lo, right: sorted.length - hi };
}

/**
 * Pixel columns (0..width-1) that contain at least one timestamp when
 * `[startMs, endMs]` is stretched over `width` pixels.
 */
export function occupiedColumns(
    sorted: Float64Array,
    startMs: number,
    endMs: number,
    width: number,
): number[] {
    const cols: number[] = [];
    const span = endMs - startMs;
    const w = Math.floor(width);
    if (w <= 0 || sorted.length === 0) return cols;
    if (span <= 0) {
        if (upperBound(sorted, endMs) > lowerBound(sorted, startMs)) cols.push(Math.floor(w / 2));
        return cols;
    }
    const msPerPx = span / w;
    let idx = lowerBound(sorted, startMs);
    const end = upperBound(sorted, endMs, idx);
    while (idx < end) {
        const x = Math.min(w - 1, Math.floor((sorted[idx] - startMs) / msPerPx));
        cols.push(x);
        // Jump to the first timestamp that lands in a later column.
        idx = lowerBound(sorted, startMs + (x + 1) * msPerPx, idx + 1);
    }
    return cols;
}

/**
 * Number of visually distinct lines among `sorted[from..to)` when timestamps
 * closer than `minGapMs` collapse into one. Stops counting at `cap`.
 */
export function distinctLines(
    sorted: Float64Array,
    from: number,
    to: number,
    minGapMs: number,
    cap: number,
): number {
    if (to <= from) return 0;
    if (!(minGapMs > 0)) return Math.min(cap, to - from);
    let count = 0;
    let idx = from;
    while (idx < to && count < cap) {
        count++;
        idx = lowerBound(sorted, sorted[idx] + minGapMs, idx + 1);
    }
    return count;
}
