/**
 * Agent self-test probe.
 *
 * Runs only when the app is launched with FOOTOS_AGENT_TEST set (see
 * scripts/agent-test). It drives the real UI — camera moves, time filter,
 * viewer — measures frame times while doing so, and writes a JSON report next
 * to the database. The runner reads that file and screenshots the simulator.
 *
 * Nothing here is imported in a normal run.
 */

import { invoke } from "@tauri-apps/api/core";
import { geoSummary, queryMap, queryTimeline, queryAlbum, photoCount } from "./query";

interface FrameStats {
    frames: number;
    durationMs: number;
    fps: number;
    p50: number;
    p95: number;
    maxMs: number;
    /** Frames longer than 50ms (visible hitch). */
    long: number;
}

interface Step {
    name: string;
    ok: boolean;
    detail?: unknown;
    frames?: FrameStats;
}

export interface AgentTestHooks {
    /** The MapLibre map instance. */
    getMap: () => any | null;
    /** Counters the map component exposes. */
    getMapStats: () => Record<string, number>;
    openPreview: () => Promise<unknown>;
    closePreview: () => void;
    bootMarks: () => Record<string, number>;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function waitFor(check: () => boolean, timeoutMs: number, stepMs = 100): Promise<boolean> {
    const start = performance.now();
    while (performance.now() - start < timeoutMs) {
        if (check()) return true;
        await sleep(stepMs);
    }
    return check();
}

/** Record frame intervals while `action` runs. */
async function measureFrames(action: () => Promise<void>): Promise<FrameStats> {
    const gaps: number[] = [];
    let last = performance.now();
    let running = true;
    const tick = (now: number) => {
        gaps.push(now - last);
        last = now;
        if (running) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    const t0 = performance.now();
    await action();
    running = false;
    const durationMs = performance.now() - t0;
    gaps.shift(); // first gap is the wait for the first frame
    const sorted = [...gaps].sort((a, b) => a - b);
    const pick = (q: number) => (sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))] : 0);
    const round = (v: number) => Math.round(v * 10) / 10;
    return {
        frames: gaps.length,
        durationMs: Math.round(durationMs),
        fps: durationMs > 0 ? round((gaps.length * 1000) / durationMs) : 0,
        p50: round(pick(0.5)),
        p95: round(pick(0.95)),
        maxMs: round(sorted.length ? sorted[sorted.length - 1] : 0),
        long: gaps.filter((g) => g > 50).length,
    };
}

function moveCamera(map: any, options: Record<string, unknown>, durationMs: number): Promise<void> {
    return new Promise((resolve) => {
        let done = false;
        const finish = () => {
            if (done) return;
            done = true;
            map.off("moveend", finish);
            resolve();
        };
        map.on("moveend", finish);
        map.easeTo({ ...options, duration: durationMs, essential: true });
        setTimeout(finish, durationMs + 1500);
    });
}

