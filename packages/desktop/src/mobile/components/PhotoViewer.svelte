<script lang="ts">
    import { convertFileSrc } from "@tauri-apps/api/core";
    import { untrack } from "svelte";
    import type { Translations } from "../../shared/i18n/types";
    import type { Locale } from "../../shared/i18n/types";

    let {
        photos,
        currentPath,
        t: _t,
        locale,
        onClose,
        onNavigate,
        resolveOriginal,
        resolveStill,
        resolveLiveVideo,
    } = $props<{
        photos: any[];
        currentPath: string;
        t: Translations;
        locale: Locale;
        onClose: () => void;
        onNavigate: (photo: any) => void;
        resolveOriginal?: (photo: any) => Promise<string | null>;
        resolveStill?: (photo: any) => Promise<string | null>;
        resolveLiveVideo?: (photo: any) => Promise<string | null>;
    }>();

    let chromeVisible = $state(true);
    let dragX = $state(0);
    let dragY = $state(0);
    let dragging = $state(false);
    let scale = $state(1);
    let panX = $state(0);
    let panY = $state(0);
    let originals = $state<Record<string, string>>({});
    let stills = $state<Record<string, string>>({});
    let liveVideos = $state<Record<string, string>>({});
    let videoPlaying = $state(false);
    let videoFrameReady = $state(false);
    let livePlaying = $state(false);
    let videoEl: HTMLVideoElement | undefined = $state();
    let liveVideoEl: HTMLVideoElement | undefined = $state();
    let viewerEl: HTMLDivElement | undefined = $state();
    let viewportW = $state(typeof window !== "undefined" ? window.innerWidth : 390);
    let viewportH = $state(typeof window !== "undefined" ? window.innerHeight : 844);

    const DISMISS_Y = 96;
    const MAX_SCALE = 4;
    const PAGE_GAP = 16;
    const SETTLE_MS = 340;
    const LIVE_HOLD_MS = 240;
    const THUMB_W = 36;
    const THUMB_H = 48;
    const THUMB_GAP = 2;
    const THUMB_CELL = THUMB_W + THUMB_GAP;
    const openedAt = Date.now();

    let settling = $state(false);
    let settleTimer: ReturnType<typeof setTimeout> | null = null;
    let liveHoldTimer: ReturnType<typeof setTimeout> | null = null;
    let velX = 0;
    let lastMoveX = 0;
    let lastMoveT = 0;

    function photoKey(photo: any): string {
        return photo?._libraryPath || photo?.path || "";
    }

    let currentIndex = $derived.by(() => {
        const i = photos.findIndex((p) => photoKey(p) === currentPath || p.path === currentPath);
        return i < 0 ? 0 : i;
    });
    let current = $derived(photos[currentIndex] ?? null);
    let prevPhoto = $derived(currentIndex > 0 ? photos[currentIndex - 1] : null);
    let nextPhoto = $derived(currentIndex < photos.length - 1 ? photos[currentIndex + 1] : null);

    function measureViewport() {
        viewportW = viewerEl?.clientWidth || window.innerWidth;
        viewportH = viewerEl?.clientHeight || window.innerHeight;
    }

    function snapHome() {
        clearSettle();
        dragX = 0;
        dragY = 0;
        dragging = false;
        settling = false;
        velX = 0;
    }

    $effect(() => {
        const onResize = () => measureViewport();
        window.scrollTo(0, 0);
        measureViewport();
        window.addEventListener("resize", onResize);
        return () => window.removeEventListener("resize", onResize);
    });

    $effect(() => {
        const el = viewerEl;
        if (!el) return;
        const pin = () => {
            if (el.scrollLeft !== 0 || el.scrollTop !== 0) {
                el.scrollLeft = 0;
                el.scrollTop = 0;
            }
        };
        pin();
        el.addEventListener("scroll", pin, { passive: true });
        return () => el.removeEventListener("scroll", pin);
    });

    $effect(() => {
        currentPath;
        untrack(() => {
            if (!dragging && !settling) snapHome();
        });
    });

    let stripScrubbing = $state(false);
    let stripEl: HTMLDivElement | undefined = $state();

    function rememberImage(key: string, filePath: string, kind: "still" | "original") {
        const url = convertFileSrc(filePath);
        const img = new Image();
        img.onload = () => {
            if (kind === "still") {
                stills = { ...stills, [key]: filePath };
            } else {
                originals = { ...originals, [key]: filePath };
            }
        };
        img.onerror = () => {
            if (kind === "still") {
                stills = { ...stills, [key]: filePath };
            } else {
                originals = { ...originals, [key]: filePath };
            }
        };
        img.src = url;
    }

    $effect(() => {
        const key = currentPath;
        if (!key.startsWith("ios-photo://")) return;
        if (stripScrubbing) return;
        const photo = untrack(
            () => photos.find((p) => photoKey(p) === key || p.path === key) ?? current,
        );
        if (!photo) return;
        let cancelled = false;
        const timer = setTimeout(() => {
            if (cancelled) return;
            if (resolveStill && !untrack(() => stills[key])) {
                resolveStill(photo).then((stillPath) => {
                    if (cancelled || !stillPath) return;
                    rememberImage(key, stillPath, "still");
                });
            }
            if (resolveOriginal && !untrack(() => originals[key])) {
                resolveOriginal(photo).then((filePath) => {
                    if (cancelled || !filePath) return;
                    rememberImage(key, filePath, "original");
                });
            }
        }, 40);
        return () => {
            cancelled = true;
            clearTimeout(timer);
        };
    });

    $effect(() => {
        const photo = current;
        const key = photoKey(photo);
        if (!photo || !key || mediaTypeOf(photo) !== "live_photo") return;
        if (untrack(() => liveVideos[key])) return;
        if (photo.video_path) {
            liveVideos = { ...liveVideos, [key]: photo.video_path };
            return;
        }
        if (!resolveLiveVideo) return;
        let cancelled = false;
        resolveLiveVideo(photo).then((filePath) => {
            if (cancelled || !filePath) return;
            liveVideos = { ...liveVideos, [key]: filePath };
        });
        return () => {
            cancelled = true;
        };
    });

    function parseDate(raw?: string): Date | null {
        if (!raw) return null;
        const normalized = raw.replace(/^(\d{4}):(\d{2}):(\d{2})/, "$1-$2-$3");
        const date = new Date(normalized);
        return Number.isNaN(date.getTime()) ? null : date;
    }

    let chromeDate = $derived.by(() => {
        const raw = current?.metadata?.date_taken || current?.date_taken;
        const date = parseDate(raw);
        if (!date) return { title: "", sub: "" };
        const loc = locale === "zh" ? "zh-CN" : "en-US";
        const sameYear = date.getFullYear() === new Date().getFullYear();
        const title = date.toLocaleDateString(loc, {
            year: sameYear ? undefined : "numeric",
            month: "long",
            day: "numeric",
        });
        const sub = date.toLocaleTimeString(loc, { hour: "numeric", minute: "2-digit" });
        return { title, sub };
    });

    function posterSrc(photo: any): string {
        if (!photo) return "";
        const key = photoKey(photo);
        if (stills[key]) return convertFileSrc(stills[key]);
        if (photo.thumb_path) return convertFileSrc(photo.thumb_path);
        if (photo.path && !photo.path.startsWith("ios-photo://") && mediaTypeOf(photo) !== "video") {
            return convertFileSrc(photo.path);
        }
        return "";
    }

    function displaySrc(photo: any, allowOriginal = false): string {
        if (!photo) return "";
        if (mediaTypeOf(photo) === "video") {
            return posterSrc(photo);
        }
        const key = photoKey(photo);
        if (stills[key]) return convertFileSrc(stills[key]);
        if (allowOriginal && originals[key]) return convertFileSrc(originals[key]);
        if (photo.path && !photo.path.startsWith("ios-photo://")) {
            return convertFileSrc(photo.path);
        }
        return posterSrc(photo);
    }

    function videoFileSrc(photo: any): string {
        if (!photo || mediaTypeOf(photo) !== "video") return "";
        const key = photoKey(photo);
        if (originals[key]) return convertFileSrc(originals[key]);
        if (photo.path && !photo.path.startsWith("ios-photo://")) {
            return convertFileSrc(photo.path);
        }
        return "";
    }

    function liveVideoSrc(photo: any): string {
        if (!photo || mediaTypeOf(photo) !== "live_photo") return "";
        const key = photoKey(photo);
        if (liveVideos[key]) return convertFileSrc(liveVideos[key]);
        if (photo.video_path) return convertFileSrc(photo.video_path);
        return "";
    }

    function mediaTypeOf(photo: any): string {
        return photo?.media_type || photo?.metadata?.media_type || "photo";
    }

    $effect(() => {
        const index = currentIndex;
        videoPlaying = false;
        videoFrameReady = false;
        untrack(() => stopLive());
        queueMicrotask(() => {
            if (currentIndex !== index) return;
            videoEl?.pause();
            if (videoEl) videoEl.currentTime = 0;
        });
    });

    function bindViewerGestures(node: HTMLElement) {
        node.addEventListener("touchstart", onTouchStart, { passive: false });
        node.addEventListener("touchmove", onTouchMove, { passive: false });
        node.addEventListener("touchend", onTouchEnd);
        node.addEventListener("touchcancel", onTouchEnd);
        return {
            destroy() {
                node.removeEventListener("touchstart", onTouchStart);
                node.removeEventListener("touchmove", onTouchMove);
                node.removeEventListener("touchend", onTouchEnd);
                node.removeEventListener("touchcancel", onTouchEnd);
            },
        };
    }

    function bindStripGestures(node: HTMLElement) {
        let x0 = 0;
        let scroll0 = 0;
        let moved = false;
        let hitIndex = 0;
        let lastX = 0;
        let lastT = 0;
        let vel = 0;
        let lastCenter = -1;
        let coast = 0;

        const measure = () => {
            stripViewW = node.clientWidth;
        };
        measure();

        const writeScroll = (value: number) => {
            const max = stripMaxScroll;
            const clamped = Math.max(0, Math.min(max, value));
            stripScroll = value < 0 || value > max
                ? (value < 0 ? value * 0.32 : max + (value - max) * 0.32)
                : value;
            node.scrollLeft = clamped;
        };

        const pickCenter = (scroll = Math.max(0, Math.min(stripMaxScroll, stripScroll))) => {
            const next = indexFromScroll(scroll);
            if (next === lastCenter || !photos[next]) return next;
            lastCenter = next;
            onFilmstrip(photos[next]);
            return next;
        };

        const stopCoast = () => {
            if (coast) {
                cancelAnimationFrame(coast);
                coast = 0;
            }
        };

        const settleCenter = (index: number) => {
            const target = scrollForIndex(index);
            const start = Math.max(0, Math.min(stripMaxScroll, stripScroll));
            const dist = target - start;
            if (Math.abs(dist) < 1) {
                stripScroll = target;
                node.scrollLeft = target;
                stripScrubbing = false;
                return;
            }
            const t0 = performance.now();
            const step = (now: number) => {
                const t = Math.min(1, (now - t0) / 200);
                const ease = 1 - (1 - t) * (1 - t);
                const s = start + dist * ease;
                stripScroll = s;
                node.scrollLeft = s;
                if (t < 1) {
                    coast = requestAnimationFrame(step);
                    return;
                }
                coast = 0;
                stripScroll = target;
                node.scrollLeft = target;
                stripScrubbing = false;
            };
            coast = requestAnimationFrame(step);
        };

        const onStart = (e: TouchEvent) => {
            e.preventDefault();
            e.stopPropagation();
            const t = e.touches[0];
            if (!t) return;
            stopCoast();
            measure();
            stripScrubbing = true;
            x0 = t.clientX;
            lastX = t.clientX;
            lastT = Date.now();
            vel = 0;
            scroll0 = Math.max(0, Math.min(stripMaxScroll, stripScroll));
            moved = false;
            lastCenter = currentIndex;
            const hit = (e.target as HTMLElement | null)?.closest("[data-strip-index]") as HTMLElement | null;
            hitIndex = hit ? Number(hit.dataset.stripIndex) : currentIndex;
            if (!Number.isFinite(hitIndex)) hitIndex = currentIndex;
        };

        const onMove = (e: TouchEvent) => {
            e.preventDefault();
            e.stopPropagation();
            const t = e.touches[0];
            if (!t) return;
            const now = Date.now();
            const dt = now - lastT;
            if (dt > 0) vel = (lastX - t.clientX) / dt;
            lastX = t.clientX;
            lastT = now;
            const dx = t.clientX - x0;
            if (Math.abs(dx) > 4) moved = true;
            writeScroll(scroll0 - dx);
            pickCenter();
        };

        const onEnd = (e: TouchEvent) => {
            e.preventDefault();
            e.stopPropagation();
            if (!moved) {
                if (photos[hitIndex]) onFilmstrip(photos[hitIndex]);
                settleCenter(hitIndex);
                return;
            }
            const max = stripMaxScroll;
            if (Math.abs(vel) > 0.15) {
                let v = vel;
                let s = Math.max(0, Math.min(max, stripScroll));
                const step = () => {
                    s += v * 16;
                    v *= 0.92;
                    if (s < 0 || s > max) {
                        s = Math.max(0, Math.min(max, s));
                        v = 0;
                    }
                    writeScroll(s);
                    pickCenter(s);
                    if (Math.abs(v) < 0.04) {
                        coast = 0;
                        settleCenter(indexFromScroll(s));
                        return;
                    }
                    coast = requestAnimationFrame(step);
                };
                coast = requestAnimationFrame(step);
                return;
            }
            settleCenter(indexFromScroll(stripScroll));
        };

        node.addEventListener("touchstart", onStart, { passive: false });
        node.addEventListener("touchmove", onMove, { passive: false });
        node.addEventListener("touchend", onEnd);
        node.addEventListener("touchcancel", onEnd);
        return {
            destroy() {
                stopCoast();
                stripScrubbing = false;
                node.removeEventListener("touchstart", onStart);
                node.removeEventListener("touchmove", onMove);
                node.removeEventListener("touchend", onEnd);
                node.removeEventListener("touchcancel", onEnd);
            },
        };
    }

    function inlineVideo(node: HTMLVideoElement) {
        node.setAttribute("playsinline", "");
        node.setAttribute("webkit-playsinline", "");
        const reveal = () => {
            videoFrameReady = true;
        };
        const nudge = () => {
            if (node.currentTime === 0) {
                try {
                    node.currentTime = 0.001;
                } catch {
                    /* iOS may reject seek before ready */
                }
            }
            reveal();
        };
        node.addEventListener("loadeddata", nudge);
        node.addEventListener("seeked", reveal);
        return {
            destroy() {
                node.removeEventListener("loadeddata", nudge);
                node.removeEventListener("seeked", reveal);
            },
        };
    }

    function toggleVideo() {
        const el = videoEl;
        const src = videoFileSrc(current);
        if (!el || !src) return;
        if (el.paused) {
            const play = el.play();
            if (play) {
                play.catch((err) => console.warn("[PhotoViewer] video play failed", err));
            }
        } else {
            el.pause();
        }
    }

    function clearLiveHold() {
        if (liveHoldTimer) {
            clearTimeout(liveHoldTimer);
            liveHoldTimer = null;
        }
    }

    function primeLive() {
        const el = liveVideoEl;
        const src = liveVideoSrc(current);
        if (!el || !src) return false;
        el.muted = true;
        el.playsInline = true;
        el.setAttribute("webkit-playsinline", "");
        const play = el.play();
        if (play) {
            play.catch((err) => console.warn("[PhotoViewer] live play failed", err));
        }
        return true;
    }

    function startLiveHold() {
        if (mediaTypeOf(current) !== "live_photo" || scale > 1.05) return;
        clearLiveHold();
        const primed = primeLive();
        liveHoldTimer = setTimeout(() => {
            liveHoldTimer = null;
            if (!gesture || gesture.mode !== "pending") return;
            if (!liveVideoSrc(current)) return;
            gesture.mode = "live";
            livePlaying = true;
            chromeVisible = false;
            if (!primed) primeLive();
        }, LIVE_HOLD_MS);
    }

    function stopLive() {
        clearLiveHold();
        livePlaying = false;
        if (liveVideoEl) {
            liveVideoEl.pause();
            liveVideoEl.muted = true;
            try {
                liveVideoEl.currentTime = 0;
            } catch {
                /* ignore */
            }
        }
    }

    $effect(() => {
        if (!livePlaying) return;
        if (!liveVideoSrc(current)) {
            livePlaying = false;
            return;
        }
        untrack(() => primeLive());
    });

    let gesture: {
        mode: "pending" | "h" | "v" | "pinch" | "pan" | "live";
        x0: number;
        y0: number;
        t0: number;
        lastTap: number;
        pinchDist: number;
        pinchScale: number;
        pan0x: number;
        pan0y: number;
    } | null = null;

    function touchPoint(e: TouchEvent, i = 0) {
        const t = e.touches[i] || e.changedTouches[i];
        return { x: t.clientX, y: t.clientY };
    }

    function dist(a: { x: number; y: number }, b: { x: number; y: number }) {
        return Math.hypot(a.x - b.x, a.y - b.y);
    }

    function resetZoom() {
        scale = 1;
        panX = 0;
        panY = 0;
    }

    function pageWidth() {
        return viewportW + PAGE_GAP;
    }

    function clearSettle() {
        if (settleTimer) {
            clearTimeout(settleTimer);
            settleTimer = null;
        }
        settling = false;
    }

    function settleTo(x: number, after?: () => void) {
        clearSettle();
        dragging = false;
        settling = true;
        dragX = x;
        dragY = 0;
        settleTimer = setTimeout(() => {
            settleTimer = null;
            settling = false;
            dragX = 0;
            dragY = 0;
            after?.();
            requestAnimationFrame(() => snapHome());
        }, SETTLE_MS);
    }

    function go(delta: number) {
        const next = currentIndex + delta;
        if (next < 0 || next >= photos.length) return;
        resetZoom();
        onNavigate(photos[next]);
    }

    function onTouchStart(e: TouchEvent) {
        e.preventDefault();
        if (settling) return;
        if (e.touches.length === 2) {
            const a = touchPoint(e, 0);
            const b = touchPoint(e, 1);
            gesture = {
                mode: "pinch",
                x0: (a.x + b.x) / 2,
                y0: (a.y + b.y) / 2,
                t0: Date.now(),
                lastTap: gesture?.lastTap ?? 0,
                pinchDist: dist(a, b),
                pinchScale: scale,
                pan0x: panX,
                pan0y: panY,
            };
            dragging = true;
            return;
        }
        if (e.touches.length !== 1) return;
        const p = touchPoint(e);
        const now = Date.now();
        const isDouble = now - (gesture?.lastTap ?? 0) < 280;
        gesture = {
            mode: isDouble ? "pan" : "pending",
            x0: p.x,
            y0: p.y,
            t0: now,
            lastTap: gesture?.lastTap ?? 0,
            pinchDist: 0,
            pinchScale: scale,
            pan0x: panX,
            pan0y: panY,
        };
        if (isDouble) {
            if (scale > 1.05) {
                resetZoom();
            } else {
                scale = 2.2;
                panX = (viewportW / 2 - p.x) * 1.2;
                panY = (viewportH / 2 - p.y) * 1.2;
            }
            gesture.lastTap = 0;
            gesture.mode = "pan";
        }
        dragging = true;
        velX = 0;
        lastMoveX = p.x;
        lastMoveT = now;
        if (!isDouble) startLiveHold();
    }

    function onTouchMove(e: TouchEvent) {
        e.preventDefault();
        if (settling || !gesture) return;
        if (e.touches.length === 2) {
            e.preventDefault();
            const a = touchPoint(e, 0);
            const b = touchPoint(e, 1);
            const d = dist(a, b);
            if (gesture.pinchDist > 0) {
                scale = Math.min(MAX_SCALE, Math.max(1, gesture.pinchScale * (d / gesture.pinchDist)));
            }
            gesture.mode = "pinch";
            return;
        }
        const p = touchPoint(e);
        const now = Date.now();
        const moveDt = now - lastMoveT;
        if (moveDt > 0) {
            velX = (p.x - lastMoveX) / moveDt;
        }
        lastMoveX = p.x;
        lastMoveT = now;
        const dx = p.x - gesture.x0;
        const dy = p.y - gesture.y0;
        if (gesture.mode === "live") {
            return;
        }
        if (gesture.mode === "pending") {
            if (Math.hypot(dx, dy) < 8) return;
            clearLiveHold();
            stopLive();
            if (scale > 1.05) {
                gesture.mode = "pan";
            } else if (Math.abs(dx) > Math.abs(dy)) {
                gesture.mode = "h";
            } else if (dy > 0) {
                gesture.mode = "v";
            } else {
                gesture.mode = "h";
            }
        }
        if (gesture.mode === "pan") {
            e.preventDefault();
            panX = gesture.pan0x + dx;
            panY = gesture.pan0y + dy;
            return;
        }
        if (gesture.mode === "h") {
            e.preventDefault();
            const atStart = currentIndex === 0 && dx > 0;
            const atEnd = currentIndex === photos.length - 1 && dx < 0;
            dragX = atStart || atEnd ? dx * 0.28 : dx;
            dragY = 0;
            return;
        }
        if (gesture.mode === "v") {
            e.preventDefault();
            dragY = Math.max(0, dy);
            dragX = dx * 0.2;
        }
    }

    function onTouchEnd(e: TouchEvent) {
        if (settling || !gesture) return;
        if (e.touches.length > 0) return;
        const dt = Date.now() - gesture.t0;
        const mode = gesture.mode;
        const dx = dragX;
        const dy = dragY;
        if (mode === "pending" && dt < 280) {
            if (mediaTypeOf(current) === "video" && videoFileSrc(current)) {
                toggleVideo();
            } else {
                chromeVisible = !chromeVisible;
            }
            gesture.lastTap = Date.now();
        } else {
            gesture.lastTap = 0;
        }
        if (mode === "live") {
            stopLive();
            dragX = 0;
            dragY = 0;
            dragging = false;
            gesture = { ...gesture, mode: "pending" };
            return;
        }
        if (mode === "h") {
            const width = pageWidth();
            const tooSoon = Date.now() - openedAt < 320;
            const far = Math.abs(dx) > width * 0.28;
            const flickPrev = velX > 0.5;
            const flickNext = velX < -0.5;
            const toPrev = dx > 0 && currentIndex > 0 && (far || flickPrev);
            const toNext = dx < 0 && currentIndex < photos.length - 1 && (far || flickNext);
            if (!tooSoon && toPrev) {
                settleTo(width, () => go(-1));
            } else if (!tooSoon && toNext) {
                settleTo(-width, () => go(1));
            } else {
                settleTo(0);
            }
            gesture = { ...gesture, mode: "pending" };
            return;
        }
        if (mode === "v" && dy > DISMISS_Y) {
            onClose();
            return;
        }
        stopLive();
        if (scale < 1.05) resetZoom();
        dragX = 0;
        dragY = 0;
        dragging = false;
        gesture = mode === "pending" ? gesture : { ...gesture, mode: "pending" };
    }

    function onFilmstrip(photo: any) {
        clearSettle();
        stopLive();
        resetZoom();
        dragX = 0;
        dragY = 0;
        dragging = false;
        onNavigate(photo);
    }

    let dismissProgress = $derived(Math.min(1, dragY / 280));
    let pageW = $derived(viewportW + PAGE_GAP);
    let stageStyle = $derived.by(() => {
        const dim = 1 - dismissProgress * 0.45;
        const s = 1 - dismissProgress * 0.12;
        // transform only: the drag stays on the compositor, no layout per touch event.
        return `transform:translate3d(${dragX}px, ${dragY}px, 0) scale(${s});opacity:${dim};`;
    });
    const STRIP_WINDOW = 32;

    let stripScroll = $state(0);
    let stripViewW = $state(typeof window !== "undefined" ? window.innerWidth : 390);
    let stripAnchor = $state(0);

    let stripItems = $derived.by(() => {
        const n = photos.length;
        const start = Math.max(0, stripAnchor - STRIP_WINDOW);
        const end = Math.min(n, stripAnchor + STRIP_WINDOW + 1);
        const items: { photo: any; index: number }[] = [];
        for (let i = start; i < end; i++) {
            items.push({ photo: photos[i], index: i });
        }
        return items;
    });

    let stripMaxScroll = $derived(Math.max(0, (stripItems.length - 1) * THUMB_CELL));

    function windowStart() {
        return Math.max(0, stripAnchor - STRIP_WINDOW);
    }

    function localIndex(index: number) {
        return Math.max(0, index - windowStart());
    }

    function scrollForIndex(index: number) {
        return localIndex(index) * THUMB_CELL;
    }

    function indexFromScroll(scroll: number) {
        const local = Math.round(scroll / THUMB_CELL);
        return Math.max(0, Math.min(photos.length - 1, windowStart() + local));
    }

    function centerStripIndex(index: number) {
        stripScroll = scrollForIndex(index);
        if (stripEl) stripEl.scrollLeft = stripScroll;
    }

    $effect(() => {
        const i = currentIndex;
        const n = photos.length;
        if (!n || stripScrubbing) return;
        untrack(() => {
            stripAnchor = i;
            requestAnimationFrame(() => centerStripIndex(i));
        });
    });

    $effect(() => {
        if (stripEl) stripEl.scrollLeft = stripScroll;
    });

    $effect(() => {
        const el = viewerEl;
        if (!el) return;
        const clear = () => window.getSelection()?.removeAllRanges();
        const block = (event: Event) => {
            event.preventDefault();
            clear();
        };
        clear();
        el.addEventListener("selectstart", block, true);
        el.addEventListener("selectionchange", clear, true);
        document.addEventListener("selectionchange", clear);
        return () => {
            el.removeEventListener("selectstart", block, true);
            el.removeEventListener("selectionchange", clear, true);
            document.removeEventListener("selectionchange", clear);
        };
    });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
    bind:this={viewerEl}
    class="viewer"
    class:dragging
    data-photo-viewer="1"
