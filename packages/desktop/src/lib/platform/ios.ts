/**
 * iOS Platform Service
 * iOS 平台服务实现（WebKit 桥接）
 */

import { invoke } from "@tauri-apps/api/core";
import type {
    PlatformCapabilities,
    ImportProgress,
    ImportResult,
    PermissionStatus,
    MapTileConfig,
    IOSPlatformService,
    NativeViewerItem,
} from "./types";

type EventHandler = { event: string; handler: EventListener };

class IOSPlatformServiceImpl implements IOSPlatformService {
    readonly platform = 'ios' as const;
    readonly isMobile = true;

    readonly capabilities: PlatformCapabilities = {
        arbitraryPathAccess: false,
        nativePhotoLibrary: true,
        runtimePermissions: true,
        revealInFileManager: false,
        folderSelection: false,
        nativeMap: true,
        nativeContextMenu: true,
        nativePhotoViewer: true,
    };

    private progressCallbacks = new Set<(p: ImportProgress) => void>();
    private completeCallbacks = new Set<(r: ImportResult) => void>();
    private scanningCallbacks = new Set<(s: boolean) => void>();
    private permissionCallbacks = new Set<(hasFullAccess: boolean) => void>();
    private eventListeners: EventHandler[] = [];
    private dbPath = '';
    private thumbDir = '';
    private _hasFullAccess = false;
    private bridgeReady = false;
    private importRunning = false;
    private pendingBatches: Array<Record<string, unknown>[]> = [];
    private exportFinished = true;
    private importTotals = { success: 0, failure: 0, duplicates: 0 };
    private originalWaiters = new Map<string, (path: string) => void>();
    private stillWaiters = new Map<string, (path: string) => void>();
    private stillEarlyWaiters = new Map<string, (path: string) => void>();
    private liveWaiters = new Map<string, (path: string) => void>();
    private cachedStills = new Map<string, string>();
    private cachedLiveVideos = new Map<string, string>();
    private inflightExports = new Map<string, Promise<{ filePath: string; stillPath: string }>>();
    private knownIdentifiers: string[] = [];
    private libraryChangedCallbacks = new Set<() => void>();
    private deniedAlert: Record<string, string> | null = null;

    get hasFullPhotoAccess(): boolean {
        return this._hasFullAccess;
    }