export async function runAgentTest(scenario: string, hooks: AgentTestHooks): Promise<void> {
    const errors: string[] = [];
    const steps: Step[] = [];
    const started = Date.now();

    const onError = (e: ErrorEvent) => errors.push(`error: ${e.message} @ ${e.filename}:${e.lineno}`);
    const onRejection = (e: PromiseRejectionEvent) => errors.push(`unhandledrejection: ${String(e.reason)}`);
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    const originalConsoleError = console.error;
    console.error = (...args: unknown[]) => {
        errors.push("console.error: " + args.map((a) => (a instanceof Error ? a.message : String(a))).join(" "));
        originalConsoleError(...args);
    };

    const step = async (name: string, run: () => Promise<Omit<Step, "name">>) => {
        try {
            steps.push({ name, ...(await run()) });
        } catch (e) {
            steps.push({ name, ok: false, detail: String(e) });
        }
    };

    try {
        // 1. Data reached the UI.
        await step("library-loaded", async () => {
            const ready = await waitFor(() => (hooks.getMapStats().total ?? 0) > 0, 30000);
            const summary = await geoSummary();
            const count = await photoCount();
            return {
                ok: ready && summary.total > 0,
                detail: { geotagged: summary.total, rows: count, extent: summary.extent, mapStats: hooks.getMapStats() },
            };
        });

        const map = hooks.getMap();
        if (!map) throw new Error("map instance not available");

        // Let the initial fitBounds and thumbnail loads settle.
        await sleep(2500);

        // 2. Raw query cost for the whole world and for a city.
        await step("query-cost", async () => {
            const time = async (run: () => Promise<unknown>) => {
                const t0 = performance.now();
                const result = await run();
                return { ms: Math.round((performance.now() - t0) * 10) / 10, result };
            };
            const world = await time(() => queryMap({ west: -180, south: -85, east: 180, north: 85 }, 2, null, null));
            const stamps = await time(() => queryTimeline(null));
            const album = await time(() => queryAlbum(null, 300));
            const w = world.result as { features: unknown[]; matched: number };
            return {
                ok: w.features.length > 0 && w.features.length < 5000,
                detail: {
                    worldQueryMs: world.ms,
                    worldFeatures: w.features.length,
                    worldMatched: w.matched,
                    timelineMs: stamps.ms,
                    timelineStamps: (stamps.result as Float64Array).length,
                    albumMs: album.ms,
                },
            };
        });

        // 3. Rendered features exist on the map.
        await step("map-rendered", async () => {
            const stats = hooks.getMapStats();
            return { ok: stats.features > 0, detail: stats };
        });

        // 4. Camera animations: this is where jank shows up.
        // Aim at the densest cluster so zooming in keeps photos on screen.
        const world = await queryMap({ west: -180, south: -85, east: 180, north: 85 }, 3, null, null);
        const densest = [...world.features].sort((a, b) => b.count - a.count)[0];
        const cx = densest?.lon ?? 0;
        const cy = densest?.lat ?? 20;
        const moves: [string, Record<string, unknown>, number][] = [
            ["pan-world", { center: [cx - 40, cy], zoom: 2 }, 1500],
            ["zoom-to-city", { center: [cx, cy], zoom: 8 }, 2000],
            ["pan-in-city", { center: [cx + 0.15, cy + 0.08] }, 1500],
            ["zoom-street", { center: [cx, cy], zoom: 14 }, 2000],
            ["pan-street", { center: [cx + 0.01, cy + 0.005] }, 1500],
            ["zoom-out-world", { center: [cx, cy], zoom: 2 }, 2200],
        ];
        for (const [name, options, duration] of moves) {
            await step(`camera:${name}`, async () => {
                const before = hooks.getMapStats();
                const frames = await measureFrames(() => moveCamera(map, options, duration));
                await sleep(400);
                const after = hooks.getMapStats();
                return {
                    ok: frames.frames > 0,
                    frames,
                    detail: {
                        zoom: Math.round(map.getZoom() * 100) / 100,
                        features: after.features,
                        queries: after.queries - before.queries,
                        setData: after.setData - before.setData,
                        lastQueryMs: after.lastQueryMs,
                    },
                };
            });
        }

        // 5. Time scrub: slide a 30-day window across the whole range, one
        // filter change per frame, the way a finger on the timeline would.
        await step("time-scrub", async () => {
            const setFilter = (window as any).__footosSetTimeFilter as
                | ((start: number | null, end: number | null) => void)
                | undefined;
            if (!setFilter) return { ok: false, detail: "time filter hook missing" };
            const stamps = await queryTimeline(null);
            if (stamps.length < 2) return { ok: false, detail: "no timestamps" };
            const min = stamps[0];
            const max = stamps[stamps.length - 1];
            const windowMs = 30 * 24 * 3600 * 1000;
            const before = hooks.getMapStats();
            const durationMs = 3000;
            const frames = await measureFrames(
                () =>
                    new Promise<void>((resolve) => {
                        const t0 = performance.now();
                        const tick = (now: number) => {
                            const k = Math.min(1, (now - t0) / durationMs);
                            const start = min + (max - min - windowMs) * k;
                            setFilter(start, start + windowMs);
                            if (k < 1) requestAnimationFrame(tick);
                            else resolve();
                        };
                        requestAnimationFrame(tick);
                    }),
            );
            await sleep(500);
            const after = hooks.getMapStats();
            const filtered = after.features;
            setFilter(null, null);
            await sleep(500);
            return {
                ok: frames.frames > 0 && after.queries > before.queries,
                frames,
                detail: {
                    queries: after.queries - before.queries,
                    setData: after.setData - before.setData,
                    queriesPerSecond: Math.round(((after.queries - before.queries) / durationMs) * 1000),
                    lastQueryMs: after.lastQueryMs,
                    featuresWhileFiltered: filtered,
                    atlasImages: after.atlasImages,
                },
            };
        });

        // 6. Viewer opens on an album window.
        await step("viewer-open", async () => {
            const t0 = performance.now();
            const result = await hooks.openPreview();
            const shown = await waitFor(() => !!document.querySelector("[data-photo-viewer]"), 5000);
            const ms = Math.round(performance.now() - t0);
            const thumbs = document.querySelectorAll("[data-strip-index]").length;
            await sleep(800);
            return { ok: shown && thumbs > 0, detail: { openMs: ms, stripThumbs: thumbs, result } };
        });
        if (scenario !== "keep-viewer") {
            hooks.closePreview();
            await sleep(400);
        }
    } catch (e) {
        errors.push("probe aborted: " + String(e));
    } finally {
        window.removeEventListener("error", onError);
        window.removeEventListener("unhandledrejection", onRejection);
        console.error = originalConsoleError;
    }

    const report = {
        scenario,
        finishedAt: new Date().toISOString(),
        elapsedMs: Date.now() - started,
        passed: errors.length === 0 && steps.length > 0 && steps.every((x) => x.ok),
        userAgent: navigator.userAgent,
        viewport: { w: window.innerWidth, h: window.innerHeight, dpr: window.devicePixelRatio },
        boot: hooks.bootMarks(),
        steps,
        errors,
    };
    try {
        await invoke("agent_test_report", { json: JSON.stringify(report, null, 2) });
    } catch (e) {
        originalConsoleError("[agent-test] failed to write report", e);
    }
}