>
    <div class="backdrop" style="opacity: {1 - dismissProgress * 0.55}"></div>

    <div class="stage" class:settling style={stageStyle}>
        {#if prevPhoto}
            <div class="slide prev" style="left:{-pageW}px">
                <img src={displaySrc(prevPhoto)} alt="" draggable="false" decoding="async" />
            </div>
        {/if}
        {#if current}
            <div
                class="slide current"
                style="left:0;transform:scale({scale}) translate3d({panX / scale}px, {panY / scale}px, 0)"
            >
                {#if mediaTypeOf(current) === "video"}
                    {#if posterSrc(current) && !videoFrameReady}
                        <img src={posterSrc(current)} alt="" class="video-still" class:thumb={!stills[photoKey(current)]} draggable="false" />
                    {/if}
                    {#if videoFileSrc(current)}
                        <video
                            bind:this={videoEl}
                            use:inlineVideo
                            src={videoFileSrc(current)}
                            playsinline
                            preload="auto"
                            class:ready={videoFrameReady}
                            onplay={() => {
                                videoFrameReady = true;
                                videoPlaying = true;
                                chromeVisible = false;
                            }}
                            onpause={() => {
                                videoPlaying = false;
                                chromeVisible = true;
                            }}
                            onended={() => {
                                videoPlaying = false;
                                chromeVisible = true;
                            }}
                        ></video>
                    {/if}
                    {#if !videoPlaying}
                        <div class="play-glyph" class:waiting={!videoFileSrc(current)} aria-hidden="true">
                            <i class="fa-solid {videoFileSrc(current) ? 'fa-play' : 'fa-spinner fa-spin'}"></i>
                        </div>
                    {/if}
                {:else}
                    <img src={displaySrc(current, true)} alt="" draggable="false" decoding="async" />
                    {#if mediaTypeOf(current) === "live_photo" && liveVideoSrc(current)}
                        <video
                            bind:this={liveVideoEl}
                            use:inlineVideo
                            src={liveVideoSrc(current)}
                            playsinline
                            muted
                            loop
                            preload="auto"
                            class="live-video"
                            class:ready={livePlaying}
                        ></video>
                    {/if}
                {/if}
                {#if mediaTypeOf(current) === "live_photo"}
                    <div class="live-badge" class:playing={livePlaying} class:waiting={!liveVideoSrc(current)}>
                        LIVE
                    </div>
                {/if}
            </div>
        {/if}
        {#if nextPhoto}
            <div class="slide next" style="left:{pageW}px">
                <img src={displaySrc(nextPhoto)} alt="" draggable="false" decoding="async" />
            </div>
        {/if}
    </div>

    <div
        class="photo-hit"
        class:above-strip={chromeVisible && dragY <= 24 && photos.length > 1}
        use:bindViewerGestures
    ></div>

    <div class="chrome" class:hidden={!chromeVisible || dragY > 24}>
        <div class="top">
            <div class="title-block">
                <div class="title">{chromeDate.title}</div>
                {#if chromeDate.sub}
                    <div class="sub">{chromeDate.sub}</div>
                {/if}
            </div>
        </div>
        {#if photos.length > 1}
            <div class="strip" bind:this={stripEl} use:bindStripGestures>
                {#each stripItems as item (item.index)}
                    <button
                        type="button"
                        class="thumb"
                        class:active={item.index === currentIndex}
                        data-strip-index={item.index}
                    >
                        {#if item.photo?.thumb_path}
                            <img src={convertFileSrc(item.photo.thumb_path)} alt="" draggable="false" />
                        {/if}
                    </button>
                {/each}
            </div>
        {/if}
    </div>
</div>

<style>
    .viewer {
        position: fixed;
        inset: 0;
        width: 100vw;
        height: 100vh;
        height: 100dvh;
        z-index: 4000;
        touch-action: none;
        overflow: hidden;
        overscroll-behavior: none;
        color: white;
        background: #000;
        isolation: isolate;
        -webkit-user-select: none;
        user-select: none;
        -webkit-touch-callout: none;
        -webkit-tap-highlight-color: transparent;
    }
    .viewer :global(*) {
        -webkit-user-select: none !important;
        user-select: none !important;
        -webkit-touch-callout: none;
    }
    .photo-hit {
        position: absolute;
        inset: 0;
        z-index: 3;
        touch-action: none;
    }
    .photo-hit.above-strip {
        bottom: calc(74px + env(safe-area-inset-bottom, 0px));
    }
    .backdrop {
        position: absolute;
        inset: 0;
        background: #000;
    }
    .stage {
        position: absolute;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
    }
    .stage.settling {
        transition: transform 0.34s cubic-bezier(0.22, 1, 0.36, 1);
    }
    .viewer:not(.dragging):not(.settling) .slide.current {
        transition: transform 0.28s cubic-bezier(0.22, 1, 0.36, 1);
    }
    .slide {
        position: absolute;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
        display: flex;
        align-items: center;
        justify-content: center;
    }
    .slide img,
    .slide video {
        width: 100%;
        height: 100%;
        max-width: 100%;
        max-height: 100%;
        object-fit: contain;
        user-select: none;
        -webkit-user-drag: none;
    }
    .slide video {
        opacity: 0;
        position: absolute;
        inset: 0;
    }
    .slide video.ready {
        opacity: 1;
    }
    .video-still {
        opacity: 1;
    }
    .video-still.thumb {
        filter: none;
    }
    .play-glyph {
        position: absolute;
        width: 64px;
        height: 64px;
        border-radius: 999px;
        background: rgba(255, 255, 255, 0.22);
        display: flex;
        align-items: center;
        justify-content: center;
        pointer-events: none;
    }
    .play-glyph i {
        font-size: 22px;
        margin-left: 3px;
    }
    .live-video {
        z-index: 1;
    }
    .live-badge {
        position: absolute;
        top: calc(env(safe-area-inset-top) + 72px);
        left: 16px;
        font-size: 11px;
        letter-spacing: 0.08em;
        font-weight: 600;
        padding: 3px 8px;
        border-radius: 999px;
        background: rgba(255, 255, 255, 0.16);
        pointer-events: none;
        z-index: 2;
    }
    .live-badge.playing {
        background: rgba(255, 255, 255, 0.28);
        box-shadow: 0 0 0 0 rgba(255, 255, 255, 0.45);
        animation: live-pulse 1.1s ease-out infinite;
    }
    .live-badge.waiting {
        opacity: 0.55;
    }
    @keyframes live-pulse {
        0% { box-shadow: 0 0 0 0 rgba(255, 255, 255, 0.4); }
        100% { box-shadow: 0 0 0 10px rgba(255, 255, 255, 0); }
    }
    .chrome {
        position: absolute;
        inset: 0;
        pointer-events: none;
        transition: opacity 0.2s ease;
    }
    .chrome.hidden {
        opacity: 0;
        visibility: hidden;
        pointer-events: none;
    }
    .top {
        display: flex;
        align-items: center;
        justify-content: center;
        padding: calc(env(safe-area-inset-top) + 8px) 12px 12px;
        background: linear-gradient(to bottom, rgba(0, 0, 0, 0.55), rgba(0, 0, 0, 0));
    }
    .title-block {
        text-align: center;
        min-width: 0;
    }
    .title {
        font-size: 16px;
        font-weight: 600;
        letter-spacing: -0.02em;
        line-height: 1.15;
    }
    .sub {
        margin-top: 2px;
        font-size: 12px;
        color: rgba(255, 255, 255, 0.62);
    }
    .strip {
        pointer-events: auto;
        position: absolute;
        left: 0;
        right: 0;
        bottom: 0;
        width: 100%;
        z-index: 5;
        display: flex;
        flex-wrap: nowrap;
        align-items: flex-end;
        gap: 2px;
        overflow-x: hidden;
        overflow-y: hidden;
        touch-action: none;
        padding: 10px calc((100% - 36px) / 2) calc(env(safe-area-inset-bottom, 0px) + 8px);
        background: #000;
    }
    .thumb {
        flex: 0 0 36px;
        width: 36px;
        height: 48px;
        padding: 0;
        border: 0;
        border-radius: 3px;
        overflow: hidden;
        opacity: 0.55;
        background: #1c1c1e;
        -webkit-tap-highlight-color: transparent;
    }
    .thumb.active {
        opacity: 1;
        box-shadow: 0 0 0 1.5px #fff;
        transform: scale(1.22);
        transform-origin: bottom center;
        z-index: 1;
    }
    .thumb img {
        width: 100%;
        height: 100%;
        object-fit: cover;
        pointer-events: none;
    }
</style>