    async initialize(dbPath: string, thumbDir: string): Promise<void> {
        this.dbPath = dbPath;
        this.thumbDir = thumbDir;

        // iOS 导入进度事件
        this.addListener('ios-import-progress', (e: CustomEvent) => {
            const detail = e.detail;
            const progress: ImportProgress = {
                success: detail.success || 0,
                failure: detail.failure || 0,
                duplicates: detail.duplicates || 0,
                current: detail.current,
                total: detail.total,
                lastPath: detail.phase || "",
            };
            this.progressCallbacks.forEach(cb => cb(progress));
        });

        // iOS 导出进度事件
        this.addListener('ios-export-progress', (e: CustomEvent) => {
            const detail = e.detail;
            const progress: ImportProgress = {
                success: this.importTotals.success,
                failure: this.importTotals.failure,
                duplicates: this.importTotals.duplicates,
                current: detail.current,
                total: detail.total,
                lastPath: detail.phase || "Exporting...",
            };
            this.progressCallbacks.forEach(cb => cb(progress));
        });

        this.addListener('ios-export-complete', () => {
            console.log("[iOS] Native export finished");
            this.exportFinished = true;
            this.finishImportIfIdle();
        });

        // iOS 导入完成事件
        this.addListener('ios-import-complete', (e: CustomEvent) => {
            const detail = e.detail;
            console.log("[iOS] Import complete:", detail);
            this.scanningCallbacks.forEach(cb => cb(false));
            this.completeCallbacks.forEach(cb => cb({
                success: detail.success || 0,
                failure: detail.failure || 0,
                duplicates: detail.duplicates || 0,
            }));
        });

        // iOS 导入开始事件
        this.addListener('ios-import-started', () => {
            console.log("[iOS] Import started");
            this.exportFinished = false;
            // Do not clear pendingBatches: reused thumbs can be handed off
            // before this event is processed, and wiping would drop them.
            if (!this.importRunning && this.pendingBatches.length === 0) {
                this.importTotals = { success: 0, failure: 0, duplicates: 0 };
            }
            this.scanningCallbacks.forEach(cb => cb(true));
        });

        // iOS 权限授予事件
        this.addListener('ios-permission-granted', (e: CustomEvent) => {
            const detail = e.detail;
            console.log("[iOS] Permission granted:", detail.type);
            this._hasFullAccess = detail.type === "full";
            this.permissionCallbacks.forEach(cb => cb(this._hasFullAccess));
        });

        // iOS 权限拒绝事件
        this.addListener('ios-permission-denied', (e: CustomEvent) => {
            const detail = e.detail;
            console.log("[iOS] Permission denied:", detail.message);
            this._hasFullAccess = false;
            this.permissionCallbacks.forEach(cb => cb(false));
            this.scanningCallbacks.forEach(cb => cb(false));
        });

        // iOS 权限状态检查结果
        this.addListener('ios-permission-status', (e: CustomEvent) => {
            const status = e.detail.status;
            console.log("[iOS] Permission status:", status);
            this._hasFullAccess = status === 3; // 3 = authorized (full access)
            this.permissionCallbacks.forEach(cb => cb(this._hasFullAccess));
        });

        // iOS 同步跳过事件
        this.addListener('ios-sync-skipped', (e: CustomEvent) => {
            const reason = e.detail.reason;
            const status = e.detail.status;
            console.log("[iOS] Sync skipped:", reason || status);
            if (reason === "up_to_date") {
                this.exportFinished = true;
                this.scanningCallbacks.forEach(cb => cb(false));
                return;
            }
            this._hasFullAccess = status === 3;
            this.permissionCallbacks.forEach(cb => cb(this._hasFullAccess));
        });

        // Assets the app can no longer see (deleted, or deselected under
        // limited access): drop their rows, then let the UI refresh.
        this.addListener('ios-assets-removed', (e: CustomEvent) => {
            const ids: string[] = e.detail?.ids ?? [];
            if (ids.length === 0) return;
            const gone = new Set(ids);
            this.knownIdentifiers = this.knownIdentifiers.filter((id) => !gone.has(id));
            void invoke<number>("remove_ios_assets", { ids })
                .then((removed) => {
                    console.log("[iOS] Removed rows of missing assets:", removed);
                    if (removed > 0) this.libraryChangedCallbacks.forEach(cb => cb());
                })
                .catch((err) => console.error("[iOS] remove_ios_assets failed", err));
        });

        // iOS Swift 桥接就绪事件
        this.addListener('footos-bridge-ready', () => {
            console.log("[iOS] Swift bridge is ready");
            this.bridgeReady = true;
        });

        // 检查桥接是否已就绪
        if ((window as any).__FOOTOS_BRIDGE_READY__) {
            console.log("[iOS] Swift bridge was already ready");
            this.bridgeReady = true;
        }

        (window as any).__handleIOSPreviewBatch = (items: Record<string, unknown>[], db: string, thumb: string) => {
            console.log("[iOS] Preview batch:", items?.length);
            if (db) this.dbPath = db;
            if (thumb) this.thumbDir = thumb;
            if (!items || items.length === 0) {
                this.finishImportIfIdle();
                return;
            }
            this.exportFinished = false;
            this.pendingBatches.push(items);
            this.scanningCallbacks.forEach(cb => cb(true));
            void this.drainImportQueue();
        };

        (window as any).__handleIOSOriginal = (identifier: string, path: string) => {
            const waiter = this.originalWaiters.get(identifier);
            if (waiter) {
                this.originalWaiters.delete(identifier);
                waiter(path);
            }
        };

        (window as any).__handleIOSViewerStill = (identifier: string, path: string) => {
            if (path) {
                this.cachedStills.set(identifier, path);
            }
            const waiter = this.stillWaiters.get(identifier);
            if (waiter) {
                this.stillWaiters.delete(identifier);
                waiter(path);
            }
            const early = this.stillEarlyWaiters.get(identifier);
            if (early) {
                this.stillEarlyWaiters.delete(identifier);
                early(path);
            }
        };

        (window as any).__handleIOSLiveVideo = (identifier: string, path: string) => {
            if (path) {
                this.cachedLiveVideos.set(identifier, path);
            }
            const waiter = this.liveWaiters.get(identifier);
            if (waiter) {
                this.liveWaiters.delete(identifier);
                waiter(path);
            }
        };

        const ready = await this.waitForBridge(5000);
        if (!ready) {
            console.warn("[iOS] PhotoKit bridge not ready; permission will be requested on import");
        }
    }

    private gridThumbsReady: ((thumbs: Record<string, string>) => void) | null = null;
    private gridThumbsListening = false;

    requestGridThumbs(paths: string[], onReady: (thumbs: Record<string, string>) => void): void {
        this.gridThumbsReady = onReady;
        if (!this.gridThumbsListening) {
            this.gridThumbsListening = true;
            this.addListener("ios-grid-thumbs", (e) => {
                const out: Record<string, string> = {};
                for (const [id, file] of Object.entries((e.detail?.thumbs ?? {}) as Record<string, string>)) {
                    out["ios-photo://" + id] = file;
                }
                this.gridThumbsReady?.(out);
            });
        }
        const identifiers = paths
            .filter((p) => p.startsWith("ios-photo://"))
            .map((p) => p.slice("ios-photo://".length));
        this.postNative({ command: "gridThumbs", identifiers });
    }

    setKnownPhotoIdentifiers(ids: string[]): void {
        this.knownIdentifiers = ids;
    }

    private addListener(event: string, handler: (e: CustomEvent) => void): void {
        const wrappedHandler = handler as EventListener;
        window.addEventListener(event, wrappedHandler);
        this.eventListeners.push({ event, handler: wrappedHandler });
    }

    cleanup(): void {
        this.eventListeners.forEach(({ event, handler }) => {
            window.removeEventListener(event, handler);
        });
        this.eventListeners = [];
        delete (window as any).__handleIOSPreviewBatch;
        delete (window as any).__handleIOSOriginal;
        delete (window as any).__handleIOSViewerStill;
        delete (window as any).__handleIOSLiveVideo;
        this.progressCallbacks.clear();
        this.completeCallbacks.clear();
        this.scanningCallbacks.clear();
    }

    async requestImport(): Promise<void> {
        const ready = await this.waitForBridge(5000);
        if (!ready) {
            console.error("[iOS] PhotoKit bridge not available");
            return;
        }
        this.exportFinished = false;
        this.importTotals = { success: 0, failure: 0, duplicates: 0 };
        this.postNative({ command: "requestImport", ...(this.deniedAlert ? { deniedAlert: this.deniedAlert } : {}) });
    }

    setAccessDeniedTexts(texts: { title: string; message: string; settings: string; cancel: string }): void {
        this.deniedAlert = texts;
    }

    onLibraryChanged(callback: () => void): () => void {
        this.libraryChangedCallbacks.add(callback);
        return () => this.libraryChangedCallbacks.delete(callback);
    }

    resetAfterClear(): void {
        this.knownIdentifiers = [];
        this.cachedStills.clear();
        this.cachedLiveVideos.clear();
        this.inflightExports.clear();
        this.pendingBatches = [];
        this.postNative({ command: "clearCaches" });
    }

    async syncPhotosIfFullAccess(): Promise<void> {
        const ready = await this.waitForBridge(5000);
        if (!ready) {
            console.log("[iOS] PhotoKit bridge not ready for auto-sync");
            return;
        }
        this.postNative({ command: "requestAccessAndSync" });
    }

    showLimitedLibraryPicker(): void {
        // Limited library picker is iOS-specific and requires native Swift code
        // With tauri-plugin-dialog, users can select any photos they want
        console.log("[iOS] Limited library picker not available, use requestImport instead");
    }

    onPermissionChange(callback: (hasFullAccess: boolean) => void): () => void {
        this.permissionCallbacks.add(callback);
        return () => this.permissionCallbacks.delete(callback);
    }

    private async drainImportQueue(): Promise<void> {
        if (this.importRunning) {
            return;
        }
        this.importRunning = true;
        this.scanningCallbacks.forEach(cb => cb(true));

        while (this.pendingBatches.length > 0) {
            const batch = this.pendingBatches.shift()!;
            console.log("[iOS] Upserting preview batch of", batch.length);
            try {
                const result: any = await invoke("upsert_ios_previews", { items: batch });
                this.importTotals.success += result.success || 0;
                this.importTotals.duplicates += result.duplicates || 0;
                this.importTotals.failure += result.failure || 0;
                console.log("[iOS] Upsert result", result);
            } catch (err) {
                console.error("[iOS] Batch import failed:", err);
                this.importTotals.failure += batch.length;
            }

            const progress: ImportProgress = {
                success: this.importTotals.success,
                failure: this.importTotals.failure,
                duplicates: this.importTotals.duplicates,
                current: this.importTotals.success + this.importTotals.failure + this.importTotals.duplicates,
                total: this.importTotals.success + this.importTotals.failure + this.importTotals.duplicates
                    + this.pendingBatches.reduce((n, b) => n + b.length, 0),
                lastPath: "Preview",
            };
            this.progressCallbacks.forEach(cb => cb(progress));
        }

        this.importRunning = false;
        this.finishImportIfIdle();
    }

    private finishImportIfIdle(): void {
        if (this.importRunning || this.pendingBatches.length > 0 || !this.exportFinished) {
            return;
        }
        const result: ImportResult = { ...this.importTotals };
        console.log("[iOS] Import complete:", result);
        void invoke("flush_photos_db").catch((e) => console.warn("[iOS] flush_photos_db", e));
        this.completeCallbacks.forEach(cb => cb(result));
        this.scanningCallbacks.forEach(cb => cb(false));
    }

    nativeMapCommand(name: string, payload: Record<string, unknown> = {}): void {
        const handler = (window as any).webkit?.messageHandlers?.footosPhotos;
        handler?.postMessage({ command: name, ...payload });
    }

    setAlbumMenuContext(items: NativeViewerItem[], titles: { copy: string; share: string; locate: string }): void {
        const handler = (window as any).webkit?.messageHandlers?.footosPhotos;
        handler?.postMessage({ command: "albumMenu", items, titles });
    }

    openNativeViewer(items: NativeViewerItem[], index: number): boolean {
        const handler = (window as any).webkit?.messageHandlers?.footosPhotos;
        if (!handler || items.length === 0) return false;
        handler.postMessage({ command: "openViewer", items, index });
        return true;
    }

    async ensureOriginal(path: string): Promise<string> {
        const media = await this.exportViewerMedia(path);
        if (!media.filePath) {
            throw new Error("Export original failed");
        }
        return media.filePath;
    }

    async ensureViewerStill(path: string): Promise<string | null> {
        if (!path.startsWith("ios-photo://")) {
            return null;
        }
        const identifier = path.slice("ios-photo://".length);
        const cached = this.cachedStills.get(identifier);
        if (cached) {
            return cached;
        }
        const ready = await this.waitForBridge(2000);
        if (!ready) {
            return null;
        }
        return new Promise((resolve) => {
            const timer = setTimeout(() => {
                this.stillEarlyWaiters.delete(identifier);
                resolve(this.cachedStills.get(identifier) || null);
            }, 8000);
            this.stillEarlyWaiters.set(identifier, (still) => {
                clearTimeout(timer);
                this.stillEarlyWaiters.delete(identifier);
                resolve(still || null);
            });
            this.postNative({ command: "exportViewerStill", identifier });
        });
    }

    async ensureLiveVideo(path: string): Promise<string | null> {
        if (!path.startsWith("ios-photo://")) {
            return null;
        }
        const identifier = path.slice("ios-photo://".length);
        const cached = this.cachedLiveVideos.get(identifier);
        if (cached) {
            return cached;
        }
        const ready = await this.waitForBridge(2000);
        if (!ready) {
            return null;
        }
        return new Promise((resolve) => {
            const timer = setTimeout(() => {
                this.liveWaiters.delete(identifier);
                resolve(this.cachedLiveVideos.get(identifier) || null);
            }, 20000);
            this.liveWaiters.set(identifier, (file) => {
                clearTimeout(timer);
                this.liveWaiters.delete(identifier);
                resolve(file || null);
            });
            this.postNative({ command: "exportLiveVideo", identifier });
        });
    }

    private async exportViewerMedia(path: string): Promise<{ filePath: string; stillPath: string }> {
        if (!path.startsWith("ios-photo://")) {
            return { filePath: path, stillPath: "" };
        }
        const identifier = path.slice("ios-photo://".length);
        const existing = this.inflightExports.get(identifier);
        if (existing) {
            return existing;
        }
        const ready = await this.waitForBridge(3000);
        if (!ready) {
            throw new Error("PhotoKit bridge not ready");
        }
        const pending = new Promise<{ filePath: string; stillPath: string }>((resolve, reject) => {
            let filePath = "";
            let stillPath = "";
            let settled = false;
            const finish = () => {
                if (settled || !filePath) return;
                settled = true;
                this.originalWaiters.delete(identifier);
                this.stillWaiters.delete(identifier);
                this.inflightExports.delete(identifier);
                resolve({ filePath, stillPath });
            };
            const timer = setTimeout(() => {
                this.originalWaiters.delete(identifier);
                this.stillWaiters.delete(identifier);
                this.inflightExports.delete(identifier);
                if (!settled) {
                    settled = true;
                    reject(new Error("Export original timed out"));
                }
            }, 90000);
            this.stillWaiters.set(identifier, (still) => {
                stillPath = still;
                if (still) {
                    this.cachedStills.set(identifier, still);
                }
                const early = this.stillEarlyWaiters.get(identifier);
                if (early) {
                    this.stillEarlyWaiters.delete(identifier);
                    early(still);
                }
            });
            this.originalWaiters.set(identifier, (file) => {
                clearTimeout(timer);
                if (file) {
                    filePath = file;
                    finish();
                } else {
                    this.inflightExports.delete(identifier);
                    reject(new Error("Export original failed"));
                }
            });
            this.postNative({ command: "exportOriginal", identifier });
        });
        this.inflightExports.set(identifier, pending);
        return pending;
    }

    async checkPermissionStatus(): Promise<PermissionStatus> {
        const ready = await this.waitForBridge(2000);
        if (ready) {
            this.postNative({ command: "checkStatus" });
        }
        if (this._hasFullAccess) return 'granted';
        return 'unknown';
    }

    private postNative(data: Record<string, unknown>): void {
        const handler = (window as any).webkit?.messageHandlers?.footosPhotos;
        if (!handler) {
            console.warn("[iOS] footosPhotos message handler not available");
            return;
        }
        const payload: Record<string, unknown> = {
            dbPath: this.dbPath,
            thumbDir: this.thumbDir,
            ...data,
        };
        // 10k+ UUIDs on every bridge call freeze WKWebView. Only send them
        // when PhotoKit actually needs the incremental skip list.
        if (data.command === "requestAccessAndSync" || data.command === "requestImport") {
            payload.knownIds = this.knownIdentifiers;
        }
        handler.postMessage(payload);
    }

    private async waitForBridge(maxWait: number): Promise<boolean> {
        if (this.bridgeReady || (window as any).__FOOTOS_BRIDGE_READY__ || (window as any).webkit?.messageHandlers?.footosPhotos) {
            this.bridgeReady = true;
            return true;
        }
        const start = Date.now();
        return new Promise((resolve) => {
            const check = () => {
                if (this.bridgeReady || (window as any).__FOOTOS_BRIDGE_READY__ || (window as any).webkit?.messageHandlers?.footosPhotos) {
                    this.bridgeReady = true;
                    resolve(true);
                    return;
                }
                if (Date.now() - start > maxWait) {
                    resolve(false);
                    return;
                }
                setTimeout(check, 200);
            };
            check();
        });
    }

    onImportProgress(callback: (progress: ImportProgress) => void): () => void {
        this.progressCallbacks.add(callback);
        return () => this.progressCallbacks.delete(callback);
    }

    onImportComplete(callback: (result: ImportResult) => void): () => void {
        this.completeCallbacks.add(callback);
        return () => this.completeCallbacks.delete(callback);
    }

    onScanningChange(callback: (isScanning: boolean) => void): () => void {
        this.scanningCallbacks.add(callback);
        return () => this.scanningCallbacks.delete(callback);
    }

    async openInSystemPlayer(path: string): Promise<void> {
        if ((window as any).webkit?.messageHandlers?.openVideo) {
            (window as any).webkit.messageHandlers.openVideo.postMessage({ path });
        }
    }

    getMapTileConfig(): MapTileConfig | null {
        return null;
    }
}

export function createIOSPlatformService(): IOSPlatformService {
    return new IOSPlatformServiceImpl();
}
